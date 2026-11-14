use std::path::PathBuf;

use brr_core::{
    AdmissionMode, Asset, AssetId, AssetKind, Build, BuildChoice, BuildId, BuildSource, BuildState,
    Channel, ConflictReason, Display, Engine, Error, Extension, ExtensionId, ExtensionSource,
    ExtensionVersion, Instance, InstanceId, InstanceKind, InstanceState, Page, Pool, PoolId,
    PoolMode, Profile, ProfileExtension, ProfileId, ProfileKind, ProfileSettings, Proxy,
    ProxyChoice, ProxyId, ProxyScheme,
};
use brr_store::{
    Connection, Store, asset, build, extension, instance, pool, profile, profile_extension, proxy,
};
use tempfile::TempDir;

const NOW: i64 = 1_700_000_000_000;
const SEALED: &[u8] = b"sealed-bytes";

#[allow(clippy::unwrap_used)]
fn open() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("brr.db")).unwrap();

    (dir, store)
}

fn proxy_at(host: &str, username: Option<&str>) -> Proxy {
    Proxy {
        id: ProxyId::new(),
        scheme: ProxyScheme::Http,
        host: host.to_string(),
        port: 1,
        username: username.map(str::to_string),
        password: None,
        healthy: None,
        exit_ip: None,
        country: None,
        latency_ms: None,
        last_error: None,
        checked_at: None,
    }
}

fn pool_of(name: &str, proxy_ids: Vec<ProxyId>) -> Pool {
    Pool {
        id: PoolId::new(),
        name: name.to_string(),
        mode: PoolMode::Sticky,
        proxy_ids,
    }
}

fn profile_named(name: &str, settings: ProfileSettings) -> Profile {
    Profile {
        id: ProfileId::new(),
        name: name.to_string(),
        kind: ProfileKind::Persistent,
        template_id: None,
        settings,
        pinned: false,
        notes: String::new(),
        tags: Vec::new(),
        data_dir: None,
        disk_quota_bytes: None,
        backup_schedule: None,
        backup_keep: 7,
        created_at: NOW,
        updated_at: NOW,
        capabilities: Vec::new(),
        login_status: None,
        builtin: false,
    }
}

fn extension_from(source_ref: &str) -> Extension {
    Extension {
        id: ExtensionId::new(),
        engine: Engine::Chromium,
        source: ExtensionSource::Cws,
        source_ref: source_ref.to_string(),
        name: source_ref.to_string(),
        versions: Vec::new(),
    }
}

fn version(version: &str) -> ExtensionVersion {
    ExtensionVersion {
        version: version.to_string(),
        path: PathBuf::from(format!("/ext/{version}")),
        size_bytes: 10,
        fetched_at: NOW,
        last_referenced_at: None,
    }
}

fn build_named(version: &str) -> Build {
    Build {
        id: BuildId::new(),
        source: BuildSource::Cft,
        engine: Engine::Chromium,
        name: "Chrome for Testing".to_string(),
        version: version.to_string(),
        channel: None,
        path: PathBuf::from(format!("/builds/{version}/chrome")),
        state: BuildState::Installed,
        sha256: None,
        size_bytes: 1,
        capabilities: Vec::new(),
        installed_at: Some(NOW),
        last_referenced_at: None,
    }
}

fn is_name_taken(result: Result<(), Error>) -> bool {
    matches!(
        result,
        Err(Error::Conflict {
            reason: ConflictReason::NameTaken,
            detail: None
        })
    )
}

#[test]
#[allow(clippy::unwrap_used)]
fn migrates_0002() {
    let dir = TempDir::new().unwrap();
    let db_file = dir.path().join("brr.db");
    let extension_id = ExtensionId::new();

    {
        let c = Connection::open(&db_file).unwrap();
        c.execute_batch(include_str!("../src/migrations/0001_init.sql"))
            .unwrap();
        c.pragma_update(None, "user_version", 1).unwrap();
        c.execute(
            "INSERT INTO extension (id, engine, source, source_ref, name) VALUES (?1, 'chromium', 'cws', 'abc', 'abc')",
            [extension_id.to_string()],
        )
        .unwrap();
        c.execute(
            "INSERT INTO extension_version (extension_id, version, path, fetched_at) VALUES (?1, '1.0.0', '/e', 5)",
            [extension_id.to_string()],
        )
        .unwrap();
    }

    for _ in 0..2 {
        let store = Store::open(&db_file).unwrap();
        let loaded = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(store.run(move |c| extension::get(c, &extension_id)))
            .unwrap();

        assert_eq!(loaded.versions.len(), 1);
        assert_eq!(loaded.versions[0].size_bytes, 0);
        assert_eq!(loaded.versions[0].fetched_at, 5);
        assert_eq!(loaded.versions[0].last_referenced_at, None);
    }
}

