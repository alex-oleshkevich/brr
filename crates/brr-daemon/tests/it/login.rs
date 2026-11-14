#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::{
    collections::HashMap,
    io::Write,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Form, Router,
    extract::State,
    response::{Html, Redirect},
    routing::{get, post},
};
use brr_core::{
    CredentialId, Error, IdleOverrides, Instance, InstanceId, LoginFailure, Tab, WaitUntil,
};
use brr_daemon::{App, CallClient, ops};
use brr_engine::page::Target;
use brr_vault::totp::totp_code;
use serde_json::{Value, json};

use common::{KillOnDrop, add_profile, chrome_app, ctx, events_of};

const PROFILE: &str = "p";
const LOGIN_PATH: &str = "/login";
const STICKY_PATH: &str = "/sticky";
const USERNAME: &str = "u1";
const STICKY_USERNAME: &str = "u2";
const PASSWORD: &str = "S3cret-pw-9";
const WRONG_PASSWORD: &str = "not-the-password";
const TOTP_SECRET: &str = "JBSWY3DPEHPK3PXP";
const TOTP_WINDOW_S: u64 = 30;
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(30);
const START_PAGE: &str = "data:text/html,<p>start</p>";
const ACCOUNT_TEXT: &str = "Logged in as u1";
const LOGIN_PERFORMED: &str = "login.performed";

const STEP_USER: &str = "<form method=post action=/user><input id=u name=username><button type=submit>Next</button></form>";
const HOP_LOGIN_PATH: &str = "/hop-login";
const HOP_USER: &str = "<form method=post action=/hop><input id=u name=username><button type=submit>Next</button></form>";
const STEP_PASSWORD: &str = "<form method=post action=/pass><input id=p type=password name=password><button type=submit>Sign in</button></form>";
const STEP_TOTP: &str = "<form method=post action=/totp><input autocomplete=one-time-code name=code><button type=submit>Verify</button></form>";
// Logs in without navigating, and keeps the typed value in the (now hidden) field.
const STICKY: &str = "<form onsubmit=\"event.preventDefault(); this.style.display='none'; document.body.append('welcome')\"><input id=u name=user><input id=p type=password oninput=\"this.setAttribute('value', this.value)\"><button type=submit>Go</button></form>";

#[derive(Default)]
struct Received {
    username: Option<String>,
    password: Option<String>,
    code: Option<String>,
}

type Site = Arc<Mutex<Received>>;

fn current_codes() -> [String; 2] {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    [now, now - TOTP_WINDOW_S].map(|at| totp_code(TOTP_SECRET, at).unwrap().expose().to_owned())
}

