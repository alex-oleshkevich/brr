mod api;
mod crypto;

use std::collections::HashMap;
use std::fmt;

use brr_core::Secret;
use reqwest::Client;
use rsa::PrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

use crypto::{Kdf, Key};

const LOGIN_TYPE: u8 = 1;
const KDF_PBKDF2: u8 = 0;
const KDF_ARGON2ID: u8 = 1;

pub struct Credentials {
    pub client_id: String,
    pub client_secret: Secret,
    pub master_password: Secret,
}

#[derive(Debug)]
pub struct ItemSummary {
    pub id: String,
    pub name: String,
    pub uris: Vec<String>,
    pub username: Option<String>,
    pub has_totp: bool,
    pub organization: Option<String>,
}

#[derive(Debug)]
pub struct Login {
    pub username: Secret,
    pub password: Secret,
    pub totp: Option<Secret>,
    pub uris: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid Bitwarden server URL: {0}")]
    InvalidServer(String),
    #[error(
        "Bitwarden rejected the API key ({0}); check client_id and client_secret under Account settings → Security → Keys → API key"
    )]
    Auth(String),
    #[error("the master password does not unlock this Bitwarden vault; check it and connect again")]
    WrongPassword,
    #[error("brr cannot read this Bitwarden vault: unsupported {0}")]
    Unsupported(String),
    #[error(
        "Bitwarden item {0} is not a login in the last sync; run bitwarden.sync or link another item"
    )]
    NotFound(String),
    #[error("cannot reach the Bitwarden server: {0}")]
    Network(String),
    #[error("unexpected response from the Bitwarden server: {0}")]
    Protocol(String),
}

impl From<crypto::Error> for Error {
    fn from(error: crypto::Error) -> Error {
        match error {
            crypto::Error::Mac => {
                Error::Protocol("an encrypted value failed its integrity check".into())
            }
            crypto::Error::Unsupported(what) => Error::Unsupported(what),
            crypto::Error::Malformed(what) => Error::Protocol(what.into()),
        }
    }
}

// Keys stay in memory only; ciphers stay encrypted until an item is listed or typed.
pub struct Vault {
    email: String,
    user_key: Key,
    organizations: HashMap<String, Organization>,
    ciphers: Vec<Cipher>,
}

struct Organization {
    name: String,
    key: Key,
}

#[derive(Deserialize)]
struct AccessToken {
    access_token: Secret,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct KdfParams {
    kdf: u8,
    kdf_iterations: u32,
    kdf_memory: Option<u32>,
    kdf_parallelism: Option<u32>,
}

#[derive(Deserialize)]
struct SyncResponse {
    profile: Profile,
    ciphers: Vec<Cipher>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    email: String,
    key: String,
    private_key: Option<String>,
    #[serde(default)]
    organizations: Vec<ProfileOrganization>,
}

#[derive(Deserialize)]
struct ProfileOrganization {
    id: String,
    name: String,
    key: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cipher {
    id: String,
    organization_id: Option<String>,
    #[serde(rename = "type")]
    kind: u8,
    name: String,
    key: Option<String>,
    login: Option<CipherLogin>,
    deleted_date: Option<String>,
}

#[derive(Deserialize)]
struct CipherLogin {
    username: Option<String>,
    password: Option<String>,
    totp: Option<String>,
    uris: Option<Vec<CipherUri>>,
}

#[derive(Deserialize)]
struct CipherUri {
    uri: Option<String>,
}

impl Vault {
    pub async fn open(
        http: &Client,
        server_url: &str,
        device_id: &str,
        creds: &Credentials,
    ) -> Result<Vault, Error> {
        let endpoints = api::endpoints(server_url)?;
        let token = api::token(http, &endpoints.identity, device_id, creds).await?;
        let AccessToken { access_token } = parse(&token, "token response")?;
        let sync = api::sync(http, &endpoints.api, access_token.expose()).await?;
        let master_password = Secret::new(creds.master_password.expose());
        // Key derivation takes up to seconds of CPU; keep it off the async workers.
        tokio::task::spawn_blocking(move || Vault::unlock(&token, &sync, &master_password))
            .await
            .map_err(|e| Error::Protocol(format!("unlocking the vault stopped: {e}")))?
    }

