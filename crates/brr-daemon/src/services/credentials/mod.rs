use std::{collections::HashSet, fmt, sync::Arc};

use brr_core::{
    ConflictReason, Credential, CredentialId, CredentialSelectors, CredentialSource, Error,
    LastLogin, PasswordCsvFormat, ProfileId, Secret, now_ms, site_origin,
};
use brr_store::{Connection, credential, profile};
use brr_vault::csv::parse_password_csv;

use crate::{
    app::App,
    internal,
    services::{bitwarden, secrets::Secrets},
};

#[cfg(test)]
mod tests;

pub const AAD_PREFIX: &str = "credential";
pub const USERNAME: &str = "username";
pub const PASSWORD: &str = "password";
pub const TOTP: &str = "totp";
pub const NOTES: &str = "notes";

pub struct Unsealed {
    pub site: String,
    pub login_url: String,
    pub username: Secret,
    pub password: Secret,
    pub totp: Option<Secret>,
    pub selectors: CredentialSelectors,
}

impl fmt::Debug for Unsealed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Unsealed")
            .field("site", &self.site)
            .finish_non_exhaustive()
    }
}

pub struct NewCredential {
    pub site: String,
    pub login_url: Option<String>,
    pub username: String,
    pub password: Secret,
    pub totp: Option<Secret>,
    pub notes: Option<String>,
    pub selectors: CredentialSelectors,
}

pub fn seal_field(
    secrets: &Secrets,
    credential_id: &CredentialId,
    field: &str,
    plaintext: &str,
) -> Result<Vec<u8>, Error> {
    secrets.seal(
        &format!("{AAD_PREFIX}/{credential_id}/{field}"),
        &Secret::new(plaintext),
    )
}

fn open_field(
    secrets: &Secrets,
    credential_id: &CredentialId,
    field: &str,
    sealed: &[u8],
) -> Result<Secret, Error> {
    secrets.open(&format!("{AAD_PREFIX}/{credential_id}/{field}"), sealed)
}

pub fn open_username(secrets: &Secrets, row: &credential::CredentialRow) -> Result<String, Error> {
    row.username_enc.as_ref().map_or_else(
        || Ok(String::new()),
        |sealed| {
            Ok(open_field(secrets, &row.id, USERNAME, sealed)?
                .expose()
                .to_owned())
        },
    )
}

pub fn new_row(
    secrets: &Secrets,
    credential_id: CredentialId,
    new: &NewCredential,
) -> Result<credential::CredentialRow, Error> {
    let now = now_ms();

    Ok(credential::CredentialRow {
        id: credential_id,
        site: new.site.clone(),
        login_url: new.login_url.clone(),
        username_enc: Some(seal_field(
            secrets,
            &credential_id,
            USERNAME,
            &new.username,
        )?),
        password_enc: Some(seal_field(
            secrets,
            &credential_id,
            PASSWORD,
            new.password.expose(),
        )?),
        totp_enc: new
            .totp
            .as_ref()
            .map(|totp| seal_field(secrets, &credential_id, TOTP, totp.expose()))
            .transpose()?,
        notes_enc: new
            .notes
            .as_deref()
            .map(|notes| seal_field(secrets, &credential_id, NOTES, notes))
            .transpose()?,
        selectors: new.selectors.clone(),
        created_at: now,
        updated_at: now,
        source: CredentialSource::Local,
        bitwarden_item_id: None,
    })
}

