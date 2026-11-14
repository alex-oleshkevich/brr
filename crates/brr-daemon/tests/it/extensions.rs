#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::{
    fs,
    io::{Cursor, Write},
    net::SocketAddr,
    path::Path,
    sync::Arc,
    time::Duration,
};

use axum::{Router, routing::get};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use brr_core::{
    AdmissionMode, Build, BuildChoice, BuildId, BuildSource, BuildState, Capability,
    ConflictReason, Engine, Error, Extension, ExtensionId, ExtensionSource, ExtensionVersion,
    IdleOverrides, ProfileExtension, now_ms,
};
use brr_daemon::{
    App,
    http::MAX_REQUEST_BYTES,
    ops,
    services::{extensions, gc, instances::LaunchRequest},
};
use brr_store::{build, extension, profile_extension};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tar::{EntryType, Header};
use tokio::{net::TcpListener, time};
use zip::{ZipWriter, write::SimpleFileOptions};

use common::{KillOnDrop, add_profile, boot, chrome, ctx, listen, scratch_app, wait_for};

const CWS_ID: &str = "abcdefghijklmnopabcdefghijklmnop";
const PUBLIC_KEY: &[u8] = b"fixture-public-key-der";
const MANIFEST: &str = r#"{"manifest_version":3,"name":"Fixture","version":"1.0.0"}"#;
const DAY_MS: i64 = 86_400_000;
const NEVER_ANSWERS: Duration = Duration::from_secs(3600);
const BUDGET_MARGIN: Duration = Duration::from_secs(2);
const CONTENT_MANIFEST: &str = r#"{"manifest_version":3,"name":"Titles","version":"1.0","content_scripts":[{"matches":["http://127.0.0.1/*"],"js":["content.js"],"run_at":"document_idle"}]}"#;
const CONTENT_SCRIPT: &str = "document.title = 'ext-ok';";

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    let op = ops::find(name).unwrap();

    ops::dispatch(ctx(app), op, input).await
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(data).unwrap();
    }

    writer.finish().unwrap().into_inner()
}

fn protobuf(field: u64, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for number in [(field << 3) | 2, value.len() as u64] {
        let mut rest = number;
        loop {
            let byte = (rest & 0x7f) as u8;
            rest >>= 7;
            if rest == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }
    out.extend_from_slice(value);

    out
}

fn crx3(zip: &[u8]) -> Vec<u8> {
    let proof = protobuf(1, PUBLIC_KEY);
    let crx_id = Sha256::digest(PUBLIC_KEY)[..16].to_vec();
    let mut header = protobuf(2, &proof);
    header.extend(protobuf(10_000, &protobuf(1, &crx_id)));
    let mut out = b"Cr24".to_vec();
    out.extend(3u32.to_le_bytes());
    out.extend((header.len() as u32).to_le_bytes());
    out.extend(header);
    out.extend_from_slice(zip);

    out
}

fn raw_entry(name: &str, kind: EntryType, data: &[u8], link: Option<&str>) -> Header {
    let mut header = Header::new_gnu();
    header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
    header.set_entry_type(kind);
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    if let Some(link) = link {
        header.set_link_name(link).unwrap();
    }
    header.set_cksum();

    header
}

// The CLI's folder upload: a "./" entry first, no "./" prefix on files.
fn tar_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    builder
        .append(&raw_entry("./", EntryType::Directory, b"", None), &b""[..])
        .unwrap();
    for (name, data) in files {
        builder
            .append(&raw_entry(name, EntryType::Regular, data, None), *data)
            .unwrap();
    }

    builder.into_inner().unwrap()
}

fn upload(bytes: &[u8]) -> Value {
    json!(STANDARD.encode(bytes))
}

async fn serve(router: Router) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    addr
}

async fn app_with(env: &[(&str, &str)]) -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), env)).await.unwrap();

    (dir, app)
}

fn chromium_build(source: BuildSource, capabilities: Vec<Capability>) -> Build {
    Build {
        id: BuildId::new(),
        source,
        engine: Engine::Chromium,
        name: "build".to_owned(),
        version: "1.0.0.0".to_owned(),
        channel: None,
        path: "/nonexistent".into(),
        state: BuildState::Installed,
        sha256: None,
        size_bytes: 0,
        capabilities,
        installed_at: Some(now_ms()),
        last_referenced_at: None,
    }
}

