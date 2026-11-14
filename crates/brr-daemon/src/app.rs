use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, DirBuilder},
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
    sync::{Arc, PoisonError, RwLock},
    time::Duration,
};

use brr_core::{
    ClientId, ClientKind, DisplayEnv, Error, EventData, Paths, Settings, Source, Sources, settings,
};
use brr_store::{Store, build};
use brr_vault::KeyLocation;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{
    internal,
    lock::DaemonLock,
    services::{
        admission::Admission,
        backups::{self, Exports},
        builds::channels,
        camoufox::CamoufoxAssets,
        captcha::Captcha,
        clients::{self, Clients},
        content::Content,
        events::EventBus,
        instances::Instances,
        leases::Leases,
        login_checks::LoginChecks,
        oneshot::{self, Oneshot},
        profiles,
        proxies::{self, ProxyTable},
        reaper,
        relay::Relay,
        retention,
        screencast::Screencasts,
        secrets::{DEFAULT_KEY_SOURCE, KEY_SOURCE_SETTING, Secrets},
        storage::Storage,
        supervisor::Supervisor,
    },
};

const RUNTIME_DIR_MODE: u32 = 0o700;
const MODE_BITS: u32 = 0o777;
const INSTANCES_DIR: &str = "instances";
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
// Per read, not per request: a stalled download fails, a slow big one keeps going.
const HTTP_READ_TIMEOUT: Duration = Duration::from_secs(60);
const VAULT_SECTION: &str = "vault.";
const STORAGE_SECTION: &str = "storage.";

pub struct Boot {
    pub paths: Paths,
    pub settings: Settings,
    pub sources: Sources,
    // The BRR_* pairs given to settings::load, kept for reloads.
    pub env: Vec<(String, String)>,
    pub display_env: DisplayEnv,
    pub path_env: OsString,
    pub cft_base_url: String,
    pub version: &'static str,
}

pub struct App {
    pub paths: Paths,
    pub display_env: DisplayEnv,
    pub path_env: OsString,
    pub cft_base_url: String,
    pub version: &'static str,
    pub store: Store,
    pub events: EventBus,
    pub clients: Clients,
    pub leases: Leases,
    pub supervisor: Supervisor,
    pub instances: Instances,
    pub screencasts: Screencasts,
    pub admission: Admission,
    pub proxies: ProxyTable,
    pub secrets: Secrets,
    pub storage: Storage,
    pub oneshot: Oneshot,
    pub content: Content,
    pub relay: Relay,
    pub login_checks: LoginChecks,
    pub exports: Exports,
    pub captcha: Captcha,
    pub camoufox: CamoufoxAssets,
    pub http: reqwest::Client,
    settings: RwLock<(Arc<Settings>, Sources)>,
    env: Vec<(String, String)>,
    settings_write: Mutex<()>,
    _lock: DaemonLock,
}

#[derive(Clone)]
pub struct Ctx {
    pub app: Arc<App>,
    pub client: CallClient,
}

#[derive(Clone, Debug)]
pub struct CallClient {
    pub client_id: ClientId,
    pub kind: ClientKind,
    pub label: String,
    pub implicit: bool,
}

impl App {
    pub async fn start(boot: Boot) -> Result<Arc<App>, Error> {
        let paths = boot.paths;

        for dir in [&paths.config_dir, &paths.data_dir] {
            fs::create_dir_all(dir).map_err(internal)?;
        }
        fs::create_dir_all(paths.state_dir.join(INSTANCES_DIR)).map_err(internal)?;
        create_private_dir(&paths.runtime_dir)?;

        let lock = DaemonLock::acquire(&paths.lock_file())?;
        // Under the lock no other daemon is using the scratch files an earlier run left.
        retention::clear_stale(&paths);
        let store = Store::open(&paths.db_file())?;
        store.run(|c| build::mark_interrupted(c)).await?;
        profiles::seed_templates(&store).await?;
        oneshot::ensure_profile(&store, &boot.settings).await?;

        let configured = boot.settings.vault.key_source;
        let at = KeyLocation {
            file: paths.vault_key_file(),
            keyring_user: paths.data_dir.to_string_lossy().into_owned(),
        };
        let secrets = Secrets::bootstrap(&store, &at, configured).await?;

        let mut loaded = (Arc::new(boot.settings), boot.sources);
        if configured.is_none() {
            // The vault was just created with the file source; pin it so a later start never guesses.
            let default = serde_json::to_value(DEFAULT_KEY_SOURCE).map_err(internal)?;
            let changes = BTreeMap::from([(KEY_SOURCE_SETTING.to_string(), default)]);
            loaded = patch_and_load(paths.config_file(), changes, boot.env.clone()).await?;
        }

        let storage = Storage::open(&store, &secrets, &paths, &loaded.0.storage).await?;

        let events = EventBus::new(store.clone()).await?;
        clients::reconcile(&store, &events).await?;
        let http = reqwest::Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .read_timeout(HTTP_READ_TIMEOUT)
            .build()
            .map_err(internal)?;

        tracing::info!(version = boot.version, runtime_dir = %paths.runtime_dir.display(), "daemon started");

        let supervisor = Supervisor::new()?;
        let content = Content::new(&paths);

        let app = Arc::new(App {
            paths,
            display_env: boot.display_env,
            path_env: boot.path_env,
            cft_base_url: boot.cft_base_url,
            version: boot.version,
            store,
            events,
            clients: Clients::default(),
            leases: Leases::default(),
            supervisor,
            instances: Instances::default(),
            screencasts: Screencasts::default(),
            proxies: ProxyTable::default(),
            admission: Admission::new(&loaded.0),
            secrets,
            storage,
            oneshot: Oneshot::default(),
            content,
            relay: Relay::default(),
            login_checks: LoginChecks::default(),
            exports: Exports::default(),
            captcha: Captcha::default(),
            camoufox: CamoufoxAssets::default(),
            http,
            settings: RwLock::new(loaded),
            env: boot.env,
            settings_write: Mutex::new(()),
            _lock: lock,
        });
        Supervisor::reconcile(&app).await?;
        channels::spawn(Arc::clone(&app));
        proxies::spawn_health(Arc::clone(&app));
        reaper::spawn(Arc::clone(&app));
        oneshot::spawn_pool(&app);
        retention::spawn(&app);
        LoginChecks::spawn(&app);
        backups::start_scheduler(&app);
        Captcha::start(&app);

        Ok(app)
    }

