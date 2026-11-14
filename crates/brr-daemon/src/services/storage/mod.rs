use std::sync::{Arc, PoisonError, RwLock};

use brr_core::{
    Error, Paths,
    settings::{STORAGE_SECRET, StorageSettings},
};
use brr_storage::{Backend, config};
use brr_store::Store;
use tokio::sync::{Mutex, MutexGuard};
use tracing::warn;

use crate::{app::App, services::secrets::Secrets};

pub struct Storage {
    backend: RwLock<Backend>,
    change: Mutex<()>,
}

impl Storage {
    // A bad config must not keep the daemon from starting: Artifacts fall back to Local and
    // storage.backend_get shows the error.
    pub async fn open(
        store: &Store,
        secrets: &Secrets,
        paths: &Paths,
        settings: &StorageSettings,
    ) -> Result<Storage, Error> {
        let backend = match open_backend(store, secrets, paths, settings).await {
            Ok(backend) => backend,
            Err(error) => {
                warn!(
                    error = format!("{error:#}"),
                    "storage backend not usable; falling back to local"
                );

                Backend::open(config(
                    &StorageSettings::default(),
                    None,
                    &paths.storage_dir(),
                )?)?
            }
        };

        Ok(Storage {
            backend: RwLock::new(backend),
            change: Mutex::default(),
        })
    }

    // One backend change at a time: its settings and its secret must come from the same call.
    pub async fn changing(&self) -> MutexGuard<'_, ()> {
        self.change.lock().await
    }

    pub fn backend(&self) -> Backend {
        self.backend
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub async fn reload(&self, app: &Arc<App>) -> Result<(), Error> {
        let settings = app.settings();
        let backend = open_backend(&app.store, &app.secrets, &app.paths, &settings.storage).await?;
        *self.backend.write().unwrap_or_else(PoisonError::into_inner) = backend;

        Ok(())
    }
}

pub async fn open_backend(
    store: &Store,
    secrets: &Secrets,
    paths: &Paths,
    settings: &StorageSettings,
) -> Result<Backend, Error> {
    let secret = secrets.get(store, STORAGE_SECRET).await?;

    Backend::open(config(settings, secret, &paths.storage_dir())?)
}
