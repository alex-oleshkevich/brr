use std::{
    fmt,
    fs::{self, File},
    io::{self, Cursor, Read, Write},
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    sync::Arc,
};

use brr_core::{Error, sha256_hex};
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::Number;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use super::network::Network;
use crate::data::FpgenPin;

const STAMP_FILE: &str = ".pinned-model";
const NETWORK_ZST: &str = "fingerprint-network.json.zst";
const VALUES_JSON_ZST: &str = "values.json.zst";
const VALUES_DAT_ZST: &str = "values.dat.zst";
const VALUES_DAT: &str = "values.dat";
const PIN_FIELD: &str = "fpgen_model";
const HEX_RADIX: u32 = 16;
const BASE85_ALPHABET: &[u8; 85] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz!#$%&()*+-;<=>?@^_`{|}~";
const BASE85_GROUP: usize = 5;
const BASE85_FILL: u8 = b'~';

// A JSON document whose objects keep their key order, which is what Python's dict gives
// fpgen: the order is the value id in the index and the tie-break in the network.
#[derive(Debug, Clone, PartialEq)]
pub enum Ordered {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Ordered>),
    Object(Vec<(String, Ordered)>),
}

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Ordered, D::Error> {
        deserializer.deserialize_any(OrderedVisitor)
    }
}

struct OrderedVisitor;

