use std::{ffi::OsString, fs, net::IpAddr, sync::Arc};

use brr_core::{
    Asset, AssetId, AssetKind, DisplayEnv, InstanceId, Paths, Profile, ProfileId, ProfileKind,
    ProfileSettings, Proxy, ProxyChoice, ProxyId, ProxyScheme, now_ms, settings,
};
use brr_store::{asset, proxy};
use tempfile::TempDir;

use super::{apply_geo, launch_geo, seed};
use crate::{
    app::{App, Boot},
    services::proxies,
};

const RECORD_BITS: u16 = 24;
const RECORD_BYTES: usize = 3;
const DATA_SEPARATOR_BYTES: usize = 16;
const METADATA_MARKER: &[u8] = b"\xab\xcd\xefMaxMind.com";
const COVERED: [u8; 3] = [1, 2, 3];
const EXIT_IP: &str = "1.2.3.4";

// The MaxMind DB encoding of the few types the fixture uses
fn control(kind: u8, len: usize) -> Vec<u8> {
    if kind <= 7 {
        vec![(kind << 5) | len as u8]
    } else {
        vec![len as u8, kind - 7]
    }
}

fn string(text: &str) -> Vec<u8> {
    [control(2, text.len()), text.as_bytes().to_vec()].concat()
}

fn uint(kind: u8, value: u64, width: usize) -> Vec<u8> {
    [
        control(kind, width),
        value.to_be_bytes()[8 - width..].to_vec(),
    ]
    .concat()
}

fn map(entries: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
    let mut out = control(7, entries.len());
    for (key, value) in entries {
        out.extend(string(key));
        out.extend(value);
    }

    out
}

fn double(value: f64) -> Vec<u8> {
    [control(3, 8), value.to_be_bytes().to_vec()].concat()
}

// An IPv4 database whose only network, 1.2.3.0/24, is in Berlin
fn berlin_database() -> Vec<u8> {
    let bits: Vec<usize> = COVERED
        .iter()
        .flat_map(|byte| {
            (0..8)
                .rev()
                .map(move |shift| usize::from(byte >> shift & 1))
        })
        .collect();
    let node_count = bits.len();
    let data = map(vec![
        ("country", map(vec![("iso_code", string("DE"))])),
        (
            "location",
            map(vec![
                ("latitude", double(52.5)),
                ("longitude", double(13.4)),
                ("time_zone", string("Europe/Berlin")),
            ]),
        ),
    ]);

    let mut tree = Vec::new();
    for (depth, bit) in bits.iter().enumerate() {
        let mut records = [node_count; 2];
        records[*bit] = if depth + 1 < node_count {
            depth + 1
        } else {
            node_count + DATA_SEPARATOR_BYTES
        };
        for value in records {
            tree.extend(&(value as u32).to_be_bytes()[4 - RECORD_BYTES..]);
        }
    }

    let metadata = map(vec![
        ("binary_format_major_version", uint(5, 2, 2)),
        ("binary_format_minor_version", uint(5, 0, 2)),
        ("build_epoch", uint(9, 0, 4)),
        ("database_type", string("Test")),
        ("description", map(vec![("en", string("Test"))])),
        ("ip_version", uint(5, 4, 2)),
        ("languages", [control(11, 1), string("en")].concat()),
        ("node_count", uint(6, node_count as u64, 4)),
        ("record_size", uint(5, u64::from(RECORD_BITS), 2)),
    ]);

    [
        tree,
        vec![0; DATA_SEPARATOR_BYTES],
        data,
        METADATA_MARKER.to_vec(),
        metadata,
    ]
    .concat()
}

async fn app(dir: &TempDir) -> Arc<App> {
    let paths = Paths {
        config_dir: dir.path().join("config"),
        data_dir: dir.path().join("data"),
        state_dir: dir.path().join("state"),
        runtime_dir: dir.path().join("run").join("brr"),
    };
    fs::create_dir_all(&paths.config_dir).unwrap();
    fs::write(paths.config_file(), "[vault]\nkey_source = \"file\"\n").unwrap();
    let (settings, sources) = settings::load(&paths.config_file(), &[]).unwrap();

    App::start(Boot {
        paths,
        settings,
        sources,
        env: Vec::new(),
        display_env: DisplayEnv::default(),
        path_env: OsString::new(),
        cft_base_url: String::new(),
        version: "0.0.0-test",
    })
    .await
    .unwrap()
}

fn profile(kind: ProfileKind, settings: ProfileSettings) -> Profile {
    Profile {
        id: ProfileId::new(),
        name: "geo".to_owned(),
        kind,
        template_id: None,
        settings,
        pinned: false,
        notes: String::new(),
        tags: Vec::new(),
        data_dir: None,
        disk_quota_bytes: None,
        backup_schedule: None,
        backup_keep: 0,
        created_at: 0,
        updated_at: 0,
        capabilities: Vec::new(),
        login_status: None,
        builtin: false,
    }
}

#[tokio::test]
async fn geo_applied() {
    let dir = TempDir::new().unwrap();
    let app = app(&dir).await;
    let database = dir.path().join("geoip.mmdb");
    fs::write(&database, berlin_database()).unwrap();
    let proxy_id = ProxyId::new();
    let row = Proxy {
        id: proxy_id,
        scheme: ProxyScheme::Http,
        host: "127.0.0.1".to_owned(),
        port: 9,
        username: None,
        password: None,
        healthy: Some(true),
        exit_ip: Some(EXIT_IP.to_owned()),
        country: None,
        latency_ms: Some(1),
        last_error: None,
        checked_at: Some(now_ms()),
    };
    app.store
        .run(move |c| {
            proxy::insert(c, &row, None)?;
            asset::put(
                c,
                &Asset {
                    id: AssetId::new(),
                    kind: AssetKind::GeoipDb,
                    version: "test".to_owned(),
                    path: database,
                    updated_at: now_ms(),
                },
            )
        })
        .await
        .unwrap();
    let located = profile(
        ProfileKind::Ephemeral,
        ProfileSettings {
            geoip: true,
            proxy: Some(ProxyChoice::ProxyId(proxy_id)),
            ..ProfileSettings::default()
        },
    );
    let instance_id = InstanceId::new();
    proxies::for_launch(&app, &instance_id, &located)
        .await
        .unwrap()
        .unwrap();

    let geo = launch_geo(&app, &instance_id, &located)
        .await
        .unwrap()
        .unwrap();
    let mut settings = located.settings.clone();
    apply_geo(&mut settings, &geo);

    assert_eq!(geo.ip, EXIT_IP.parse::<IpAddr>().unwrap());
    assert_eq!(settings.timezone.as_deref(), Some("Europe/Berlin"));
    assert_eq!(settings.locale.as_deref(), Some("de-DE"));
    let position = settings.geolocation.unwrap();
    assert_eq!((position.latitude, position.longitude), (52.5, 13.4));

    let unset = profile(ProfileKind::Ephemeral, ProfileSettings::default());
    assert_eq!(launch_geo(&app, &instance_id, &unset).await.unwrap(), None);
}

#[test]
fn seed_is_stable_for_persistent_profiles_only() {
    let persistent = profile(ProfileKind::Persistent, ProfileSettings::default());
    let ephemeral = profile(ProfileKind::Ephemeral, ProfileSettings::default());

    assert!(seed(&persistent).is_some());
    assert_eq!(seed(&persistent), seed(&persistent));
    assert_eq!(seed(&ephemeral), None);
}
