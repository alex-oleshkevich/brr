use std::path::PathBuf;

use brr_core::{
    Build, BuildChoice, BuildId, BuildSource, BuildState, Capability, Channel, Engine, Error,
    now_ms,
};
use brr_store::{Connection, Store, build};

use super::resolve_build;

fn build_row(
    source: BuildSource,
    state: BuildState,
    channel: Option<&str>,
    installed_at: i64,
) -> Build {
    Build {
        id: BuildId::new(),
        source,
        engine: Engine::Chromium,
        name: "chrome".to_string(),
        version: format!("{}.{installed_at}", now_ms()),
        channel: channel.map(str::to_string),
        path: PathBuf::from("/nonexistent"),
        state,
        sha256: None,
        size_bytes: 0,
        capabilities: vec![Capability::Pdf],
        installed_at: Some(installed_at),
        last_referenced_at: None,
    }
}

fn channel(source: BuildSource, name: &str) -> BuildChoice {
    BuildChoice::Channel(Channel {
        source,
        name: name.to_string(),
    })
}

async fn with_builds(rows: Vec<Build>) -> Store {
    let store = Store::open_in_memory().unwrap();
    store
        .run(move |c| rows.iter().try_for_each(|b| build::insert(c, b)))
        .await
        .unwrap();

    store
}

async fn resolve(store: &Store, choice: BuildChoice) -> Result<Build, Error> {
    store
        .run(move |c: &mut Connection| resolve_build(c, &choice))
        .await
}

fn reason(result: Result<Build, Error>) -> String {
    match result {
        Err(Error::BuildMissing { reason }) => reason,
        other => panic!("expected BuildMissing, got {other:?}"),
    }
}

#[tokio::test]
async fn resolve_build_prefers_newest_cft_then_system() {
    let old = build_row(BuildSource::Cft, BuildState::Installed, Some("stable"), 1);
    let new = build_row(BuildSource::Cft, BuildState::Installed, Some("stable"), 2);
    let beta = build_row(BuildSource::Cft, BuildState::Installed, Some("beta"), 3);
    let system = build_row(BuildSource::System, BuildState::Registered, None, 4);
    let store = with_builds(vec![old, new.clone(), beta, system.clone()]).await;

    let stable = resolve(&store, channel(BuildSource::Cft, "stable"))
        .await
        .unwrap();
    let canary = resolve(&store, channel(BuildSource::Cft, "canary"))
        .await
        .unwrap();

    assert_eq!(stable.id, new.id);
    assert_eq!(canary.id, system.id);
}

#[tokio::test]
async fn resolve_build_missing() {
    let empty = with_builds(Vec::new()).await;
    let installing = build_row(BuildSource::Cft, BuildState::Installing, Some("stable"), 1);
    let busy = with_builds(vec![installing.clone()]).await;

    let none = resolve(&empty, channel(BuildSource::Cft, "stable")).await;
    let other_source = resolve(&empty, channel(BuildSource::Camoufox, "latest-compatible")).await;
    let not_ready = resolve(&busy, BuildChoice::BuildId(installing.id)).await;
    let unknown = resolve(&busy, BuildChoice::BuildId(BuildId::new())).await;

    assert_eq!(
        reason(none),
        "no cft stable build installed; run brr build install cft --channel stable"
    );
    assert!(reason(other_source).starts_with("no camoufox latest-compatible build installed"));
    reason(not_ready);
    reason(unknown);
}

#[tokio::test]
async fn resolve_build_by_id_accepts_registered() {
    let system = build_row(BuildSource::System, BuildState::Registered, None, 1);
    let store = with_builds(vec![system.clone()]).await;

    let found = resolve(&store, BuildChoice::BuildId(system.id))
        .await
        .unwrap();

    assert_eq!(found.id, system.id);
}

#[tokio::test]
async fn resolve_build_picks_the_newest_version_of_any_source_without_system_fallback() {
    let old = build_row(
        BuildSource::Ungoogled,
        BuildState::Installed,
        Some("stable"),
        1,
    );
    let new = build_row(
        BuildSource::Ungoogled,
        BuildState::Installed,
        Some("stable"),
        2,
    );
    let system = build_row(BuildSource::System, BuildState::Registered, None, 3);
    let store = with_builds(vec![new.clone(), old, system]).await;

    let found = resolve(&store, channel(BuildSource::Ungoogled, "stable"))
        .await
        .unwrap();
    let missing = resolve(&store, channel(BuildSource::Brave, "stable")).await;

    assert_eq!(found.id, new.id);
    assert!(reason(missing).starts_with("no brave stable build installed"));
}