// `hop_to`: another site that /hop sends the browser to, as an SSO hop or a late redirect would.
async fn serve(hop_to: String) -> (String, Site) {
    let site = Site::default();
    let app = Router::new()
        .route(HOP_LOGIN_PATH, get(async || Html(HOP_USER)))
        .route(
            "/hop",
            post(async move || Html(format!("<script>location.href='{hop_to}/pass'</script>"))),
        )
        .route("/login", get(async || Html(STEP_USER)))
        .route("/sticky", get(async || Html(STICKY)))
        .route(
            "/user",
            post(
                async |State(site): State<Site>, Form(form): Form<HashMap<String, String>>| {
                    site.lock().unwrap().username = form.get("username").cloned();

                    Html(STEP_PASSWORD)
                },
            ),
        )
        .route(
            "/pass",
            get(async || Html(STEP_PASSWORD)).post(
                async |State(site): State<Site>, Form(form): Form<HashMap<String, String>>| {
                    let password = form.get("password").cloned();
                    let accepted = password.as_deref() == Some(PASSWORD);
                    site.lock().unwrap().password = password;

                    Html(if accepted { STEP_TOTP } else { STEP_PASSWORD })
                },
            ),
        )
        .route(
            "/totp",
            post(
                async |State(site): State<Site>, Form(form): Form<HashMap<String, String>>| {
                    let code = form.get("code").cloned();
                    let accepted = code
                        .as_ref()
                        .is_some_and(|code| current_codes().contains(code));
                    site.lock().unwrap().code = code;

                    if accepted {
                        Ok(Redirect::to("/account"))
                    } else {
                        Err(Html(STEP_TOTP))
                    }
                },
            ),
        )
        .route(
            "/account",
            get(async || Html(format!("<p>{ACCOUNT_TEXT}</p>"))),
        )
        .with_state(site.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    (origin, site)
}

struct Fixture {
    _dir: tempfile::TempDir,
    app: Arc<App>,
    _browsers: KillOnDrop,
    client: CallClient,
    instance_id: InstanceId,
    tab: Tab,
    origin: String,
    site: Site,
    other_origin: String,
    other_site: Site,
}

async fn call(f: &Fixture, name: &str, input: Value) -> Result<Value, Error> {
    let mut context = ctx(&f.app);
    context.client = f.client.clone();

    ops::dispatch(context, ops::find(name).unwrap(), input).await
}

async fn fixture() -> Option<Fixture> {
    let (dir, app, build_id) = chrome_app(&[]).await?;
    add_profile(&app, build_id, PROFILE, false, IdleOverrides::default()).await;
    let browsers = KillOnDrop::new(&app, Vec::new());
    let (other_origin, other_site) = serve(String::new()).await;
    let (origin, site) = serve(other_origin.clone()).await;
    let client = ctx(&app).client;

    let mut context = ctx(&app);
    context.client = client.clone();
    let launched = ops::dispatch(
        context,
        ops::find("instance.launch").unwrap(),
        json!({"profile": PROFILE, "url": START_PAGE}),
    )
    .await
    .unwrap();
    let instance: Instance = serde_json::from_value(launched["instance"].clone()).unwrap();
    browsers.track(instance.id);

    Some(Fixture {
        _dir: dir,
        app,
        _browsers: browsers,
        client,
        instance_id: instance.id,
        tab: serde_json::from_value(launched["tab"].clone()).unwrap(),
        origin,
        site,
        other_origin,
        other_site,
    })
}

async fn credential(f: &Fixture, password: &str, totp: bool, linked: bool, path: &str) -> Value {
    credential_for(f, USERNAME, password, totp, linked, path).await
}

async fn credential_for(
    f: &Fixture,
    username: &str,
    password: &str,
    totp: bool,
    linked: bool,
    path: &str,
) -> Value {
    let mut input = json!({
        "site": f.origin,
        "login_url": format!("{}{path}", f.origin),
        "username": username,
        "password": password,
    });
    if totp {
        input["totp"] = json!(TOTP_SECRET);
    }
    if linked {
        input["profile_ids"] = json!([PROFILE]);
    }

    call(f, "credential.create", input).await.unwrap()
}

async fn login(f: &Fixture, credential: &Value) -> Result<Value, Error> {
    call(
        f,
        "tab.login",
        json!({"tab_id": f.tab.id, "credential_id": credential["id"]}),
    )
    .await
}

#[tokio::test]
async fn multi_step_totp() {
    let Some(f) = fixture().await else { return };
    let created = credential(&f, PASSWORD, true, true, LOGIN_PATH).await;

    let done = login(&f, &created).await.unwrap();

    assert_eq!(done["ok"], true);
    assert_eq!(done["steps"], 3);
    assert!(done["url"].as_str().unwrap().ends_with("/account"));
    {
        let received = f.site.lock().unwrap();
        assert_eq!(received.username.as_deref(), Some(USERNAME));
        assert_eq!(received.password.as_deref(), Some(PASSWORD));
        assert!(current_codes().contains(received.code.as_ref().unwrap()));
    }
    let events = events_of(&f.app, LOGIN_PERFORMED, f.instance_id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["ok"], true);
    assert_eq!(events[0]["step"], 3);
}

#[tokio::test]
async fn unknown_credential_is_not_found() {
    let Some(f) = fixture().await else { return };

    let result = login(&f, &json!({"id": CredentialId::new().to_string()})).await;

    assert!(
        matches!(result, Err(Error::NotFound { ref what }) if what.starts_with("credential")),
        "{result:?}"
    );
}

#[tokio::test]
async fn not_linked() {
    let Some(f) = fixture().await else { return };
    let created = credential(&f, PASSWORD, true, false, LOGIN_PATH).await;

    let result = login(&f, &created).await;

    assert!(matches!(
        result,
        Err(Error::Conflict {
            reason: brr_core::ConflictReason::CredentialNotLinked,
            ..
        })
    ));
    let url = call(
        &f,
        "tab.evaluate",
        json!({"tab_id": f.tab.id, "expression": "location.href"}),
    )
    .await
    .unwrap();
    assert_eq!(url["result"], START_PAGE);
    assert!(
        events_of(&f.app, LOGIN_PERFORMED, f.instance_id)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn wrong_password() {
    let Some(f) = fixture().await else { return };
    let created = credential(&f, WRONG_PASSWORD, true, true, LOGIN_PATH).await;

    let result = login(&f, &created).await;

    assert_eq!(
        result,
        Err(Error::LoginFailed {
            step: 2,
            reason: LoginFailure::Rejected
        })
    );
    let events = events_of(&f.app, LOGIN_PERFORMED, f.instance_id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["ok"], false);
    assert_eq!(events[0]["step"], 2);
}

// The site hands the browser to another origin after the username: the password goes nowhere.
#[tokio::test]
async fn redirect_to_another_site_gets_no_password() {
    let Some(f) = fixture().await else { return };
    let created = credential(&f, PASSWORD, false, true, HOP_LOGIN_PATH).await;

    let result = login(&f, &created).await;

    assert_eq!(
        result,
        Err(Error::LoginFailed {
            step: 2,
            reason: LoginFailure::NoForm
        })
    );
    assert_eq!(f.other_site.lock().unwrap().password, None);
}

// The fill is bound to the origin that was vetted: in another site's document it types nothing.
#[tokio::test]
async fn pinned_fill_refuses_another_origin() {
    let Some(f) = fixture().await else { return };
    let page = f
        .app
        .instances
        .page(&f.instance_id, &f.tab.id)
        .await
        .unwrap();
    let target = Target::Selector("#u".to_owned());
    let navigate = async |origin: &str| {
        f.app
            .instances
            .navigate(
                &f.instance_id,
                &f.tab.id,
                format!("{origin}{LOGIN_PATH}"),
                WaitUntil::Load,
                NAVIGATE_TIMEOUT,
            )
            .await
            .unwrap();
    };
    navigate(&f.other_origin).await;

    let refused = page
        .fill_pinned(&target, PASSWORD, NAVIGATE_TIMEOUT, &f.origin)
        .await;

    assert_eq!(refused, Err(Error::StaleRef));
    let typed = page
        .evaluate_utility("document.querySelector('#u').value")
        .await
        .unwrap();
    assert_eq!(typed, "");
    page.fill_pinned(&target, PASSWORD, NAVIGATE_TIMEOUT, &f.other_origin)
        .await
        .unwrap();
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);

        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn no_secret_leaks() {
    let captured = Captured::default();
    let writer = captured.clone();
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || writer.clone())
            .finish(),
    );
    let Some(f) = fixture().await else { return };
    let created = credential(&f, PASSWORD, true, true, LOGIN_PATH).await;
    login(&f, &created).await.unwrap();
    // One Credential per site and username.
    let sticky = credential_for(&f, STICKY_USERNAME, PASSWORD, false, true, STICKY_PATH).await;
    assert_eq!(login(&f, &sticky).await.unwrap()["steps"], 1);

    let mut seen = vec![
        call(&f, "tab.snapshot", json!({"tab_id": f.tab.id}))
            .await
            .unwrap(),
        call(&f, "tab.get_html", json!({"tab_id": f.tab.id}))
            .await
            .unwrap(),
        json!(events_of(&f.app, LOGIN_PERFORMED, f.instance_id).await),
        json!(String::from_utf8_lossy(&captured.0.lock().unwrap())),
    ];
    seen.push(
        call(&f, "tab.console_messages", json!({"tab_id": f.tab.id}))
            .await
            .unwrap(),
    );

    // The sticky page keeps the typed value in a hidden field; a script may not read it back.
    let evaluated = call(
        &f,
        "tab.evaluate",
        json!({"tab_id": f.tab.id, "expression": "document.querySelector('#p').value"}),
    )
    .await;
    assert!(matches!(
        evaluated,
        Err(Error::Conflict {
            reason: brr_core::ConflictReason::SecretOnPage,
            ..
        })
    ));
    let code = f.site.lock().unwrap().code.clone().unwrap();
    for text in seen.iter().map(ToString::to_string) {
        for secret in [PASSWORD, TOTP_SECRET, code.as_str()] {
            assert!(!text.contains(secret), "{secret} leaked");
        }
    }
}
