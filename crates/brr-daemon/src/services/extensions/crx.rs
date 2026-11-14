use brr_core::Error;
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 4] = b"Cr24";
const CRX2_PREFIX_BYTES: usize = 16;
const CRX3_PREFIX_BYTES: usize = 12;
const CRX_ID_BYTES: usize = 16;
const PROOF_RSA_FIELD: u64 = 2;
const PROOF_ECDSA_FIELD: u64 = 3;
const SIGNED_DATA_FIELD: u64 = 10_000;
const PUBLIC_KEY_FIELD: u64 = 1;
const CRX_ID_FIELD: u64 = 1;
const WIRE_VARINT: u64 = 0;
const WIRE_FIXED64: u64 = 1;
const WIRE_BYTES: u64 = 2;
const WIRE_FIXED32: u64 = 5;
const UPDATECHECK_TAG: &str = "<updatecheck";

pub struct Crx<'a> {
    pub zip: &'a [u8],
    pub public_key: Option<Vec<u8>>,
}

fn bad_crx(message: &str) -> Error {
    Error::InvalidArgument {
        field: "file".to_string(),
        message: message.to_string(),
    }
}

fn read_u32(bytes: &[u8], at: usize) -> Option<usize> {
    let raw: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;

    usize::try_from(u32::from_le_bytes(raw)).ok()
}

pub fn parse(bytes: &[u8]) -> Result<Crx<'_>, Error> {
    if bytes.get(..MAGIC.len()) != Some(MAGIC) {
        return Err(bad_crx("not a CRX file"));
    }
    let truncated = || bad_crx("the CRX file is truncated");

    match read_u32(bytes, 4) {
        Some(2) => {
            let key_len = read_u32(bytes, 8).ok_or_else(truncated)?;
            let signature_len = read_u32(bytes, 12).ok_or_else(truncated)?;
            let key_end = CRX2_PREFIX_BYTES + key_len;
            let public_key = bytes
                .get(CRX2_PREFIX_BYTES..key_end)
                .ok_or_else(truncated)?
                .to_vec();
            let zip = bytes.get(key_end + signature_len..).ok_or_else(truncated)?;

            Ok(Crx {
                zip,
                public_key: Some(public_key),
            })
        }
        Some(3) => {
            let header_len = read_u32(bytes, 8).ok_or_else(truncated)?;
            let header_end = CRX3_PREFIX_BYTES + header_len;
            let header = bytes
                .get(CRX3_PREFIX_BYTES..header_end)
                .ok_or_else(truncated)?;

            Ok(Crx {
                zip: &bytes[header_end..],
                public_key: public_key(header),
            })
        }
        _ => Err(bad_crx("unsupported CRX version; expected 2 or 3")),
    }
}

fn varint(buf: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;

    for shift in (0..64).step_by(7) {
        let byte = *buf.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }

    None
}

// Only length-delimited fields matter here; the others are skipped by their wire size.
fn fields(buf: &[u8]) -> Vec<(u64, &[u8])> {
    let mut found = Vec::new();
    let mut at = 0;

    while at < buf.len() {
        let Some(tag) = varint(buf, &mut at) else {
            break;
        };
        let (number, wire) = (tag >> 3, tag & 7);
        let skip = match wire {
            WIRE_VARINT => varint(buf, &mut at).map(|_| 0),
            WIRE_FIXED64 => Some(8),
            WIRE_FIXED32 => Some(4),
            WIRE_BYTES => {
                let Some(len) = varint(buf, &mut at).and_then(|l| usize::try_from(l).ok()) else {
                    break;
                };
                let Some(value) = at.checked_add(len).and_then(|end| buf.get(at..end)) else {
                    break;
                };
                at += len;
                found.push((number, value));

                Some(0)
            }
            _ => None,
        };
        match skip {
            Some(bytes) => at += bytes,
            None => break,
        }
    }

    found
}

// The developer key is the proof whose hash is the CRX id; a Web Store file carries Google's proof too.
fn public_key(header: &[u8]) -> Option<Vec<u8>> {
    let mut keys = Vec::new();
    let mut crx_id = None;

    for (number, value) in fields(header) {
        match number {
            PROOF_RSA_FIELD | PROOF_ECDSA_FIELD => {
                if let Some((_, key)) = fields(value)
                    .into_iter()
                    .find(|(n, _)| *n == PUBLIC_KEY_FIELD)
                {
                    keys.push(key.to_vec());
                }
            }
            SIGNED_DATA_FIELD => {
                crx_id = fields(value)
                    .into_iter()
                    .find(|(n, _)| *n == CRX_ID_FIELD)
                    .map(|(_, id)| id.to_vec());
            }
            _ => {}
        }
    }

    let matching = crx_id.and_then(|id| {
        keys.iter()
            .find(|key| Sha256::digest(key)[..CRX_ID_BYTES] == id[..])
            .cloned()
    });

    matching.or_else(|| keys.into_iter().next())
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let value = &tag[start..];

    Some(value[..value.find('"')?].replace("&amp;", "&").to_string())
}

// (version, codebase) of the first <updatecheck>; None when the answer offers no update.
pub fn scan_updatecheck(xml: &str) -> Option<(String, String)> {
    let from = &xml[xml.find(UPDATECHECK_TAG)?..];
    let tag = &from[..from.find('>')?];

    Some((attribute(tag, "version")?, attribute(tag, "codebase")?))
}
