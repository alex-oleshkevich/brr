use std::{collections::HashSet, fmt, sync::Arc};

use brr_core::{
    ConflictReason, Credential, CredentialId, CredentialSelectors, Error, LastLogin,
    PasswordCsvFormat, ProfileId, Secret, now_ms,
};
use brr_store::{Connection, credential, profile};
use brr_vault::csv::parse_password_csv;

use crate::{app::App, internal, services::secrets::Secrets};

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
    Ok(open_field(secrets, &row.id, USERNAME, &row.username_enc)?
        .expose()
        .to_owned())
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
        username_enc: seal_field(secrets, &credential_id, USERNAME, &new.username)?,
        password_enc: seal_field(secrets, &credential_id, PASSWORD, new.password.expose())?,
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
    })
}

pub fn view(
    secrets: &Secrets,
    row: &credential::CredentialRow,
    profile_ids: Vec<ProfileId>,
    last_login: Option<LastLogin>,
) -> Result<Credential, Error> {
    Ok(Credential {
        id: row.id,
        site: row.site.clone(),
        login_url: row.login_url.clone(),
        username: open_username(secrets, row)?,
        selectors: row.selectors.clone(),
        totp_set: row.totp_enc.is_some(),
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

    view(&app.secrets, &row, profile_ids, last_logins.remove(&id))
}

pub async fn unseal(app: &Arc<App>, credential_id: &CredentialId) -> Result<Unsealed, Error> {
    let id = *credential_id;
    let row = app.store.run(move |c| credential::get(c, &id)).await?;
    let secrets = &app.secrets;

    Ok(Unsealed {
        login_url: row.login_url.unwrap_or_else(|| row.site.clone()),
        username: open_field(secrets, &id, USERNAME, &row.username_enc)?,
        password: open_field(secrets, &id, PASSWORD, &row.password_enc)?,
        totp: row
            .totp_enc
            .map(|sealed| open_field(secrets, &id, TOTP, &sealed))
            .transpose()?,
        site: row.site,
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
        if open_field(secrets, &row.id, USERNAME, &row.username_enc)?.expose() == username {
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
                let username =
                    open_field(&secrets_app.secrets, &row.id, USERNAME, &row.username_enc)?;
                known.insert((row.site, username.expose().to_owned()));
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