impl<'de> Visitor<'de> for OrderedVisitor {
    type Value = Ordered;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_unit<E>(self) -> Result<Ordered, E> {
        Ok(Ordered::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Ordered, E> {
        Ok(Ordered::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Ordered, E> {
        Ok(Ordered::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Ordered, E> {
        Ok(Ordered::Number(value.into()))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Ordered, E> {
        Number::from_f64(value)
            .map(Ordered::Number)
            .ok_or_else(|| E::custom("non-finite number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Ordered, E> {
        Ok(Ordered::String(value.to_owned()))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Ordered, A::Error> {
        let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Ordered::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Ordered, A::Error> {
        let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some(entry) = map.next_entry()? {
            entries.push(entry);
        }
        Ok(Ordered::Object(entries))
    }
}

pub fn parse_ordered(text: &str) -> Result<Ordered, serde_json::Error> {
    serde_json::from_str(text)
}

// values.json: {"<hex offset>": length, ...}; the position in the document is the value id
struct ValueIndex(Vec<(u64, u64)>);

impl<'de> Deserialize<'de> for ValueIndex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ValueIndex, D::Error> {
        deserializer.deserialize_map(ValueIndexVisitor)
    }
}

struct ValueIndexVisitor;

impl<'de> Visitor<'de> for ValueIndexVisitor {
    type Value = ValueIndex;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an object of hex offset to length")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ValueIndex, A::Error> {
        let mut slices = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some((offset, length)) = map.next_entry::<String, u64>()? {
            let offset = u64::from_str_radix(&offset, HEX_RADIX).map_err(de::Error::custom)?;
            slices.push((offset, length));
        }
        Ok(ValueIndex(slices))
    }
}

pub struct Model {
    network: Network,
    slices: Vec<(u64, u64)>,
    values: File,
}

impl Model {
    // Read-only: the directory must already hold values.dat (unpack_model or `zstd -d`)
    pub fn open(dir: &Path) -> Result<Arc<Model>, Error> {
        let network = Network::build(&read_zst_json(&dir.join(NETWORK_ZST))?)?;
        let ValueIndex(slices) = read_zst_json(&dir.join(VALUES_JSON_ZST))?;

        let values_path = dir.join(VALUES_DAT);
        let expected_len = slices
            .iter()
            .map(|(offset, length)| offset + length)
            .max()
            .unwrap_or(0);
        let values = File::open(&values_path).map_err(|err| internal("opening values.dat", err))?;
        let len = values
            .metadata()
            .map_err(|err| internal("reading values.dat", err))?
            .len();
        if len != expected_len {
            return Err(invalid(format!(
                "values.dat is {len} bytes but values.json describes {expected_len}; unpack the model again"
            )));
        }

        Ok(Arc::new(Model {
            network,
            slices,
            values,
        }))
    }

    pub fn network(&self) -> &Network {
        &self.network
    }

    // The raw JSON text behind a base85 value id, as in fpgen's lookup_value
    pub fn lookup_text(&self, id: &str) -> Result<String, Error> {
        let index = usize::try_from(base85_to_int(id)?)
            .map_err(|_| invalid(format!("value id {id} is out of range")))?;
        let &(offset, length) = self
            .slices
            .get(index)
            .ok_or_else(|| invalid(format!("value id {id} is out of range")))?;

        let mut buf = vec![0u8; length as usize];
        self.values
            .read_exact_at(&mut buf, offset)
            .map_err(|err| internal("reading values.dat", err))?;
        String::from_utf8(buf).map_err(|err| internal("decoding a model value", err))
    }
}

pub fn unpack_model(zip_path: &Path, pin: &FpgenPin, dir: &Path) -> Result<(), Error> {
    let stamp_path = dir.join(STAMP_FILE);
    if fs::read_to_string(&stamp_path).is_ok_and(|stamp| stamp.trim() == pin.sha256) {
        return Ok(());
    }

    let zip_bytes =
        fs::read(zip_path).map_err(|err| internal("reading the fpgen model archive", err))?;
    if sha256_hex(&zip_bytes) != pin.sha256 {
        return Err(invalid("sha256 mismatch".to_owned()));
    }

    let mut archive = ZipArchive::new(Cursor::new(zip_bytes))
        .map_err(|err| invalid(format!("not a zip archive: {err}")))?;
    let mut members = Vec::with_capacity(pin.files.len());
    for name in &pin.files {
        // Names become file names under dir
        if Path::new(name)
            .file_name()
            .is_none_or(|base| base != name.as_str())
        {
            return Err(invalid(format!("unexpected member path {name}")));
        }

        let mut bytes = Vec::new();
        archive
            .by_name(name)
            .map_err(|err| invalid(format!("archive is missing {name}: {err}")))?
            .read_to_end(&mut bytes)
            .map_err(|err| internal("reading an fpgen model member", err))?;
        if pin.file_sha256.get(name).map(String::as_str) != Some(sha256_hex(&bytes).as_str()) {
            return Err(invalid(format!("{name} sha256 mismatch")));
        }
        members.push((name, bytes));
    }

    fs::create_dir_all(dir).map_err(|err| internal("creating the fpgen model directory", err))?;
    // An interrupted unpack must not leave a stamp over a partial model
    let _ = fs::remove_file(&stamp_path);

    let dat_path = dir.join(VALUES_DAT);
    let dat_temp = temp_path(&dat_path);
    let dat_zst = members
        .iter()
        .find(|(name, _)| *name == VALUES_DAT_ZST)
        .map(|(_, bytes)| bytes.as_slice())
        .ok_or_else(|| invalid(format!("the pin does not list {VALUES_DAT_ZST}")))?;
    let mut dat = HashingFile {
        file: File::create(&dat_temp).map_err(|err| internal("creating values.dat", err))?,
        hasher: Sha256::new(),
    };
    if let Err(err) = zstd::stream::copy_decode(dat_zst, &mut dat) {
        let _ = fs::remove_file(&dat_temp);
        return Err(internal("decompressing values.dat", err));
    }
    if pin.decompressed_sha256.get(VALUES_DAT).map(String::as_str)
        != Some(hex(&dat.hasher.finalize()).as_str())
    {
        let _ = fs::remove_file(&dat_temp);
        return Err(invalid(format!("{VALUES_DAT} sha256 mismatch")));
    }
    fs::rename(&dat_temp, &dat_path).map_err(|err| internal("placing values.dat", err))?;

    for (name, bytes) in members.iter().filter(|(name, _)| *name != VALUES_DAT_ZST) {
        let path = dir.join(name);
        let temp = temp_path(&path);
        fs::write(&temp, bytes).map_err(|err| internal("writing an fpgen model file", err))?;
        fs::rename(&temp, &path).map_err(|err| internal("placing an fpgen model file", err))?;
    }

    let stamp_temp = temp_path(&stamp_path);
    fs::write(&stamp_temp, format!("{}\n", pin.sha256))
        .map_err(|err| internal("writing the model stamp", err))?;
    fs::rename(&stamp_temp, &stamp_path).map_err(|err| internal("placing the model stamp", err))
}

struct HashingFile {
    file: File,
    hasher: Sha256,
}

impl Write for HashingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.file.write(buf)?;
        self.hasher.update(&buf[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn read_zst_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, Error> {
    let file = File::open(path).map_err(|err| {
        invalid(format!(
            "cannot open {}: {err}; unpack the fpgen model first",
            path.display()
        ))
    })?;
    let decoder = zstd::stream::read::Decoder::new(file)
        .map_err(|err| internal("reading an fpgen model file", err))?;
    serde_json::from_reader(io::BufReader::new(decoder))
        .map_err(|err| invalid(format!("{} is not valid model JSON: {err}", path.display())))
}

// fpgen's base85_to_int: base64.b85decode, then the bytes as a big-endian integer
fn base85_to_int(id: &str) -> Result<u64, Error> {
    let mut bytes = id.as_bytes().to_vec();
    let padding = (BASE85_GROUP - bytes.len() % BASE85_GROUP) % BASE85_GROUP;
    bytes.resize(bytes.len() + padding, BASE85_FILL);

    let mut decoded = Vec::with_capacity(bytes.len() / BASE85_GROUP * 4);
    for group in bytes.chunks_exact(BASE85_GROUP) {
        let mut acc = 0u64;
        for &byte in group {
            let digit = BASE85_ALPHABET
                .iter()
                .position(|&candidate| candidate == byte)
                .ok_or_else(|| invalid(format!("value id {id} is not base85")))?;
            acc = acc * BASE85_ALPHABET.len() as u64 + digit as u64;
        }
        let word = u32::try_from(acc).map_err(|_| invalid(format!("value id {id} overflows")))?;
        decoded.extend_from_slice(&word.to_be_bytes());
    }
    decoded.truncate(decoded.len() - padding);

    decoded
        .iter()
        .try_fold(0u64, |acc, &byte| {
            acc.checked_mul(256)
                .map(|shifted| shifted + u64::from(byte))
        })
        .ok_or_else(|| invalid(format!("value id {id} is too large")))
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn invalid(message: String) -> Error {
    Error::InvalidArgument {
        field: PIN_FIELD.to_owned(),
        message,
    }
}

fn internal(doing: &str, err: impl fmt::Display) -> Error {
    Error::Internal {
        message: format!("{doing}: {err}"),
    }
}
