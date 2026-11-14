use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use brr_content::{Adblock, fetch_list};
use brr_core::{Asset, AssetId, AssetKind, Error, Paths, now_ms, sha256_hex};
use brr_engine::page::ContentOptions;
use brr_store::asset;
use tokio::{fs, io::AsyncWriteExt, sync::watch, time};
use tracing::warn;

use crate::{App, internal};

#[cfg(test)]
mod tests;

const FIRST_LOAD_TIMEOUT: Duration = Duration::from_secs(30);
const LIST_MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;
// A failing download retries at this pace instead of spinning on an overdue list.
const MIN_REFRESH: Duration = Duration::from_secs(60 * 60);
const PATH_HASH_CHARS: usize = 16;
const VERSION_HASH_CHARS: usize = 12;

pub struct Content {
    adblock: Adblock,
    started: AtomicBool,
    // True once the first load finished, whether or not it got any list.
    loaded: watch::Sender<bool>,
    dir: PathBuf,
}

impl Content {
    pub fn new(paths: &Paths) -> Content {
        Content {
            adblock: Adblock::start(),
            started: AtomicBool::new(false),
            loaded: watch::channel(false).0,
            dir: paths.data_dir.join("assets").join("adblock"),
        }
    }

    // Lists load on the first request for blocking, so a daemon whose Profiles never block ads
    // makes no network calls and keeps no timer.
    pub async fn options(
        &self,
        app: &Arc<App>,
        block_ads: bool,
        dismiss_cookies: bool,
    ) -> ContentOptions {
        if block_ads {
            // The load and the refresh after it run in one task of their own, so a caller that
            // stops waiting never cancels the load.
            if !self.started.swap(true, Ordering::SeqCst) {
                tokio::spawn(load_and_refresh(Arc::downgrade(app)));
            }
            let mut loaded = self.loaded.subscribe();
            if time::timeout(FIRST_LOAD_TIMEOUT, loaded.wait_for(|done| *done))
                .await
                .is_err()
            {
                warn!("adblock lists still loading; continuing without them");
            }
        }

        ContentOptions {
            adblock: block_ads.then(|| self.adblock.clone()),
            dismiss_cookies,
        }
    }

    async fn ensure_lists(&self, app: &App) -> Result<(), Error> {
        let urls = app.settings().content.adblock_lists.clone();
        let wanted: Vec<PathBuf> = urls.iter().map(|url| self.path_of(url)).collect();
        fs::create_dir_all(&self.dir).await.map_err(internal)?;

        let rows = app
            .store
            .run(|c| asset::by_kind(c, AssetKind::AdblockList))
            .await?;
        for row in rows.iter().filter(|row| !wanted.contains(&row.path)) {
            let id = row.id;
            app.store.run(move |c| asset::delete(c, &id)).await?;
            let _ = fs::remove_file(&row.path).await;
        }

        let now = now_ms();
        let mut texts = Vec::new();
        for (url, path) in urls.iter().zip(&wanted) {
            let cached = rows.iter().find(|row| row.path == *path);
            let fresh = cached.is_some_and(|row| now - row.updated_at < LIST_MAX_AGE_MS);
            if fresh && let Ok(text) = fs::read_to_string(path).await {
                texts.push(text);

                continue;
            }

            // An empty body would replace a good cache with nothing.
            let fetched = fetch_list(&app.http, url)
                .await
                .map_err(|error| error.to_string())
                .and_then(|text| {
                    if text.trim().is_empty() {
                        Err("the list is empty".to_owned())
                    } else {
                        Ok(text)
                    }
                });
            match fetched {
                Ok(text) => {
                    self.store(app, cached, path, &text).await?;
                    texts.push(text);
                }
                Err(error) => {
                    warn!(%error, "adblock list download failed; keeping the cached list");
                    if let Ok(text) = fs::read_to_string(path).await {
                        texts.push(text);
                    }
                }
            }
        }
        self.adblock.replace(texts);

        Ok(())
    }

    // The list is renamed into place, so a crash mid-write never leaves a truncated filter set.
    async fn store(
        &self,
        app: &App,
        cached: Option<&Asset>,
        path: &Path,
        text: &str,
    ) -> Result<(), Error> {
        let partial = path.with_extension("part");
        let mut file = fs::File::create(&partial).await.map_err(internal)?;
        file.write_all(text.as_bytes()).await.map_err(internal)?;
        file.sync_all().await.map_err(internal)?;
        drop(file);
        fs::rename(&partial, path).await.map_err(internal)?;

        let row = Asset {
            id: cached.map_or_else(AssetId::new, |row| row.id),
            kind: AssetKind::AdblockList,
            version: sha256_prefix(text.as_bytes(), VERSION_HASH_CHARS),
            path: path.to_owned(),
            updated_at: now_ms(),
        };

        app.store.run(move |c| asset::put(c, &row)).await
    }

    // A configured list that was never stored is overdue from the start.
    async fn oldest_update(&self, app: &App) -> i64 {
        let rows = app
            .store
            .run(|c| asset::by_kind(c, AssetKind::AdblockList))
            .await
            .unwrap_or_default();

        app.settings()
            .content
            .adblock_lists
            .iter()
            .map(|url| self.path_of(url))
            .map(|path| {
                rows.iter()
                    .find(|row| row.path == path)
                    .map_or(0, |row| row.updated_at)
            })
            .min()
            .unwrap_or_else(now_ms)
    }

    fn path_of(&self, url: &str) -> PathBuf {
        self.dir
            .join(sha256_prefix(url.as_bytes(), PATH_HASH_CHARS))
            .with_extension("txt")
    }
}

// Held weakly: a strong reference would keep the daemon lock after the last owner drops the App.
async fn load_and_refresh(app: Weak<App>) {
    let Some(live) = app.upgrade() else {
        return;
    };
    if let Err(error) = live.content.ensure_lists(&live).await {
        warn!(%error, "adblock lists not loaded; blocking nothing until the next refresh");
    }
    live.content.loaded.send_replace(true);
    drop(live);

    loop {
        let Some(live) = app.upgrade() else {
            return;
        };
        let oldest = live.content.oldest_update(&live).await;
        let wait = u64::try_from(oldest + LIST_MAX_AGE_MS - now_ms()).unwrap_or(0);
        drop(live);
        time::sleep(Duration::from_millis(wait).max(MIN_REFRESH)).await;

        let Some(live) = app.upgrade() else {
            return;
        };
        if let Err(error) = live.content.ensure_lists(&live).await {
            warn!(%error, "adblock list refresh failed");
        }
    }
}

fn sha256_prefix(bytes: &[u8], chars: usize) -> String {
    sha256_hex(bytes)[..chars].to_owned()
}