async fn insert_build(app: &Arc<App>, row: Build) {
    app.store
        .run(move |c| build::insert(c, &row))
        .await
        .unwrap();
}

async fn pinned_profile(app: &Arc<App>, build_id: BuildId, name: &str) -> brr_core::Profile {
    add_profile(app, build_id, name, false, IdleOverrides::default()).await
}

fn invalid_field(result: Result<Value, Error>) -> String {
    match result {
        Err(Error::InvalidArgument { field, .. }) => field,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

async fn set_extensions(
    app: &Arc<App>,
    profile: &str,
    extension_id: &Value,
) -> Result<Value, Error> {
    call(
        app,
        "profile.extensions_set",
        json!({"profile": profile, "extensions": [
            {"extension_id": extension_id, "pinned_version": null, "auto_update": false}
        ]}),
    )
    .await
}

#[tokio::test]
async fn oversized_body_gets_the_json_error() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;

    let response = reqwest::Client::new()
        .post(format!("http://{addr}/v1/ops/settings.get"))
        .body(vec![b' '; MAX_REQUEST_BYTES + 1])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 413);
    assert_eq!(
        response.json::<Value>().await.unwrap()["code"],
        "RequestTooLarge"
    );
}

#[tokio::test]
async fn add_cws() {
    let archive = crx3(&zip_of(&[
        ("manifest.json", MANIFEST.as_bytes()),
        ("_metadata/verified_contents.json", b"{}"),
    ]));
    let addr = serve(Router::new().route("/crx", get(move || async move { archive }))).await;
    let url = format!("http://{addr}/crx");
    let (_dir, app) = app_with(&[("BRR_EXTENSIONS__CWS_UPDATE_URL", &url)]).await;

    let added = call(&app, "extension.add", json!({"ref": CWS_ID}))
        .await
        .unwrap();
    let listed = call(&app, "extension.list", json!({})).await.unwrap();

    assert_eq!(added["engine"], "chromium");
    assert_eq!(added["source"], "cws");
    let dir = Path::new(added["versions"][0]["path"].as_str().unwrap());
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["key"], STANDARD.encode(PUBLIC_KEY));
    assert!(!dir.join("_metadata").exists());
    assert_eq!(listed["extensions"][0]["profiles"], json!([]));
    assert_eq!(listed["extensions"][0]["latest_version"], "1.0.0");
}

#[tokio::test]
async fn add_local_dedupes_and_accepts_a_dot_entry() {
    let (_dir, app) = scratch_app().await;
    let tar = tar_of(&[("manifest.json", MANIFEST.as_bytes())]);

    let first = call(&app, "extension.add", json!({"file": upload(&tar)}))
        .await
        .unwrap();
    let again = call(&app, "extension.add", json!({"file": upload(&tar)}))
        .await
        .unwrap();
    let both = call(
        &app,
        "extension.add",
        json!({"file": upload(&tar), "ref": CWS_ID}),
    )
    .await;

    assert_eq!(first["id"], again["id"]);
    assert_eq!(first["source"], "local");
    assert_eq!(first["engine"], "chromium");
    assert_eq!(invalid_field(both), "ref");
}

fn referenced_detail(result: Result<Value, Error>) -> String {
    match result {
        Err(Error::Conflict {
            reason: ConflictReason::Referenced,
            detail,
        }) => detail.unwrap(),
        other => panic!("expected a Referenced conflict, got {other:?}"),
    }
}

#[tokio::test]
async fn remove_is_refused_while_a_profile_enables_the_extension() {
    let (dir, app) = scratch_app().await;
    let tar = tar_of(&[("manifest.json", MANIFEST.as_bytes())]);
    let added = call(&app, "extension.add", json!({"file": upload(&tar)}))
        .await
        .unwrap();
    let build_row = chromium_build(BuildSource::System, vec![Capability::UnpackedExtensions]);
    let build_id = build_row.id;
    insert_build(&app, build_row).await;
    let profile = pinned_profile(&app, build_id, "enabler").await;
    set_extensions(&app, &profile.name, &added["id"])
        .await
        .unwrap();
    let remove = json!({"extension_id": added["id"]});

    let refused = call(&app, "extension.remove", remove.clone()).await;

    assert!(referenced_detail(refused).contains(&profile.name));
    let files = dir
        .path()
        .join("data/extensions")
        .join(added["id"].as_str().unwrap());
    assert!(files.exists());

    call(
        &app,
        "profile.extensions_set",
        json!({"profile": profile.name, "extensions": []}),
    )
    .await
    .unwrap();
    call(&app, "extension.remove", remove).await.unwrap();

    assert!(!files.exists());
}