    // The offline half of `open`: the identity token response and the sync response as received.
    pub fn unlock(token: &[u8], sync: &[u8], master_password: &Secret) -> Result<Vault, Error> {
        let params: KdfParams = parse(token, "token response")?;
        let SyncResponse { profile, ciphers } = parse(sync, "sync response")?;

        let kdf = match (params.kdf, params.kdf_memory, params.kdf_parallelism) {
            (KDF_PBKDF2, _, _) => Kdf::Pbkdf2 {
                iterations: params.kdf_iterations,
            },
            (KDF_ARGON2ID, Some(memory_mib), Some(parallelism)) => Kdf::Argon2id {
                iterations: params.kdf_iterations,
                memory_mib,
                parallelism,
            },
            (KDF_ARGON2ID, _, _) => {
                return Err(Error::Protocol(
                    "Argon2id without memory or parallelism".into(),
                ));
            }
            (other, _, _) => return Err(Error::Unsupported(format!("KDF type {other}"))),
        };

        let email = profile.email.trim().to_lowercase();
        let master_key = crypto::master_key(master_password.expose(), &email, &kdf)?;
        let stretched = crypto::stretch(&master_key)?;
        let user_key = crypto::decrypt(&profile.key, &stretched).map_err(|e| match e {
            crypto::Error::Mac => Error::WrongPassword,
            e => e.into(),
        })?;
        let user_key = Key::from_bytes(&user_key)?;

        let mut organizations = HashMap::new();
        let confirmed: Vec<_> = profile
            .organizations
            .into_iter()
            .filter_map(|org| Some((org.id, org.name, org.key?)))
            .collect();
        if !confirmed.is_empty() {
            let private_key = profile
                .private_key
                .ok_or_else(|| Error::Protocol("organizations without a private key".into()))?;
            let der = crypto::decrypt(&private_key, &user_key)?;
            let private_key = PrivateKey::from_pkcs8_der(&der)
                .map_err(|_| Error::Protocol("the private key is not PKCS#8 RSA".into()))?;
            for (id, name, key) in confirmed {
                let key = Key::from_bytes(&crypto::decrypt_rsa(&key, &private_key)?)?;
                organizations.insert(id, Organization { name, key });
            }
        }

        let ciphers = ciphers
            .into_iter()
            .filter(|cipher| cipher.kind == LOGIN_TYPE && cipher.deleted_date.is_none())
            .collect();

        Ok(Vault {
            email,
            user_key,
            organizations,
            ciphers,
        })
    }

    pub fn email(&self) -> &str {
        &self.email
    }

    // An item that fails to decrypt is left out rather than hiding every other item.
    pub fn items(&self) -> Vec<ItemSummary> {
        self.ciphers
            .iter()
            .filter_map(|cipher| self.summary(cipher).ok())
            .collect()
    }

    pub fn login(&self, item_id: &str) -> Result<Login, Error> {
        let (cipher, login) = self
            .ciphers
            .iter()
            .filter(|cipher| cipher.id == item_id)
            .find_map(|cipher| Some((cipher, cipher.login.as_ref()?)))
            .ok_or_else(|| Error::NotFound(api::clean(item_id)))?;
        let key = self.cipher_key(cipher)?;

        Ok(Login {
            username: decrypt_secret(login.username.as_deref(), &key)?,
            password: decrypt_secret(login.password.as_deref(), &key)?,
            totp: login
                .totp
                .as_deref()
                .map(|totp| decrypt_text(totp, &key).map(Secret::new))
                .transpose()?,
            uris: decrypt_uris(login, &key)?,
        })
    }

    fn summary(&self, cipher: &Cipher) -> Result<ItemSummary, Error> {
        let key = self.cipher_key(cipher)?;
        let login = cipher.login.as_ref();

        Ok(ItemSummary {
            id: cipher.id.clone(),
            name: decrypt_text(&cipher.name, &key)?,
            uris: login
                .map(|login| decrypt_uris(login, &key))
                .transpose()?
                .unwrap_or_default(),
            username: login
                .and_then(|login| login.username.as_deref())
                .map(|username| decrypt_text(username, &key))
                .transpose()?,
            has_totp: login.is_some_and(|login| login.totp.is_some()),
            organization: cipher
                .organization_id
                .as_ref()
                .and_then(|id| self.organizations.get(id))
                .map(|org| org.name.clone()),
        })
    }

    fn cipher_key(&self, cipher: &Cipher) -> Result<Key, Error> {
        let base = match &cipher.organization_id {
            Some(id) => {
                &self
                    .organizations
                    .get(id)
                    .ok_or_else(|| {
                        Error::Protocol("an item belongs to an unknown organization".into())
                    })?
                    .key
            }
            None => &self.user_key,
        };

        match &cipher.key {
            Some(item_key) => Ok(Key::from_bytes(&crypto::decrypt(item_key, base)?)?),
            None => Ok(base.clone()),
        }
    }
}

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vault")
            .field("email", &self.email)
            .field("items", &self.ciphers.len())
            .finish_non_exhaustive()
    }
}

fn parse<T: DeserializeOwned>(body: &[u8], what: &str) -> Result<T, Error> {
    serde_json::from_slice(body)
        .map_err(|e| Error::Protocol(format!("{what}: {}", api::clean(&e.to_string()))))
}

fn decrypt_text(enc_string: &str, key: &Key) -> Result<String, Error> {
    let mut plain = crypto::decrypt(enc_string, key)?;
    String::from_utf8(std::mem::take(&mut *plain)).map_err(|e| {
        drop(Zeroizing::new(e.into_bytes()));
        Error::Protocol("a decrypted value is not UTF-8".into())
    })
}

fn decrypt_secret(enc_string: Option<&str>, key: &Key) -> Result<Secret, Error> {
    let text = enc_string.map(|e| decrypt_text(e, key)).transpose()?;
    Ok(Secret::new(text.unwrap_or_default()))
}

fn decrypt_uris(login: &CipherLogin, key: &Key) -> Result<Vec<String>, Error> {
    login
        .uris
        .iter()
        .flatten()
        .filter_map(|uri| uri.uri.as_deref())
        .map(|uri| decrypt_text(uri, key))
        .collect()
}

#[cfg(test)]
mod tests;