    pub async fn shutdown(self: &Arc<App>) {
        self.oneshot.close();
        self.instances.shutdown_all(self).await;
    }

    pub fn settings(&self) -> Arc<Settings> {
        let guard = self.settings.read().unwrap_or_else(PoisonError::into_inner);

        Arc::clone(&guard.0)
    }

    pub fn sources(&self) -> Sources {
        let guard = self.settings.read().unwrap_or_else(PoisonError::into_inner);

        guard.1.clone()
    }

    pub async fn set_settings(
        self: &Arc<App>,
        changes: BTreeMap<String, Value>,
    ) -> Result<(), Error> {
        // The backend must pass its connection test and reload; only storage.backend_set does that.
        if let Some(key) = changes.keys().find(|key| key.starts_with(STORAGE_SECTION)) {
            return Err(Error::InvalidArgument {
                field: key.clone(),
                message: "use storage.backend_set to change the storage backend".to_string(),
            });
        }

        self.apply_settings(changes).await
    }

    // The file patch, the in-memory swap and the event happen together even if the caller is dropped.
    pub(crate) async fn apply_settings(
        self: &Arc<App>,
        changes: BTreeMap<String, Value>,
    ) -> Result<(), Error> {
        let app = Arc::clone(self);

        tokio::spawn(async move { app.write_settings(changes).await })
            .await
            .map_err(internal)?
    }

    async fn write_settings(&self, changes: BTreeMap<String, Value>) -> Result<(), Error> {
        let _writing = self.settings_write.lock().await;
        let sources = self.sources();

        for key in changes.keys() {
            if sources.get(key) == Some(&Source::Env) {
                let env_name = format!("BRR_{}", key.replacen('.', "__", 1).to_uppercase());

                return Err(Error::InvalidArgument {
                    field: key.clone(),
                    message: format!("set by env {env_name}; change it there"),
                });
            }

            if key.starts_with(VAULT_SECTION) {
                return Err(Error::InvalidArgument {
                    field: key.clone(),
                    message: format!("{key} is fixed when the vault is created"),
                });
            }
        }

        let keys: Vec<String> = changes.keys().cloned().collect();
        let loaded = patch_and_load(self.paths.config_file(), changes, self.env.clone()).await?;
        *self
            .settings
            .write()
            .unwrap_or_else(PoisonError::into_inner) = loaded;

        self.events
            .publish(EventData::SettingsChanged { keys })
            .await?;

        Ok(())
    }
}

async fn patch_and_load(
    config_file: PathBuf,
    changes: BTreeMap<String, Value>,
    env: Vec<(String, String)>,
) -> Result<(Arc<Settings>, Sources), Error> {
    tokio::task::spawn_blocking(move || {
        settings::patch(&config_file, &changes)?;
        let (settings, sources) = settings::load(&config_file, &env)?;

        Ok((Arc::new(settings), sources))
    })
    .await
    .map_err(internal)?
}

// The runtime dir holds the socket; a directory someone else owns or can write to
// would let them take over the daemon.
fn create_private_dir(dir: &Path) -> Result<(), Error> {
    DirBuilder::new()
        .recursive(true)
        .mode(RUNTIME_DIR_MODE)
        .create(dir)
        .map_err(internal)?;

    // symlink_metadata: a symlink planted at the path must not pass for the directory.
    let metadata = fs::symlink_metadata(dir).map_err(internal)?;
    let is_dir = metadata.is_dir();
    let owned = metadata.uid() == nix::unistd::geteuid().as_raw();
    let private = metadata.mode() & MODE_BITS == RUNTIME_DIR_MODE;

    if is_dir && owned && private {
        return Ok(());
    }

    Err(internal(format!(
        "runtime dir {} must be owned by you with mode 0700; remove it or point XDG_RUNTIME_DIR elsewhere",
        dir.display()
    )))
}