#[tokio::test]
async fn engine_mismatch() {
    let (_dir, app) = scratch_app().await;
    let xpi = zip_of(&[(
        "manifest.json",
        br#"{"manifest_version":2,"name":"Fox","version":"1.0","browser_specific_settings":{"gecko":{"id":"fox@example"}}}"#,
    )]);
    let added = call(&app, "extension.add", json!({"file": upload(&xpi)}))
        .await
        .unwrap();
    let row = chromium_build(BuildSource::System, vec![Capability::UnpackedExtensions]);
    let build_id = row.id;
    insert_build(&app, row).await;
    let profile = pinned_profile(&app, build_id, "chromium-user").await;

    let enabled = set_extensions(&app, &profile.name, &added["id"]).await;

    assert_eq!(added["engine"], "camoufox");
    assert_eq!(invalid_field(enabled), "extensions");
}

#[tokio::test]
async fn engine_unsupported() {
    let (_dir, app) = scratch_app().await;
    let tar = tar_of(&[("manifest.json", MANIFEST.as_bytes())]);
    let added = call(&app, "extension.add", json!({"file": upload(&tar)}))
        .await
        .unwrap();
    let row = chromium_build(BuildSource::HeadlessShell, vec![Capability::Pdf]);
    let build_id = row.id;
    insert_build(&app, row).await;
    let profile = pinned_profile(&app, build_id, "shell-user").await;

    let enabled = set_extensions(&app, &profile.name, &added["id"]).await;

    assert_eq!(
        enabled.err(),
        Some(Error::EngineUnsupported {
            capability: Capability::UnpackedExtensions
        })
    );
}

#[tokio::test]
async fn tar_traversal_rejected() {
    let (dir, app) = scratch_app().await;
    let mut dotdot = tar::Builder::new(Vec::new());
    dotdot
        .append(
            &raw_entry(
                "manifest.json",
                EntryType::Regular,
                MANIFEST.as_bytes(),
                None,
            ),
            MANIFEST.as_bytes(),
        )
        .unwrap();
    dotdot
        .append(
            &raw_entry("../evil", EntryType::Regular, b"x", None),
            &b"x"[..],
        )
        .unwrap();
    let mut link = tar::Builder::new(Vec::new());
    link.append(
        &raw_entry(
            "manifest.json",
            EntryType::Regular,
            MANIFEST.as_bytes(),
            None,
        ),
        MANIFEST.as_bytes(),
    )
    .unwrap();
    link.append(
        &raw_entry("evil", EntryType::Symlink, b"", Some("/etc")),
        &b""[..],
    )
    .unwrap();

    for archive in [dotdot.into_inner().unwrap(), link.into_inner().unwrap()] {
        let added = call(&app, "extension.add", json!({"file": upload(&archive)})).await;

        assert_eq!(invalid_field(added), "file");
    }

    let data = dir.path().join("data");
    assert!(!data.join("evil").exists());
    assert!(!data.join("extensions/evil").exists());
    assert!(!dir.path().join("evil").exists());
    let leftovers = fs::read_dir(data.join("extensions"))
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0);
}

