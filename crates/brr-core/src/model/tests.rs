use std::{cmp::Ordering, collections::BTreeMap};

use serde_json::json;
use test_case::test_case;

use super::{
    Fingerprint, Geolocation, PoolId, ProfileSettings, ProxyChoice, ProxyId, Size, WebRtcPolicy,
    compare_versions,
};
use crate::Error;

fn invalid_field(settings: &ProfileSettings) -> Option<String> {
    match settings.validate() {
        Err(Error::InvalidArgument { field, .. }) => Some(field),
        _ => None,
    }
}

#[test]
fn m1_template_settings_still_decode() {
    let settings: ProfileSettings =
        serde_json::from_value(json!({"build": {"channel": "cft:stable"}, "display": "headless"}))
            .unwrap();

    assert_eq!(settings.webrtc, WebRtcPolicy::Default);
    assert!(settings.prefs.is_empty());
    assert_eq!(settings.proxy, None);
    assert_eq!(settings, ProfileSettings::default());
    assert_eq!(settings.validate(), Ok(()));
}

#[test]
fn settings_round_trip_with_new_fields() {
    let pool_id = PoolId::new();
    let settings = ProfileSettings {
        proxy: Some(ProxyChoice::PoolId(pool_id)),
        webrtc: WebRtcPolicy::DisableNonProxiedUdp,
        prefs: BTreeMap::from([("intl.accept_languages".to_string(), json!("de"))]),
        fingerprint: Some(Fingerprint::default()),
        ..ProfileSettings::default()
    };
    let value = serde_json::to_value(&settings).unwrap();

    assert_eq!(value["proxy"], json!({"pool_id": pool_id.to_string()}));
    assert_eq!(value["webrtc"], "disable_non_proxied_udp");
    assert_eq!(
        serde_json::from_value::<ProfileSettings>(value).unwrap(),
        settings
    );
}

#[test]
fn proxy_choice_encodes_like_build_choice() {
    let proxy_id = ProxyId::new();
    let value = json!({"proxy_id": proxy_id.to_string()});

    assert_eq!(
        serde_json::from_value::<ProxyChoice>(value.clone()).unwrap(),
        ProxyChoice::ProxyId(proxy_id)
    );
    assert_eq!(
        serde_json::to_value(ProxyChoice::ProxyId(proxy_id)).unwrap(),
        value
    );
}

#[test_case(91.0, 0.0, None)]
#[test_case(-91.0, 0.0, None)]
#[test_case(0.0, 181.0, None)]
#[test_case(0.0, -181.0, None)]
#[test_case(0.0, 0.0, Some(-1.0))]
#[test_case(f64::NAN, 0.0, None)]
fn validate_rejects_bad_geolocation(latitude: f64, longitude: f64, accuracy: Option<f64>) {
    let settings = ProfileSettings {
        geolocation: Some(Geolocation {
            latitude,
            longitude,
            accuracy,
        }),
        ..ProfileSettings::default()
    };

    assert_eq!(
        invalid_field(&settings).as_deref(),
        Some("settings.geolocation")
    );
}

#[test]
fn validate_rejects_geolocation_from_json() {
    let settings: ProfileSettings = serde_json::from_value(json!({
        "build": {"channel": "cft:stable"}, "display": "headless",
        "geolocation": {"latitude": 91, "longitude": 0}
    }))
    .unwrap();

    assert_eq!(
        invalid_field(&settings).as_deref(),
        Some("settings.geolocation")
    );
}

#[test]
fn validate_accepts_edge_geolocation() {
    let settings = ProfileSettings {
        geolocation: Some(Geolocation {
            latitude: -90.0,
            longitude: 180.0,
            accuracy: Some(0.0),
        }),
        ..ProfileSettings::default()
    };

    assert_eq!(settings.validate(), Ok(()));
}

#[test_case("")]
#[test_case(&"k".repeat(201))]
fn validate_rejects_bad_pref_keys(key: &str) {
    let settings = ProfileSettings {
        prefs: BTreeMap::from([(key.to_string(), json!(1))]),
        ..ProfileSettings::default()
    };

    assert_eq!(invalid_field(&settings).as_deref(), Some("settings.prefs"));
}

#[test]
fn validate_accepts_longest_pref_key() {
    let settings = ProfileSettings {
        prefs: BTreeMap::from([("k".repeat(200), json!(1))]),
        ..ProfileSettings::default()
    };

    assert_eq!(settings.validate(), Ok(()));
}

#[test_case(0, 100)]
#[test_case(100, 0)]
#[test_case(10_001, 100)]
#[test_case(100, 10_001)]
fn validate_rejects_bad_screen_sizes(width: u32, height: u32) {
    let size = Size { width, height };
    let viewport = ProfileSettings {
        viewport: Some(size.clone()),
        ..ProfileSettings::default()
    };
    let fingerprint = ProfileSettings {
        fingerprint: Some(Fingerprint {
            screen: Some(size),
            ..Fingerprint::default()
        }),
        ..ProfileSettings::default()
    };

    assert_eq!(
        invalid_field(&viewport).as_deref(),
        Some("settings.viewport")
    );
    assert_eq!(
        invalid_field(&fingerprint).as_deref(),
        Some("settings.fingerprint.screen")
    );
}

#[test]
fn validate_accepts_screen_bounds() {
    for side in [1, 10_000] {
        let settings = ProfileSettings {
            viewport: Some(Size {
                width: side,
                height: side,
            }),
            ..ProfileSettings::default()
        };

        assert_eq!(settings.validate(), Ok(()));
    }
}

#[test_case("152.0.10", "152.0.9", Ordering::Greater)]
#[test_case("1.2", "1.2.0", Ordering::Equal)]
#[test_case("1.2", "1.10", Ordering::Less)]
#[test_case("1.x", "1.0", Ordering::Equal)]
#[test_case("2", "1.9.9", Ordering::Greater)]
fn compare_versions_orders(a: &str, b: &str, expected: Ordering) {
    assert_eq!(compare_versions(a, b), expected);
    assert_eq!(compare_versions(b, a), expected.reverse());
}

#[test_case("tab_min", "idle.tab_min")]
#[test_case("instance_min", "idle.instance_min")]
fn validate_rejects_zero_idle_minutes(key: &str, field: &str) {
    let mut settings = ProfileSettings::default();
    match key {
        "tab_min" => settings.idle.tab_min = Some(0),
        _ => settings.idle.instance_min = Some(0),
    }

    assert_eq!(invalid_field(&settings).as_deref(), Some(field));
}
