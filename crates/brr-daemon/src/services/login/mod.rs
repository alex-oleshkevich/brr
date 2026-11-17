use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use brr_core::{
    ConflictReason, CredentialId, Error, EventData, LoginFailure, WaitUntil, now_ms, site_origin,
};
use brr_engine::{
    Navigated,
    page::{ClickOptions, MouseButton, Page, Target},
};
use brr_store::credential;
use brr_vault::totp::{STEP_SECONDS, totp_code};
use tracing::info;

use crate::{
    app::App,
    ops::TabCall,
    services::credentials::{Unsealed, unseal},
};

#[cfg(test)]
mod tests;

const MAX_STEPS: u8 = 3;
const FIRST_STEP: u8 = 1;
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(30);
const STEP_WAIT: Duration = Duration::from_secs(15);
const ACTION_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(200);
const MS_PER_S: u64 = 1000;
// A code that expires within this long is not worth typing: the site may check it late.
const TOTP_MIN_REMAINING_MS: u64 = 5000;
const ENTER: &str = "Enter";
const LINK_HELP: &str =
    "link the credential to this Tab's Profile first (credential.update profile_ids)";

#[derive(Debug)]
pub struct LoginOutcome {
    pub steps: u8,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Username,
    Password,
    Totp,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Progress {
    Done,
    Advance,
    Stuck,
}

// The visible fields of the page now, as selectors.
#[derive(Default)]
struct Found {
    username: Option<String>,
    password: Option<String>,
    totp: Option<String>,
    submit: Option<String>,
}

impl Found {
    fn selector(&self, field: Field) -> Option<&String> {
        match field {
            Field::Username => self.username.as_ref(),
            Field::Password => self.password.as_ref(),
            Field::Totp => self.totp.as_ref(),
        }
    }