#[tokio::test]
async fn proxy_repo() {
    let (_dir, store) = open();

    store
        .run(|c| {
            let plain = proxy_at("h", None);
            proxy::insert(c, &plain, Some(SEALED))?;
            assert!(is_name_taken(proxy::insert(c, &proxy_at("h", None), None)));

            let named = proxy_at("h", Some("a"));
            proxy::insert(c, &named, None)?;
            assert!(is_name_taken(proxy::insert(
                c,
                &proxy_at("h", Some("a")),
                None
            )));

            let (listed, next) = proxy::list(c, &Page::new(None, None))?;
            assert_eq!(listed.len(), 2);
            assert_eq!(next, None);
            assert!(listed.iter().all(|p| p.password.is_none()));
            assert_eq!(proxy::all(c)?, listed);
            assert_eq!(proxy::get(c, &plain.id)?.password, None);
            assert_eq!(proxy::password_enc(c, &plain.id)?.as_deref(), Some(SEALED));
            assert_eq!(proxy::password_enc(c, &named.id)?, None);

            let (first, next) = proxy::list(c, &Page::new(Some(1), None))?;
            assert_eq!(first.len(), 1);
            let (second, _) = proxy::list(c, &Page::new(Some(1), next))?;
            assert_ne!(first[0].id, second[0].id);

            proxy::set_password(c, &named.id, Some(b"x"))?;
            assert_eq!(
                proxy::password_enc(c, &named.id)?.as_deref(),
                Some(&b"x"[..])
            );
            proxy::set_password(c, &named.id, None)?;
            assert_eq!(proxy::password_enc(c, &named.id)?, None);

            let health = proxy::Health {
                healthy: true,
                exit_ip: Some("1.2.3.4".to_string()),
                country: Some("DE".to_string()),
                latency_ms: Some(80),
                last_error: None,
                checked_at: NOW,
            };
            proxy::set_health(c, &plain.id, &health)?;
            let checked = proxy::get(c, &plain.id)?;
            assert_eq!(checked.healthy, Some(true));
            assert_eq!(checked.exit_ip.as_deref(), Some("1.2.3.4"));
            assert_eq!(checked.latency_ms, Some(80));
            assert_eq!(checked.checked_at, Some(NOW));

            let moved = Proxy {
                host: "other".to_string(),
                port: 2,
                ..checked
            };
            proxy::update(c, &moved)?;
            let updated = proxy::get(c, &plain.id)?;
            assert_eq!((updated.host.as_str(), updated.port), ("other", 2));
            assert_eq!(updated.healthy, Some(true));
            assert_eq!(proxy::password_enc(c, &plain.id)?.as_deref(), Some(SEALED));

            proxy::delete(c, &plain.id)?;
            assert!(matches!(
                proxy::get(c, &plain.id),
                Err(Error::NotFound { .. })
            ));

            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn pool_repo() {
    let (_dir, store) = open();

    store
        .run(|c| {
            let (p1, p2) = (proxy_at("one", None), proxy_at("two", None));
            let profile = profile_named("alpha", ProfileSettings::default());
            proxy::insert(c, &p1, None)?;
            proxy::insert(c, &p2, None)?;
            profile::insert(c, &profile)?;

            let mut p = pool_of("rotating", vec![p1.id, p2.id]);
            pool::insert(c, &p)?;
            assert!(is_name_taken(pool::insert(c, &pool_of("rotating", vec![]))));
            assert_eq!(pool::get(c, &p.id)?.proxy_ids, vec![p1.id, p2.id]);
            assert_eq!(pool::names_with_proxy(c, &p1.id)?, vec!["rotating"]);

            pool::set_sticky(c, &p.id, &profile.id, &p1.id)?;
            p.proxy_ids = vec![p2.id];
            pool::update(c, &p)?;
            assert_eq!(pool::sticky(c, &p.id, &profile.id)?, None);
            pool::set_sticky(c, &p.id, &profile.id, &p2.id)?;
            pool::set_sticky(c, &p.id, &profile.id, &p2.id)?;

            assert_eq!(pool::get(c, &p.id)?, p);
            assert_eq!(pool::find(c, "rotating")?, p);
            assert_eq!(pool::find(c, &p.id.to_string())?, p);
            assert_eq!(pool::sticky(c, &p.id, &profile.id)?, Some(p2.id));
            assert!(pool::names_with_proxy(c, &p1.id)?.is_empty());

            let (listed, _) = pool::list(c, &Page::new(None, None))?;
            assert_eq!(listed, vec![p.clone()]);

            proxy::delete(c, &p2.id)?;
            assert!(pool::get(c, &p.id)?.proxy_ids.is_empty());
            assert_eq!(pool::sticky(c, &p.id, &profile.id)?, None);

            pool::delete(c, &p.id)?;
            assert!(matches!(pool::get(c, &p.id), Err(Error::NotFound { .. })));

            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn references() {
    let (_dir, store) = open();

    store
        .run(|c| {
            let (proxy_id, pool_id) = (ProxyId::new(), PoolId::new());
            let pinned = build_named("1.0.0");
            build::insert(c, &pinned)?;

            let channel = |name: &str| Channel {
                source: BuildSource::Cft,
                name: name.to_string(),
            };
            let with = |build: BuildChoice, proxy: Option<ProxyChoice>| ProfileSettings {
                build,
                proxy,
                ..ProfileSettings::default()
            };

            let via_proxy = profile_named(
                "via-proxy",
                with(
                    BuildChoice::BuildId(pinned.id),
                    Some(ProxyChoice::ProxyId(proxy_id)),
                ),
            );
            let via_pool = profile_named(
                "via-pool",
                with(
                    BuildChoice::Channel(channel("stable")),
                    Some(ProxyChoice::PoolId(pool_id)),
                ),
            );
            let plain = profile_named("plain", with(BuildChoice::Channel(channel("stable")), None));
            let beta = profile_named("beta", with(BuildChoice::Channel(channel("beta")), None));
            for p in [&via_proxy, &via_pool, &plain, &beta] {
                profile::insert(c, p)?;
            }

            assert_eq!(
                profile::using_proxy(c, &proxy_id)?,
                vec![(via_proxy.id, "via-proxy".to_string())]
            );
            assert_eq!(
                profile::using_pool(c, &pool_id)?,
                vec![(via_pool.id, "via-pool".to_string())]
            );
            assert_eq!(
                profile::pinning_build(c, &pinned.id)?,
                vec![(via_proxy.id, "via-proxy".to_string())]
            );
            assert_eq!(
                profile::followed_channels(c)?,
                vec![channel("beta"), channel("stable")]
            );

            c.execute(
                "UPDATE profile SET channel = 'garbage' WHERE id = ?1",
                [beta.id.to_string()],
            )
            .map_err(|e| Error::Internal {
                message: e.to_string(),
            })?;
            assert_eq!(profile::followed_channels(c)?, vec![channel("stable")]);

            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn build_references() {
    let (_dir, store) = open();

    store
        .run(|c| {
            let used = build_named("1.0.0");
            let idle = build_named("2.0.0");
            build::insert(c, &used)?;
            build::insert(c, &idle)?;

            let running = Instance {
                build_id: Some(used.id),
                ..instance_in(InstanceState::Running)
            };
            let stopped = Instance {
                build_id: Some(idle.id),
                ..instance_in(InstanceState::Stopped)
            };
            instance::insert(c, &running)?;
            instance::insert(c, &stopped)?;

            assert_eq!(instance::using_build(c, &used.id)?, vec![running.id]);
            assert!(instance::using_build(c, &idle.id)?.is_empty());

            build::touch_referenced(c, &[used.id], NOW)?;
            assert_eq!(build::get(c, &used.id)?.last_referenced_at, Some(NOW));
            assert_eq!(build::get(c, &idle.id)?.last_referenced_at, None);

            build::delete(c, &idle.id)?;
            assert!(matches!(
                build::get(c, &idle.id),
                Err(Error::NotFound { .. })
            ));

            Ok(())
        })
        .await
        .unwrap();
}

fn instance_in(state: InstanceState) -> Instance {
    Instance {
        id: InstanceId::new(),
        kind: InstanceKind::Managed,
        profile_id: None,
        build_id: None,
        state,
        display: Display::Headless,
        priority: 5,
        admission: AdmissionMode::Queue,
        pid: None,
        log_path: None,
        cdp_url: None,
        data_dir: None,
        ephemeral: false,
        promote_to_name: None,
        restart_count: 0,
        last_activity_at: NOW,
        queued_at: NOW,
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

#[tokio::test]
async fn extension_repo() {
    let (_dir, store) = open();

    store
        .run(|c| {
            let e = extension_from("abc");
            extension::insert(c, &e)?;
            assert!(is_name_taken(extension::insert(c, &extension_from("abc"))));

            for v in ["1.9.0", "1.10.0"] {
                extension::add_version(c, &e.id, &version(v))?;
            }
            let loaded = extension::get(c, &e.id)?;
            let versions: Vec<_> = loaded.versions.iter().map(|v| v.version.as_str()).collect();
            assert_eq!(versions, ["1.10.0", "1.9.0"]);

            extension::add_version(
                c,
                &e.id,
                &ExtensionVersion {
                    size_bytes: 99,
                    ..version("1.9.0")
                },
            )?;
            extension::touch_versions(c, &[(e.id, "1.9.0".to_string())], NOW)?;
            let loaded = extension::get(c, &e.id)?;
            assert_eq!(loaded.versions[1].size_bytes, 99);
            assert_eq!(loaded.versions[1].last_referenced_at, Some(NOW));
            assert_eq!(loaded.versions[0].last_referenced_at, None);

            extension::set_name(c, &e.id, "renamed")?;
            let found = extension::find_by_source(c, ExtensionSource::Cws, "abc")?;
            assert_eq!(found.map(|f| f.name), Some("renamed".to_string()));
            assert!(extension::find_by_source(c, ExtensionSource::Amo, "abc")?.is_none());

            let other = extension_from("def");
            extension::insert(c, &other)?;
            let (first, next) = extension::list(c, &Page::new(Some(1), None))?;
            let (second, _) = extension::list(c, &Page::new(Some(1), next))?;
            assert_eq!((first[0].id, second[0].id), (other.id, e.id));
            assert_eq!(first[0].versions, Vec::new());
            assert_eq!(second[0].versions.len(), 2);

            let profile = profile_named("alpha", ProfileSettings::default());
            profile::insert(c, &profile)?;
            let link = ProfileExtension {
                extension_id: e.id,
                pinned_version: Some("1.9.0".to_string()),
                auto_update: false,
            };
            profile_extension::set(c, &profile.id, std::slice::from_ref(&link))?;
            assert_eq!(profile_extension::get(c, &profile.id)?, vec![link.clone()]);
            assert_eq!(
                profile_extension::profiles_using(c, &e.id)?,
                vec![(profile.id, "alpha".to_string())]
            );
            assert_eq!(profile_extension::all(c)?, vec![(profile.id, link.clone())]);

            extension::delete_version(c, &e.id, "1.9.0")?;
            assert_eq!(extension::get(c, &e.id)?.versions.len(), 1);

            extension::delete(c, &e.id)?;
            assert!(matches!(
                extension::get(c, &e.id),
                Err(Error::NotFound { .. })
            ));
            assert!(profile_extension::get(c, &profile.id)?.is_empty());
            let orphans: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM extension_version WHERE extension_id = ?1",
                    [e.id.to_string()],
                    |row| row.get(0),
                )
                .map_err(|e| Error::Internal {
                    message: e.to_string(),
                })?;
            assert_eq!(orphans, 0);

            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn asset_repo() {
    let (_dir, store) = open();

    store
        .run(|c| {
            let geoip = Asset {
                id: AssetId::new(),
                kind: AssetKind::GeoipDb,
                version: "1".to_string(),
                path: PathBuf::from("/a/geoip"),
                updated_at: NOW,
            };
            asset::put(c, &geoip)?;
            let newer = Asset {
                version: "2".to_string(),
                ..geoip.clone()
            };
            asset::put(c, &newer)?;

            assert_eq!(asset::by_kind(c, AssetKind::GeoipDb)?, vec![newer]);
            assert!(asset::by_kind(c, AssetKind::AdblockList)?.is_empty());

            asset::delete(c, &geoip.id)?;
            assert!(asset::by_kind(c, AssetKind::GeoipDb)?.is_empty());

            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn extension_list_rejects_malformed_cursor() {
    let store = Store::open_in_memory().unwrap();

    let result = store
        .run(|c| extension::list(c, &Page::new(Some(1), Some("no-separator".to_string()))))
        .await;

    assert!(matches!(
        result,
        Err(Error::InvalidArgument { field, .. }) if field == "cursor"
    ));
}
