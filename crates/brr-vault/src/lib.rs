use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use brr_core::{Error, KeySource};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

pub mod archive;
pub mod csv;
pub mod totp;

#[cfg(test)]
mod tests;

pub const KEYRING_SERVICE: &str = "brr";

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const KEY_TEMP_SUFFIX: &str = ".tmp";
const KEY_FILE_MODE: u32 = 0o600;
const KEY_FILE_DEPENDENCY: &str = "vault key file";
const KEYRING_NAME: &str = "keyring";
const KEY_FILE_HELP: &str = "restore it from a backup or start with a fresh data dir";
const KEYRING_HELP: &str = "unlock the keyring";

pub struct KeyLocation {
    pub file: PathBuf,
    pub keyring_user: String,
}

pub struct Vault {
    cipher: XChaCha20Poly1305,
}

impl Vault {
    pub fn create(source: KeySource, at: &KeyLocation) -> Result<Vault, Error> {
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        rand::fill(&mut *key);

        match source {
            KeySource::File => create_key_file(&at.file, &*key)?,
            KeySource::Keyring => create_keyring_key(at, &*key)?,
        }
        Ok(Vault::from_key(&key))
    }

    pub fn open(source: KeySource, at: &KeyLocation) -> Result<Vault, Error> {
        let (key, origin) =
            match source {
                KeySource::File => (
                    read_key_file(&at.file)?,
                    format!("vault key file {}", at.file.display()),
                ),
                KeySource::Keyring => (
                    Zeroizing::new(keyring_entry(at)?.get_secret().map_err(
                        |error| match error {
                            keyring::Error::NoEntry => missing_keyring_key(at),
                            _ => keyring_unavailable(),
                        },
                    )?),
                    format!("vault key {} in the keyring", at.keyring_user),
                ),
            };

        let key: Zeroizing<[u8; KEY_LEN]> =
            Zeroizing::new(key.as_slice().try_into().map_err(|_| Error::Internal {
                message: format!(
                    "{origin} has {} bytes, expected {KEY_LEN}; {KEY_FILE_HELP}",
                    key.len()
                ),
            })?);
        Ok(Vault::from_key(&key))
    }

    pub fn encrypt(&self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::fill(&mut nonce);

        let ciphertext = self
            .cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| Error::Internal {
                message: "secret is too large to encrypt".into(),
            })?;

        let mut sealed = nonce.to_vec();
        sealed.extend_from_slice(&ciphertext);
        Ok(sealed)
    }

    pub fn decrypt(&self, aad: &[u8], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
        let (nonce, ciphertext) = sealed
            .split_first_chunk::<NONCE_LEN>()
            .ok_or_else(cannot_decrypt)?;
        let payload = Payload {
            msg: ciphertext,
            aad,
        };
        self.cipher
            .decrypt(&XNonce::from(*nonce), payload)
            .map(Zeroizing::new)
            .map_err(|_| cannot_decrypt())
    }

    fn from_key(key: &[u8; KEY_LEN]) -> Vault {
        Vault {
            cipher: XChaCha20Poly1305::new(key.into()),
        }
    }
}

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Vault")
    }
}

// Written to a temp file and linked into place, so a failed write never leaves a short key file
// behind, and an existing key is never overwritten.
fn create_key_file(file: &Path, key: &[u8]) -> Result<(), Error> {
    let mut temp = file.as_os_str().to_owned();
    temp.push(KEY_TEMP_SUFFIX);
    let temp = PathBuf::from(temp);
    let write_error = |e: std::io::Error| Error::Internal {
        message: format!("cannot write vault key file {}: {e}", file.display()),
    };

    match fs::remove_file(&temp) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(write_error(e)),
    }

    let mut handle = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(KEY_FILE_MODE)
        .open(&temp)
        .map_err(|e| Error::Internal {
            message: format!("cannot create vault key file {}: {e}", file.display()),
        })?;
    let written = handle.write_all(key).and_then(|()| handle.sync_all());
    let linked = written.and_then(|()| fs::hard_link(&temp, file));
    // The temp name is dead either way; the key lives on under its real name after a link.
    let _ = fs::remove_file(&temp);

    linked.map_err(|e| match e.kind() {
        ErrorKind::AlreadyExists => Error::Internal {
            message: format!(
                "vault key file {} already exists; refusing to overwrite",
                file.display()
            ),
        },
        _ => write_error(e),
    })
}

fn read_key_file(file: &Path) -> Result<Zeroizing<Vec<u8>>, Error> {
    fs::read(file)
        .map(Zeroizing::new)
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => Error::DependencyMissing {
                name: KEY_FILE_DEPENDENCY.into(),
                help: KEY_FILE_HELP.into(),
            },
            _ => Error::Internal {
                message: format!("cannot read vault key file {}: {e}", file.display()),
            },
        })
}

fn create_keyring_key(at: &KeyLocation, key: &[u8]) -> Result<(), Error> {
    let entry = keyring_entry(at)?;
    match entry.get_secret() {
        Ok(_) => {
            return Err(Error::Internal {
                message: format!(
                    "vault key for {} already exists in the keyring; refusing to overwrite",
                    at.keyring_user
                ),
            });
        }
        Err(keyring::Error::NoEntry) => {}
        Err(_) => return Err(keyring_unavailable()),
    }
    entry.set_secret(key).map_err(|_| keyring_unavailable())
}

fn keyring_entry(at: &KeyLocation) -> Result<keyring::Entry, Error> {
    keyring::Entry::new(KEYRING_SERVICE, &at.keyring_user).map_err(|_| keyring_unavailable())
}

fn keyring_unavailable() -> Error {
    Error::DependencyMissing {
        name: KEYRING_NAME.into(),
        help: KEYRING_HELP.into(),
    }
}

fn missing_keyring_key(at: &KeyLocation) -> Error {
    Error::DependencyMissing {
        name: format!("{KEYRING_NAME} entry {}", at.keyring_user),
        help: KEY_FILE_HELP.into(),
    }
}

fn cannot_decrypt() -> Error {
    Error::Internal {
        message: "secret cannot be decrypted with this vault key".into(),
    }
}