#[tokio::test]
async fn update_budget() {
    let addr = serve(Router::new().route(
        "/crx",
        get(|| async {
            time::sleep(NEVER_ANSWERS).await;
            "late"
        }),
    ))
    .await;
    let url = format!("http://{addr}/crx");
    let (dir, app) = app_with(&[("BRR_EXTENSIONS__CWS_UPDATE_URL", &url)]).await;
    let version_dir = dir.path().join("data/extensions/installed/1.0.0");
    fs::create_dir_all(&version_dir).unwrap();
    let extension_id = ExtensionId::new();
    let row = Extension {
        id: extension_id,
        engine: Engine::Chromium,
        source: ExtensionSource::Cws,
        source_ref: CWS_ID.to_owned(),
        name: "Installed".to_owned(),
        versions: Vec::new(),
    };
    let version = ExtensionVersion {
        version: "1.0.0".to_owned(),
        path: version_dir.clone(),
        size_bytes: 0,
        fetched_at: now_ms(),
        last_referenced_at: None,
    };
    let build_row = chromium_build(BuildSource::System, vec![Capability::UnpackedExtensions]);
    let build_id = build_row.id;
    insert_build(&app, build_row.clone()).await;
    let profile = pinned_profile(&app, build_id, "updater").await;
    let (profile_id, links) = (
        profile.id,
        vec![ProfileExtension {
            extension_id,
            pinned_version: None,
            auto_update: true,
        }],
    );
    app.store
        .run(move |c| {
            extension::insert(c, &row)?;
            extension::add_version(c, &row.id, &version)?;
            profile_extension::set(c, &profile_id, &links)
        })
        .await
        .unwrap();

    time::pause();
    let started = time::Instant::now();
    let paths = extensions::launch_paths(&app, &profile, &build_row)
        .await
        .unwrap();

    assert_eq!(paths, vec![version_dir]);
    assert!(started.elapsed() < extensions::UPDATE_BUDGET + BUDGET_MARGIN);
}

async fn origin() -> SocketAddr {
    serve(Router::new().route(
        "/",
        get(|| async { axum::response::Html("<title>plain</title>") }),
    ))
    .await
}

#[tokio::test]
async fn loads_in_browser() {
    let Some(chrome_path) = chrome() else {
        return;
    };
    let (_dir, app) = scratch_app().await;
    let tar = tar_of(&[
        ("manifest.json", CONTENT_MANIFEST.as_bytes()),
        ("content.js", CONTENT_SCRIPT.as_bytes()),
    ]);
    let added = call(&app, "extension.add", json!({"file": upload(&tar)}))
        .await
        .unwrap();
    let mut row = chromium_build(BuildSource::System, vec![Capability::UnpackedExtensions]);
    row.path = chrome_path;
    row.state = BuildState::Registered;
    let build_id = row.id;
    insert_build(&app, row).await;
    let mut profile = pinned_profile(&app, build_id, "with-extension").await;
    profile.settings.build = BuildChoice::BuildId(build_id);
    set_extensions(&app, &profile.name, &added["id"])
        .await
        .unwrap();
    let origin = origin().await;

    let (running, _) = app
        .instances
        .launch(
            &app,
            LaunchRequest {
                profile,
                display: None,
                priority: 0,
                admission: AdmissionMode::Queue,
                force: false,
            },
        )
        .await
        .unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    let tab = app
        .instances
        .open_tab(&running.id, Some(format!("http://{origin}/")))
        .await
        .unwrap();

    wait_for("the content script's title", async || {
        let view = app.instances.view(&running.id).unwrap();
        view.tabs
            .iter()
            .any(|t| t.id == tab.id && t.title == "ext-ok")
    })
    .await;

    // Removing it would pull the files from under the running browser, whatever the Profile enables.
    let library_id: ExtensionId = serde_json::from_value(added["id"].clone()).unwrap();
    let profile_id = running.profile_id.unwrap();
    app.store
        .run(move |c| profile_extension::set(c, &profile_id, &[]))
        .await
        .unwrap();
    let refused = call(
        &app,
        "extension.remove",
        json!({"extension_id": library_id}),
    )
    .await;
    assert!(referenced_detail(refused).contains("running instance"));

    // An update moved the Profile to 2.0; the running browser still has 1.0 loaded.
    app.store
        .run(move |c| {
            extension::add_version(
                c,
                &library_id,
                &ExtensionVersion {
                    version: "2.0".to_owned(),
                    path: "/nonexistent".into(),
                    size_bytes: 0,
                    fetched_at: now_ms(),
                    last_referenced_at: None,
                },
            )?;
            profile_extension::set(
                c,
                &profile_id,
                &[ProfileExtension {
                    extension_id: library_id,
                    pinned_version: Some("2.0".to_owned()),
                    auto_update: false,
                }],
            )
        })
        .await
        .unwrap();
    let later = now_ms() + DAY_MS;
    let loaded_item = format!("{library_id}@1.0");

    let while_running = gc::gc_candidates(&app, later, 0).await.unwrap();
    app.instances
        .stop(&app, &running.id, brr_core::EndReason::Stop)
        .await
        .unwrap();
    wait_for("the instance to release its extensions", async || {
        app.instances.loaded_extensions().is_empty()
    })
    .await;
    let after_stop = gc::gc_candidates(&app, later + DAY_MS, 0).await.unwrap();

    assert!(while_running.iter().all(|item| item.id != loaded_item));
    assert!(after_stop.iter().any(|item| item.id == loaded_item));
}

