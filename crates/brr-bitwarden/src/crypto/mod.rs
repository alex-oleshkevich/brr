use aes::Aes256;
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine, engine::general_purpose::STANDARD};
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, KeyIvInit};
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use pbkdf2::pbkdf2_hmac;
use rsa::PrivateKey;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const KEY_LEN: usize = 64;
const HALF_KEY_LEN: usize = 32;
const MASTER_KEY_LEN: usize = 32;
const KIB_PER_MIB: u32 = 1024;
const ENC_INFO: &[u8] = b"enc";
const MAC_INFO: &[u8] = b"mac";
const TYPE_SEPARATOR: char = '.';
const PART_SEPARATOR: char = '|';
const AES_CBC_HMAC: &str = "2";
const RSA_OAEP_SHA256: &str = "3";
const RSA_OAEP_SHA1: &str = "4";

#[derive(Debug)]
pub(crate) enum Error {
    Mac,
    Unsupported(String),
    Malformed(&'static str),
}

pub(crate) enum Kdf {
    Pbkdf2 {
        iterations: u32,
    },
    Argon2id {
        iterations: u32,
        memory_mib: u32,
        parallelism: u32,
    },
}

// Encryption key followed by MAC key, the layout Bitwarden uses for user, organization and item keys.
#[derive(Clone)]
pub(crate) struct Key(Zeroizing<[u8; KEY_LEN]>);

impl Key {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Key, Error> {
        if bytes.len() != KEY_LEN {
            return Err(Error::Malformed("a key is not 64 bytes long"));
        }

        let mut key = Zeroizing::new([0; KEY_LEN]);
        key.copy_from_slice(bytes);
        Ok(Key(key))
    }

    fn enc(&self) -> &[u8] {
        &self.0[..HALF_KEY_LEN]
    }

    fn mac(&self) -> &[u8] {
        &self.0[HALF_KEY_LEN..]
    }
}

pub(crate) fn master_key(
    password: &str,
    email: &str,
    kdf: &Kdf,
) -> Result<Zeroizing<[u8; MASTER_KEY_LEN]>, Error> {
    let salt = email.trim().to_lowercase();
    let mut key = Zeroizing::new([0; MASTER_KEY_LEN]);

    match *kdf {
        Kdf::Pbkdf2 { iterations } => {
            pbkdf2_hmac::<Sha256>(password.as_bytes(), salt.as_bytes(), iterations, &mut *key);
        }
        Kdf::Argon2id {
            iterations,
            memory_mib,
            parallelism,
        } => {
            let params = Params::new(
                memory_mib.saturating_mul(KIB_PER_MIB),
                iterations,
                parallelism,
                Some(MASTER_KEY_LEN),
            )
            .map_err(|e| Error::Unsupported(format!("Argon2id parameters: {e}")))?;
            Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
                .hash_password_into(
                    password.as_bytes(),
                    &Sha256::digest(salt.as_bytes()),
                    &mut *key,
                )
                .map_err(|e| Error::Unsupported(format!("Argon2id parameters: {e}")))?;
        }
    }

    Ok(key)
}

pub(crate) fn stretch(master_key: &[u8; MASTER_KEY_LEN]) -> Result<Key, Error> {
    let hkdf = Hkdf::<Sha256>::from_prk(master_key)
        .map_err(|_| Error::Malformed("the master key is too short to stretch"))?;
    let mut key = Zeroizing::new([0; KEY_LEN]);
    let (enc, mac) = key.split_at_mut(HALF_KEY_LEN);
    hkdf.expand(ENC_INFO, enc)
        .and_then(|()| hkdf.expand(MAC_INFO, mac))
        .map_err(|_| Error::Malformed("the master key cannot be stretched"))?;
    Ok(Key(key))
}

pub(crate) fn decrypt(enc_string: &str, key: &Key) -> Result<Zeroizing<Vec<u8>>, Error> {
    let (kind, body) = enc_string
        .split_once(TYPE_SEPARATOR)
        .ok_or(Error::Malformed("an encrypted value has no type"))?;
    if kind != AES_CBC_HMAC {
        return Err(Error::Unsupported(format!("encryption type {kind}")));
    }

    let mut parts = body.split(PART_SEPARATOR);
    let (Some(iv), Some(data), Some(mac), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(Error::Malformed("an encrypted value is not iv|data|mac"));
    };
    let iv = decode(iv)?;
    let data = decode(data)?;
    let mac = decode(mac)?;

    let mut hmac = <Hmac<Sha256> as KeyInit>::new_from_slice(key.mac())
        .map_err(|_| Error::Malformed("a MAC key has the wrong length"))?;
    hmac.update(&iv);
    hmac.update(&data);
    hmac.verify_slice(&mac).map_err(|_| Error::Mac)?;

    let mut plain = Zeroizing::new(data);
    let len = cbc::Decryptor::<Aes256>::new_from_slices(key.enc(), &iv)
        .map_err(|_| Error::Malformed("an encrypted value has a bad IV"))?
        .decrypt_padded::<Pkcs7>(&mut plain)
        .map_err(|_| Error::Malformed("an encrypted value has bad padding"))?
        .len();
    plain.truncate(len);
    Ok(plain)
}

pub(crate) fn decrypt_rsa(
    enc_string: &str,
    private_key: &PrivateKey,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    let (kind, body) = enc_string
        .split_once(TYPE_SEPARATOR)
        .ok_or(Error::Malformed("an encrypted key has no type"))?;
    let data = decode(body)?;

    let plain = match kind {
        RSA_OAEP_SHA1 => private_key.oaep_decrypt::<Sha1>(&data),
        RSA_OAEP_SHA256 => private_key.oaep_decrypt::<Sha256>(&data),
        _ => return Err(Error::Unsupported(format!("key encryption type {kind}"))),
    };

    plain
        .map(Zeroizing::new)
        .map_err(|_| Error::Malformed("an organization key cannot be decrypted"))
}

fn decode(part: &str) -> Result<Vec<u8>, Error> {
    STANDARD
        .decode(part)
        .map_err(|_| Error::Malformed("an encrypted value is not base64"))
}

#[cfg(test)]
pub(crate) mod tests;