    fn present(&self) -> Vec<Field> {
        [Field::Username, Field::Password, Field::Totp]
            .into_iter()
            .filter(|field| self.selector(*field).is_some())
            .collect()
    }
}

// How long to wait before computing a code: one that is about to expire waits for the next window.
pub fn totp_wait(now_ms: u64) -> Duration {
    let window_ms = STEP_SECONDS * MS_PER_S;
    let remaining_ms = window_ms - now_ms % window_ms;
    if remaining_ms < TOTP_MIN_REMAINING_MS {
        Duration::from_millis(remaining_ms)
    } else {
        Duration::ZERO
    }
}

// The fields of this step: the ones present that no earlier step filled, in typing order.
pub fn to_fill(present: &[Field], done: &[Field]) -> Vec<Field> {
    [Field::Username, Field::Password, Field::Totp]
        .into_iter()
        .filter(|field| present.contains(field) && !done.contains(field))
        .collect()
}

// After a submit. Through only once a password or code went in and no field asks for more; a
// new password or code field means the next step; anything else, the site did not take it.
// A username field is ignored once the password is typed: a logged-in page may show one.
pub fn progress(present: &[Field], done: &[Field]) -> Progress {
    let secret_typed = done.contains(&Field::Password) || done.contains(&Field::Totp);
    let asking: Vec<&Field> = present
        .iter()
        .filter(|field| !(secret_typed && **field == Field::Username))
        .collect();

    if asking
        .iter()
        .any(|field| **field != Field::Username && !done.contains(field))
    {
        Progress::Advance
    } else if asking.is_empty() && secret_typed {
        Progress::Done
    } else {
        Progress::Stuck
    }
}

fn now_unix_ms() -> u64 {
    u64::try_from(now_ms()).unwrap_or_default()
}

fn failed(step: u8, reason: LoginFailure) -> Error {
    Error::LoginFailed { step, reason }
}

async fn visible_selector(page: &Page, selector: Option<&String>) -> Result<Option<String>, Error> {
    let Some(selector) = selector else {
        return Ok(None);
    };
    let visible = page
        .selector_visible(&Target::Selector(selector.clone()))
        .await?;

    Ok(visible.then(|| selector.clone()))
}

// The credential's own selector decides when it is set; otherwise the page's field finder.
async fn find(page: &Page, creds: &Unsealed) -> Result<Found, Error> {
    let own = &creds.selectors;
    let fields = page.login_fields().await?;
    let pick = async |own: &Option<String>, found: Option<String>| match own {
        Some(_) => visible_selector(page, own.as_ref()).await,
        None => Ok(found),
    };

    Ok(Found {
        username: pick(&own.username_selector, fields.username).await?,
        password: pick(&own.password_selector, fields.password).await?,
        totp: pick(&own.totp_selector, fields.totp).await?,
        submit: fields.submit,
    })
}

// The page's origin when a secret may be typed into it: only into the site it belongs to.
async fn vetted_origin(page: &Page, creds: &Unsealed) -> Result<Option<String>, Error> {
    let origin = site_origin(&page.url().await?);
    let allowed = origin.is_some()
        && (origin.as_ref() == Some(&creds.site) || origin == site_origin(&creds.login_url));

    Ok(origin.filter(|_| allowed))
}

async fn fill_field(
    page: &Page,
    creds: &Unsealed,
    field: Field,
    selector: &str,
    step: u8,
) -> Result<(), Error> {
    let target = Target::Selector(selector.to_owned());
    if field == Field::Username {
        return page
            .fill(&target, creds.username.expose(), ACTION_TIMEOUT)
            .await;
    }

    // The code is computed and the wait over before the origin is checked, so nothing slow sits
    // between the check and the typing.
    let code = match field {
        Field::Totp => {
            let Some(secret) = &creds.totp else {
                return Err(failed(step, LoginFailure::NoTotpSecret));
            };

            tokio::time::sleep(totp_wait(now_unix_ms())).await;

            Some(totp_code(secret.expose(), now_unix_ms() / MS_PER_S)?)
        }
        _ => None,
    };
    let value = code.as_ref().unwrap_or(&creds.password);

    // Registered and masked before the first character is typed.
    page.state().add_secret(value.expose());
    page.mask(&target, ACTION_TIMEOUT).await?;

    // The fill re-checks this origin in the page itself, in the call that sets the value.
    let Some(origin) = vetted_origin(page, creds).await? else {
        return Err(failed(step, LoginFailure::NoForm));
    };

    match page
        .fill_pinned(&target, value.expose(), ACTION_TIMEOUT, &origin)
        .await
    {
        Err(Error::StaleRef) => Err(failed(step, LoginFailure::NoForm)),
        other => other,
    }
}

async fn submit(page: &Page, creds: &Unsealed, found: &Found) -> Result<(), Error> {
    let selector = creds
        .selectors
        .submit_selector
        .as_ref()
        .or(found.submit.as_ref());

    match selector {
        Some(selector) => {
            let options = ClickOptions {
                button: MouseButton::Left,
                modifiers: Vec::new(),
                click_count: 1,
            };

            page.click(
                &Target::Selector(selector.clone()),
                &options,
                ACTION_TIMEOUT,
            )
            .await
        }
        None => page.press_key(None, ENTER, ACTION_TIMEOUT).await,
    }
}

// Until the main frame navigated and loaded since `navigations`, or a password or code field it
// did not have appeared; a page that does neither is judged as it is.
async fn wait_after_submit(
    page: &Page,
    creds: &Unsealed,
    before: &[Field],
    navigations: u64,
    budget: Duration,
) {
    let appeared = async {
        loop {
            // Errors mean the page is mid-navigation.
            if let Ok(found) = find(page, creds).await
                && found
                    .present()
                    .iter()
                    .any(|field| *field != Field::Username && !before.contains(field))
            {
                return;
            }

            tokio::time::sleep(POLL_INTERVAL).await;
        }
    };

    let _ = tokio::time::timeout(budget, async {
        tokio::select! {
            () = page.wait_navigated(navigations) => {}
            () = appeared => {}
        }
    })
    .await;
}

// `step` is the step that was running when this ended, for the caller's event.
async fn run_steps(
    app: &Arc<App>,
    tab: &TabCall,
    creds: &Unsealed,
    deadline: Instant,
    step: &mut u8,
) -> Result<(), Error> {
    let page = &tab.page;
    let mut done: Vec<Field> = Vec::new();
    let mut found = find(page, creds).await?;

    loop {
        *step += 1;
        let present = found.present();
        let todo = to_fill(&present, &done);
        let no_login_field =
            !present.contains(&Field::Username) && !present.contains(&Field::Password);
        if (*step == 1 && no_login_field) || todo.is_empty() {
            return Err(failed(*step, LoginFailure::NoForm));
        }

        let targets: Vec<(Field, String)> = todo
            .into_iter()
            .filter_map(|field| found.selector(field).map(|s| (field, s.clone())))
            .collect();
        for (field, selector) in targets {
            fill_field(page, creds, field, &selector, *step).await?;
            done.push(field);
        }

        let navigations = page.navigation_count();
        submit(page, creds, &found).await?;
        let budget = STEP_WAIT.min(deadline.saturating_duration_since(Instant::now()));
        wait_after_submit(page, creds, &present, navigations, budget).await;

        found = find(page, creds).await?;
        let present = found.present();
        let progressed = progress(&present, &done);
        if progressed == Progress::Advance && *step < MAX_STEPS {
            continue;
        }

        // The Profile's login check outranks the page: a site may show a password field to a
        // person who is logged in (a change-password form).
        let checked = app.login_checks.check_site(app, tab, &creds.site).await?;
        return match (checked, progressed) {
            (Some(true), _) => Ok(()),
            (Some(false), _) => Err(failed(*step, LoginFailure::NotLoggedIn)),
            (None, Progress::Done) if !page.password_visible().await? => Ok(()),
            (None, Progress::Done) => Err(failed(*step, LoginFailure::NotLoggedIn)),
            (None, _) if present.is_empty() => Err(failed(*step, LoginFailure::NoForm)),
            (None, _) => Err(failed(*step, LoginFailure::Rejected)),
        };
    }
}

async fn prepare(
    app: &Arc<App>,
    tab: &TabCall,
    credential_id: &CredentialId,
    deadline: Instant,
) -> Result<Unsealed, Error> {
    let linked = match tab.profile_id {
        Some(profile_id) => {
            let id = *credential_id;
            app.store
                .run(move |c| {
                    credential::get(c, &id)?;

                    credential::is_linked(c, &id, &profile_id)
                })
                .await?
        }
        None => false,
    };
    if !linked {
        return Err(Error::Conflict {
            reason: ConflictReason::CredentialNotLinked,
            detail: Some(LINK_HELP.to_owned()),
        });
    }

    // Always: the form loads in a fresh document, so no script the Tab ran earlier is still
    // listening on it.
    let creds = unseal(app, credential_id).await?;
    let (_, navigated) = app
        .instances
        .navigate(
            &tab.instance_id,
            &tab.tab_id,
            creds.login_url.clone(),
            WaitUntil::Load,
            NAVIGATE_TIMEOUT.min(deadline.saturating_duration_since(Instant::now())),
        )
        .await?;
    // A file instead of a page: there is no form to fill.
    if navigated == Navigated::Download {
        return Err(failed(FIRST_STEP, LoginFailure::NoForm));
    }

    Ok(creds)
}

// Fills the login form of the Tab's page with the credential, step by step, within `timeout`.
// Nothing here puts the password or a code into an error, an event or a log line.
pub async fn login(
    app: &Arc<App>,
    tab: &TabCall,
    credential_id: &CredentialId,
    timeout: Duration,
) -> Result<LoginOutcome, Error> {
    let deadline = Instant::now() + timeout;
    let timed_out = || Error::Timeout {
        after_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
    };
    // Held to the end, so a script cannot read what is typed, however often the page navigates.
    let _no_evaluate = tab.page.lock_evaluate();
    let creds =
        tokio::time::timeout_at(deadline.into(), prepare(app, tab, credential_id, deadline))
            .await
            .map_err(|_| timed_out())??;

    let mut step = 0;
    let result = tokio::time::timeout_at(
        deadline.into(),
        run_steps(app, tab, &creds, deadline, &mut step),
    )
    .await
    .unwrap_or_else(|_| Err(timed_out()));

    let ok = result.is_ok();
    if step > 0 {
        app.events
            .publish(EventData::LoginPerformed {
                tab_id: tab.tab_id,
                instance_id: tab.instance_id,
                credential_id: *credential_id,
                ok,
                step: Some(step),
            })
            .await?;
    }
    info!(tab_id = %tab.tab_id, %credential_id, ok, step, "login");

    result?;

    Ok(LoginOutcome {
        steps: step,
        url: tab.page.url().await?,
    })
}