#[tokio::test]
async fn non_object_manifest_is_rejected() {
    let (_dir, app) = scratch_app().await;
    let tar = tar_of(&[("manifest.json", b"[]")]);

    let added = call(&app, "extension.add", json!({"file": upload(&tar)})).await;

    assert_eq!(invalid_field(added), "file");
}

fn write_dir(dir: &Path, bytes: usize) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("payload"), vec![b'x'; bytes]).unwrap();
}

#[tokio::test]
async fn gc_candidates_and_delete() {
    const OLD_BYTES: usize = 100;
    const LATE_BYTES: usize = 200;
    const VERSION_BYTES: usize = 50;
    let (dir, app) = scratch_app().await;
    let now = now_ms();
    let old_at = now - 31 * DAY_MS;
    let builds_dir = app.paths.builds_dir();
    let mut rows = Vec::new();
    for (version, bytes) in [
        ("1.0.0.1", OLD_BYTES),
        ("1.0.0.2", LATE_BYTES),
        ("1.0.0.3", 10),
    ] {
        let version_dir = builds_dir.join("cft").join(version);
        write_dir(&version_dir, bytes);
        let mut row = chromium_build(BuildSource::Cft, vec![Capability::UnpackedExtensions]);
        row.version = version.to_owned();
        row.path = version_dir.join("payload");
        row.installed_at = Some(old_at);
        rows.push(row);
    }
    let (old, late, pinned) = (rows[0].clone(), rows[1].clone(), rows[2].clone());
    for row in rows {
        insert_build(&app, row).await;
    }
    let _holder = pinned_profile(&app, pinned.id, "holder").await;

    let extension_id = ExtensionId::new();
    let mut versions = Vec::new();
    for (version, bytes) in [("1", VERSION_BYTES), ("2", 10)] {
        let version_dir = dir
            .path()
            .join("data/extensions")
            .join(extension_id.to_string())
            .join(version);
        write_dir(&version_dir, bytes);
        versions.push(ExtensionVersion {
            version: version.to_owned(),
            path: version_dir,
            size_bytes: bytes as u64,
            fetched_at: old_at,
            last_referenced_at: None,
        });
    }
    let row = Extension {
        id: extension_id,
        engine: Engine::Chromium,
        source: ExtensionSource::Local,
        source_ref: "sha".to_owned(),
        name: "Lib".to_owned(),
        versions: Vec::new(),
    };
    app.store
        .run(move |c| {
            extension::insert(c, &row)?;
            versions
                .iter()
                .try_for_each(|v| extension::add_version(c, &row.id, v))
        })
        .await
        .unwrap();

    let candidates = gc::gc_candidates(&app, now, 30).await.unwrap();
    // Pin one candidate between the calls.
    let _latecomer = pinned_profile(&app, late.id, "latecomer").await;
    let (deleted, freed) = gc::gc_delete(&app, &candidates).await.unwrap();

    let ids: Vec<&str> = candidates.iter().map(|i| i.id.as_str()).collect();
    assert!(ids.contains(&old.id.to_string().as_str()));
    assert!(ids.contains(&late.id.to_string().as_str()));
    assert!(ids.contains(&format!("{extension_id}@1").as_str()));
    assert!(!ids.contains(&pinned.id.to_string().as_str()));
    assert_eq!(candidates.len(), 3);
    // The pinned latecomer is skipped, so it is neither counted nor freed.
    assert_eq!(deleted, 2);
    assert_eq!(freed, (OLD_BYTES + VERSION_BYTES) as u64);
    assert!(!builds_dir.join("cft/1.0.0.1").exists());
    assert!(builds_dir.join("cft/1.0.0.2").exists());
    assert!(builds_dir.join("cft/1.0.0.3").exists());
    assert!(
        !dir.path()
            .join("data/extensions")
            .join(extension_id.to_string())
            .join("1")
            .exists()
    );
    assert!(
        dir.path()
            .join("data/extensions")
            .join(extension_id.to_string())
            .join("2")
            .exists()
    );
}