pub fn view(
    app: &App,
    row: &credential::CredentialRow,
    profile_ids: Vec<ProfileId>,
    last_login: Option<LastLogin>,
) -> Result<Credential, Error> {
    let (username, totp_set) = match &row.bitwarden_item_id {
        Some(item_id) if row.source == CredentialSource::Bitwarden => {
            app.bitwarden.summary(item_id)
        }
        _ => (open_username(&app.secrets, row)?, row.totp_enc.is_some()),
    };

    Ok(Credential {
        id: row.id,
        site: row.site.clone(),
        login_url: row.login_url.clone(),
        username,
        selectors: row.selectors.clone(),
        totp_set,
        source: row.source,
        bitwarden_item_id: row.bitwarden_item_id.clone(),
        profile_ids,
        last_login,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub fn resolve_profiles(c: &Connection, profiles: &[String]) -> Result<Vec<ProfileId>, Error> {
    let mut ids = profiles
        .iter()
        .map(|profile| profile::find(c, profile).map(|found| found.id))
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort();
    ids.dedup();

    Ok(ids)
}

pub async fn load(app: &Arc<App>, credential_id: &CredentialId) -> Result<Credential, Error> {
    let id = *credential_id;
    let (row, profile_ids, mut last_logins) = app
        .store
        .run(move |c| {
            Ok((
                credential::get(c, &id)?,
                credential::profiles(c, &id)?,
                credential::last_logins(c)?,
            ))
        })
        .await?;

    view(app, &row, profile_ids, last_logins.remove(&id))
}

pub async fn unseal(app: &Arc<App>, credential_id: &CredentialId) -> Result<Unsealed, Error> {
    let id = *credential_id;
    let row = app.store.run(move |c| credential::get(c, &id)).await?;
    let secrets = &app.secrets;

    if row.source == CredentialSource::Bitwarden {
        return unseal_bitwarden(app, row);
    }
    let (Some(username_enc), Some(password_enc)) = (&row.username_enc, &row.password_enc) else {
        return Err(internal(format!("credential {id} has no sealed login")));
    };

    Ok(Unsealed {
        login_url: row.login_url.unwrap_or_else(|| row.site.clone()),
        username: open_field(secrets, &id, USERNAME, username_enc)?,
        password: open_field(secrets, &id, PASSWORD, password_enc)?,
        totp: row
            .totp_enc
            .map(|sealed| open_field(secrets, &id, TOTP, &sealed))
            .transpose()?,
        site: row.site,
        selectors: row.selectors,
    })
}

// The item's URIs are the source of truth: a page the item does not list never gets its secrets,
// even when the item changed after it was linked.
fn unseal_bitwarden(app: &App, row: credential::CredentialRow) -> Result<Unsealed, Error> {
    let Some(item_id) = row.bitwarden_item_id else {
        return Err(internal(format!(
            "credential {} has no Bitwarden item",
            row.id
        )));
    };
    let login = bitwarden::login(app, &item_id)?;
    let login_url = row.login_url.unwrap_or_else(|| row.site.clone());
    let listed = |url: &str| {
        site_origin(url).is_some_and(|origin| {
            login
                .uris
                .iter()
                .any(|uri| site_origin(uri).as_ref() == Some(&origin))
        })
    };
    if !listed(&row.site) || !listed(&login_url) {
        return Err(Error::InvalidArgument {
            field: "credential_id".to_owned(),
            message: format!(
                "Bitwarden item {item_id} does not list {} or the login URL among its URIs; fix the item or link it again",
                row.site
            ),
        });
    }

    Ok(Unsealed {
        site: row.site,
        login_url,
        username: login.username,
        password: login.password,
        totp: login.totp,
        selectors: row.selectors,
    })
}

// One credential per site and username; `except` is the credential being updated. Called inside
// the same store closure as the write, so two concurrent writes cannot both pass.
pub fn ensure_unique(
    secrets: &Secrets,
    c: &Connection,
    site: &str,
    username: &str,
    except: Option<&CredentialId>,
) -> Result<(), Error> {
    let rows = credential::all(c)?;

    for row in rows
        .iter()
        .filter(|row| row.site == site && Some(&row.id) != except)
    {
        if open_username(secrets, row)? == username {
            return Err(Error::Conflict {
                reason: ConflictReason::CredentialExists,
                detail: None,
            });
        }
    }

    Ok(())
}

pub async fn import(
    app: &Arc<App>,
    format: PasswordCsvFormat,
    text: String,
    profile_ids: Vec<String>,
) -> Result<(u32, u32), Error> {
    let parsed = tokio::task::spawn_blocking(move || parse_password_csv(&text, format))
        .await
        .map_err(internal)??;
    let skipped = parsed.skipped;
    let secrets_app = Arc::clone(app);

    app.store
        .run(move |c| {
            let linked = resolve_profiles(c, &profile_ids)?;

            let mut known: HashSet<(String, String)> = HashSet::new();
            for row in credential::all(c)? {
                let username = open_username(&secrets_app.secrets, &row)?;
                known.insert((row.site, username));
            }

            let mut skipped = skipped;
            let mut rows = Vec::new();
            for login in parsed.logins {
                if !known.insert((login.site.clone(), login.username.clone())) {
                    skipped += 1;
                    continue;
                }

                let new = NewCredential {
                    site: login.site,
                    login_url: Some(login.login_url),
                    username: login.username,
                    password: login.password,
                    totp: None,
                    notes: login.note,
                    selectors: CredentialSelectors::default(),
                };
                rows.push(new_row(&secrets_app.secrets, CredentialId::new(), &new)?);
            }

            let tx = c.transaction().map_err(internal)?;
            for row in &rows {
                credential::insert(&tx, row)?;
                credential::set_profiles(&tx, &row.id, &linked)?;
            }
            tx.commit().map_err(internal)?;

            Ok((u32::try_from(rows.len()).map_err(internal)?, skipped))
        })
        .await
}
