// Each test binary uses a different subset of these helpers.
#![allow(dead_code, clippy::disallowed_macros)]

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use brr_core::{
    AdmissionMode, Build, BuildChoice, BuildId, BuildSource, BuildState, ClientId, ClientKind,
    Display, DisplayEnv, Engine, IdleOverrides, Instance, InstanceId, InstanceKind, InstanceState,
    Order, Page, Paths, Profile, ProfileId, ProfileKind, ProfileSettings, Tab, TabId, TabState,
    TargetId, now_ms, settings,
};
use brr_daemon::{App, Boot, CallClient, Ctx};
use brr_engine::procs::kill_marked;
use brr_store::{build, event, instance, profile};
use tempfile::TempDir;
use tokio::time;

pub const VERSION: &str = "0.0.0-test";
pub const TEST_BUILD_VERSION: &str = "999.0.0.0";
const CHROME_ENV: &str = "BRR_TEST_CHROME";
const LEASE_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);

const SETTLE_POLL: Duration = Duration::from_millis(10);

const CONFIG_WITH_FILE_VAULT: &str = "[vault]\nkey_source = \"file\"\n";

pub fn paths(dir: &Path) -> Paths {
    Paths {
        config_dir: dir.join("config"),
        data_dir: dir.join("data"),
        state_dir: dir.join("state"),
        runtime_dir: dir.join("run").join("brr"),
    }
}

pub fn boot(dir: &Path, env: &[(&str, &str)]) -> Boot {
    let paths = paths(dir);
    fs::create_dir_all(&paths.config_dir).unwrap();
    fs::write(paths.config_file(), CONFIG_WITH_FILE_VAULT).unwrap();

    let env: Vec<(String, String)> = env
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    let (settings, sources) = settings::load(&paths.config_file(), &env).unwrap();

    Boot {
        paths,
        settings,
        sources,
        env,
        display_env: DisplayEnv::default(),
        path_env: OsString::new(),
        cft_base_url: String::new(),
        version: VERSION,
    }
}

pub async fn scratch_app() -> (TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[])).await.unwrap();

    (dir, app)
}

pub fn ctx(app: &Arc<App>) -> Ctx {
    Ctx {
        app: Arc::clone(app),
        client: CallClient {
            client_id: ClientId::new(),
            kind: ClientKind::Cli,
            label: "test".to_string(),
            implicit: true,
        },
    }
}

pub async fn settled(mut done: impl AsyncFnMut() -> bool) {
    time::timeout(LEASE_SETTLE_TIMEOUT, async {
        while !done().await {
            time::sleep(SETTLE_POLL).await;
        }
    })
    .await
    .unwrap();
}

pub fn chrome() -> Option<PathBuf> {
    let chrome = std::env::var_os(CHROME_ENV).map(PathBuf::from);
    if chrome.is_none() {
        eprintln!("skipped: {CHROME_ENV} is not set");
    }

    chrome
}

// Kills what a failing test leaves behind; a stopped instance has nothing left to kill.
pub struct KillOnDrop(Arc<App>, Mutex<Vec<InstanceId>>);

impl KillOnDrop {
    pub fn new(app: &Arc<App>, instance_ids: Vec<InstanceId>) -> KillOnDrop {
        KillOnDrop(Arc::clone(app), Mutex::new(instance_ids))
    }

    pub fn track(&self, instance_id: InstanceId) {
        self.1.lock().unwrap().push(instance_id);
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let instance_ids = self.1.lock().unwrap().iter().copied().collect();
        kill_marked(&instance_ids);
    }
}

pub fn instance_row(state: InstanceState, data_dir: Option<PathBuf>) -> Instance {
    Instance {
        id: InstanceId::new(),
        kind: InstanceKind::Managed,
        profile_id: None,
        build_id: None,
        state,
        display: Display::Headless,
        priority: 0,
        admission: AdmissionMode::Queue,
        pid: None,
        log_path: None,
        cdp_url: None,
        ephemeral: data_dir.is_some(),
        data_dir,
        promote_to_name: None,
        restart_count: 0,
        last_activity_at: 0,
        queued_at: 0,
        started_at: None,
        ended_at: None,
        end_reason: None,
        error: None,
        profile_name: None,
        build: None,
        tab_count: 0,
        lease_count: 0,
        rss_bytes: None,
        capabilities: Vec::new(),
    }
}

pub fn tab_row(instance_id: InstanceId, state: TabState) -> Tab {
    Tab {
        id: TabId::new(),
        instance_id,
        target_id: TargetId("target".to_string()),
        state,
        url: "about:blank".to_string(),
        title: String::new(),
        opened_by_brr: true,
        pinned: false,
        last_activity_at: 0,
        created_at: 0,
        closed_at: None,
        lease: None,
    }
}

pub async fn register_build(app: &Arc<App>, chrome: &Path) -> BuildId {
    let registered = Build {
        id: BuildId::new(),
        source: BuildSource::System,
        engine: Engine::Chromium,
        name: "Chrome".to_owned(),
        version: TEST_BUILD_VERSION.to_owned(),
        channel: None,
        path: chrome.to_path_buf(),
        state: BuildState::Registered,
        sha256: None,
        size_bytes: 0,
        capabilities: Vec::new(),
        installed_at: None,
        last_referenced_at: None,
    };
    let build_id = registered.id;
    app.store
        .run(move |c| build::insert(c, &registered))
        .await
        .unwrap();

    build_id
}

