use std::{
    ffi::OsString,
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use axum::{Router, http::StatusCode, routing::get};
use brr_core::{AssetKind, DisplayEnv, Paths, now_ms, settings};
use brr_store::asset;
use tempfile::TempDir;
use tokio::net::TcpListener;

use super::LIST_MAX_AGE_MS;
use crate::{App, Boot};

const CONFIG_WITH_FILE_VAULT: &str = "[vault]\nkey_source = \"file\"\n";
const RULES: &str = "/ad.js$script\n";

struct Lists {
    hits: Arc<AtomicUsize>,
    failing: Arc<AtomicBool>,
    url: String,
}

async fn serve_list() -> Lists {
    let hits = Arc::new(AtomicUsize::new(0));
    let failing = Arc::new(AtomicBool::new(false));
    let (counted, down) = (Arc::clone(&hits), Arc::clone(&failing));
    let router = Router::new().route(
        "/list.txt",
        get(move || async move {
            counted.fetch_add(1, Ordering::SeqCst);
            if down.load(Ordering::SeqCst) {
                return (StatusCode::INTERNAL_SERVER_ERROR, "");
            }

            (StatusCode::OK, RULES)
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/list.txt", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await });

    Lists { hits, failing, url }
}

async fn scratch_app(lists: &Lists) -> (TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths {
        config_dir: dir.path().join("config"),
        data_dir: dir.path().join("data"),
        state_dir: dir.path().join("state"),
        runtime_dir: dir.path().join("run").join("brr"),
    };
    fs::create_dir_all(&paths.config_dir).unwrap();
    fs::write(paths.config_file(), CONFIG_WITH_FILE_VAULT).unwrap();
    let env = vec![(
        "BRR_CONTENT__ADBLOCK_LISTS".to_owned(),
        format!("[\"{}\"]", lists.url),
    )];
    let (settings, sources) = settings::load(&paths.config_file(), &env).unwrap();

    let app = App::start(Boot {
        paths,
        settings,
        sources,
        env,
        display_env: DisplayEnv::default(),
        path_env: OsString::new(),
        cft_base_url: String::new(),
        version: "0.0.0-test",
    })
    .await
    .unwrap();

    (dir, app)
}

async fn rows(app: &App) -> Vec<brr_core::Asset> {
    app.store
        .run(|c| asset::by_kind(c, AssetKind::AdblockList))
        .await
        .unwrap()
}

async fn age_rows(app: &App) {
    for mut row in rows(app).await {
        row.updated_at = now_ms() - LIST_MAX_AGE_MS - 60 * 60 * 1000;
        app.store.run(move |c| asset::put(c, &row)).await.unwrap();
    }
}

#[tokio::test]
async fn refresh_and_failure() {
    let lists = serve_list().await;
    let (_dir, app) = scratch_app(&lists).await;

    let options = app.content.options(&app, true, false).await;
    assert!(options.adblock.is_some());
    assert_eq!(lists.hits.load(Ordering::SeqCst), 1);

    app.content.options(&app, true, false).await;
    assert_eq!(lists.hits.load(Ordering::SeqCst), 1);

    age_rows(&app).await;
    app.content.ensure_lists(&app).await.unwrap();
    assert_eq!(lists.hits.load(Ordering::SeqCst), 2);
    let renewed = rows(&app).await;
    assert!(now_ms() - renewed[0].updated_at < LIST_MAX_AGE_MS);

    age_rows(&app).await;
    let before = rows(&app).await;
    lists.failing.store(true, Ordering::SeqCst);
    app.content.ensure_lists(&app).await.unwrap();
    assert_eq!(lists.hits.load(Ordering::SeqCst), 3);

    let after = rows(&app).await;
    assert_eq!(after, before);
    assert_eq!(fs::read_to_string(&after[0].path).unwrap(), RULES);
    assert!(
        app.content
            .options(&app, true, false)
            .await
            .adblock
            .is_some()
    );
}

#[tokio::test]
async fn removed_list_is_deleted() {
    let lists = serve_list().await;
    let (_dir, app) = scratch_app(&lists).await;
    app.content.options(&app, true, false).await;
    let stale = rows(&app).await.remove(0);
    let orphan = brr_core::Asset {
        id: brr_core::AssetId::new(),
        path: stale.path.with_file_name("gone.txt"),
        ..stale.clone()
    };
    fs::write(&orphan.path, RULES).unwrap();
    let put = orphan.clone();
    app.store.run(move |c| asset::put(c, &put)).await.unwrap();

    app.content.ensure_lists(&app).await.unwrap();

    assert_eq!(rows(&app).await, vec![stale]);
    assert!(!orphan.path.exists());
}
