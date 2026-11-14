use brr_core::{Error, KeySource, Secret, SecretStatus, now_ms};
use brr_store::{Store, secret};
use brr_vault::{KeyLocation, Vault};

use crate::internal;

#[cfg(test)]
mod tests;

const CHECK_NAME: &str = "vault.check";
const CHECK_MARKER: &[u8; 16] = b"brr-vault-check\0";
const HINT_PREFIX: &str = "…";
const HINT_CHARS: usize = 4;
// A shorter value would be mostly spelled out by its own hint.
const MIN_HINT_SOURCE_CHARS: usize = 2 * HINT_CHARS;
pub const KEY_SOURCE_SETTING: &str = "vault.key_source";
pub const DEFAULT_KEY_SOURCE: KeySource = KeySource::File;

pub struct Secrets {
    vault: Vault,
}

impl Secrets {
    pub async fn bootstrap(
        store: &Store,
        at: &KeyLocation,
        configured: Option<KeySource>,
    ) -> Result<Secrets, Error> {
        let check = store.run(|c| secret::get(c, CHECK_NAME)).await?;

        let Some(sealed) = check else {
            // The file source is the default: the keyring can block on a GUI unlock prompt.
            let source = configured.unwrap_or(DEFAULT_KEY_SOURCE);
            let vault = Vault::create(source, at).map_err(|e| with_remedy(e, source, at, true))?;
            let sealed = vault.encrypt(CHECK_NAME.as_bytes(), CHECK_MARKER)?;
            store
                .run(move |c| secret::put(c, CHECK_NAME, &sealed, now_ms()))
                .await?;

            return Ok(Secrets { vault });
        };

        let source = configured.ok_or_else(|| Error::InvalidArgument {
            field: KEY_SOURCE_SETTING.to_string(),
            message:
                "config.toml lost vault.key_source; set it back to the source used at creation"
                    .to_string(),
        })?;
        let vault = Vault::open(source, at).map_err(|e| with_remedy(e, source, at, false))?;
        vault.decrypt(CHECK_NAME.as_bytes(), &sealed)?;

        Ok(Secrets { vault })
    }

    pub async fn set(&self, store: &Store, name: &str, value: &Secret) -> Result<(), Error> {
        reject_reserved(name)?;

        let sealed = self
            .vault
            .encrypt(name.as_bytes(), value.expose().as_bytes())?;
        let name = name.to_string();
        store
            .run(move |c| secret::put(c, &name, &sealed, now_ms()))
            .await
    }

    pub async fn get(&self, store: &Store, name: &str) -> Result<Option<Secret>, Error> {
        reject_reserved(name)?;

        let row = name.to_string();
        let Some(sealed) = store.run(move |c| secret::get(c, &row)).await? else {
            return Ok(None);
        };
        let plain = self.vault.decrypt(name.as_bytes(), &sealed)?;
        let value = std::str::from_utf8(&plain).map_err(internal)?;

        Ok(Some(Secret::new(value)))
    }

    pub async fn status(&self, store: &Store, name: &str) -> Result<SecretStatus, Error> {
        let Some(value) = self.get(store, name).await? else {
            return Ok(SecretStatus {
                set: false,
                hint: None,
            });
        };

        Ok(status_of(&value))
    }

    pub fn seal(&self, aad: &str, value: &Secret) -> Result<Vec<u8>, Error> {
        self.vault
            .encrypt(aad.as_bytes(), value.expose().as_bytes())
    }

    pub fn open(&self, aad: &str, sealed: &[u8]) -> Result<Secret, Error> {
        let plain = self.vault.decrypt(aad.as_bytes(), sealed)?;

        Ok(Secret::new(std::str::from_utf8(&plain).map_err(internal)?))
    }

    pub async fn delete(&self, store: &Store, name: &str) -> Result<(), Error> {
        reject_reserved(name)?;

        let name = name.to_string();
        store.run(move |c| secret::delete(c, &name)).await
    }
}

pub fn status_of(value: &Secret) -> SecretStatus {
    let value = value.expose();
    let count = value.chars().count();
    let hint = if count < MIN_HINT_SOURCE_CHARS {
        HINT_PREFIX.to_string()
    } else {
        let tail: String = value.chars().skip(count - HINT_CHARS).collect();
        format!("{HINT_PREFIX}{tail}")
    };

    SecretStatus {
        set: true,
        hint: Some(hint),
    }
}

fn reject_reserved(name: &str) -> Result<(), Error> {
    if name == CHECK_NAME {
        return Err(Error::InvalidArgument {
            field: "name".to_string(),
            message: format!("`{CHECK_NAME}` is reserved for the vault; pick another name"),
        });
    }

    Ok(())
}

// The vault names the missing dependency; the daemon knows which file or setting to point at.
fn with_remedy(error: Error, source: KeySource, at: &KeyLocation, creating: bool) -> Error {
    match (error, source) {
        (Error::DependencyMissing { help, .. }, KeySource::File) => Error::DependencyMissing {
            name: format!("vault key file {}", at.file.display()),
            help,
        },
        (Error::DependencyMissing { name, help }, KeySource::Keyring) if creating => {
            Error::DependencyMissing {
                name,
                help: format!("{help} or set {KEY_SOURCE_SETTING} = \"file\""),
            }
        }
        (error, _) => error,
    }
}