const MCP_PROTOCOL_VERSION: &str = "2026-07-28";
const MCP_META_PREFIX: &str = "io.modelcontextprotocol/";

pub async fn listen(app: &Arc<App>) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(std::future::IntoFuture::into_future(axum::serve(
        listener,
        brr_daemon::http::router(Arc::clone(app)),
    )));

    addr
}

pub async fn post(
    addr: std::net::SocketAddr,
    query: &str,
    method: &str,
    mut params: serde_json::Value,
) -> reqwest::Response {
    params["_meta"] = serde_json::json!({
        format!("{MCP_META_PREFIX}protocolVersion"): MCP_PROTOCOL_VERSION,
        format!("{MCP_META_PREFIX}clientInfo"): { "name": "test", "version": "0" },
        format!("{MCP_META_PREFIX}clientCapabilities"): {},
    });
    let name = params["name"].as_str().map(str::to_string);
    let mut request = reqwest::Client::new()
        .post(format!("http://{addr}/mcp{query}"))
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION)
        .header("Mcp-Method", method)
        .json(
            &serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
        );
    if let Some(name) = name {
        request = request.header("Mcp-Name", name);
    }

    request.send().await.unwrap()
}

pub async fn rpc(
    addr: std::net::SocketAddr,
    query: &str,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let response = post(addr, query, method, params).await;
    let status = response.status();
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(status.is_success(), "{status}: {body}");

    body
}

pub async fn tool_names(addr: std::net::SocketAddr, query: &str) -> Vec<String> {
    let body = rpc(addr, query, "tools/list", serde_json::json!({})).await;

    body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

const POLL_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_STEP: Duration = Duration::from_millis(50);
const STATE_CHANGED: &str = "instance.state_changed";

pub async fn wait_for(what: &str, mut done: impl AsyncFnMut() -> bool) {
    let found = time::timeout(POLL_TIMEOUT, async {
        while !done().await {
            time::sleep(POLL_STEP).await;
        }
    })
    .await;

    assert!(found.is_ok(), "timed out waiting for {what}");
}

pub async fn row(app: &Arc<App>, instance_id: InstanceId) -> Instance {
    app.store
        .run(move |c| instance::get(c, &instance_id))
        .await
        .unwrap()
}

pub async fn events_of(
    app: &Arc<App>,
    kind: &str,
    instance_id: InstanceId,
) -> Vec<serde_json::Value> {
    let query = event::Query {
        since: None,
        kinds: vec![kind.to_owned()],
        subject: None,
        instance_id: Some(instance_id),
        from_ts: None,
        to_ts: None,
        order: Order::Asc,
        page: Page::new(Some(1000), None),
    };
    let (events, _) = app
        .store
        .run(move |c| event::list(c, &query))
        .await
        .unwrap();

    events.into_iter().map(|event| event.data).collect()
}

pub async fn triggers(app: &Arc<App>, instance_id: InstanceId) -> Vec<String> {
    events_of(app, STATE_CHANGED, instance_id)
        .await
        .iter()
        .map(|data| data["trigger"].as_str().unwrap().to_owned())
        .collect()
}

pub async fn chrome_app(env: &[(&str, &str)]) -> Option<(TempDir, Arc<App>, BuildId)> {
    let chrome = chrome()?;
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), env)).await.unwrap();
    let build_id = register_build(&app, &chrome).await;

    Some((dir, app, build_id))
}

pub async fn add_profile(
    app: &Arc<App>,
    build_id: BuildId,
    name: &str,
    pinned: bool,
    idle: IdleOverrides,
) -> Profile {
    let now = now_ms();
    let created = Profile {
        id: ProfileId::new(),
        name: name.to_owned(),
        kind: ProfileKind::Ephemeral,
        template_id: None,
        settings: ProfileSettings {
            build: BuildChoice::BuildId(build_id),
            display: Display::Headless,
            idle,
            ..ProfileSettings::default()
        },
        pinned,
        notes: String::new(),
        tags: Vec::new(),
        data_dir: None,
        disk_quota_bytes: None,
        backup_schedule: None,
        backup_keep: 0,
        created_at: now,
        updated_at: now,
        capabilities: Vec::new(),
        login_status: None,
        builtin: false,
    };
    let stored = created.clone();
    app.store
        .run(move |c| profile::insert(c, &stored))
        .await
        .unwrap();

    created
}

pub fn profile_row(
    kind: ProfileKind,
    data_dir: Option<PathBuf>,
    disk_quota_bytes: Option<u64>,
) -> Profile {
    let now = now_ms();

    Profile {
        id: ProfileId::new(),
        name: "p".to_owned(),
        kind,
        template_id: None,
        settings: ProfileSettings::default(),
        pinned: false,
        notes: String::new(),
        tags: Vec::new(),
        data_dir,
        disk_quota_bytes,
        backup_schedule: None,
        backup_keep: 0,
        created_at: now,
        updated_at: now,
        capabilities: Vec::new(),
        login_status: None,
        builtin: false,
    }
}
