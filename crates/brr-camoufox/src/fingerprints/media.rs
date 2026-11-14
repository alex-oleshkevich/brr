use serde_json::Value;

use super::{
    config::{ConfigExt, number},
    identity_seed, py_sum,
};
use crate::{coherence::Config, data::MEDIA_DEVICES, pycompat, pyrandom::PyRandom};

static EMPTY: Vec<Value> = Vec::new();

// Share of machines with no microphone at all, and with a built-in camera
fn p_no_mic(os_key: &str) -> f64 {
    match os_key {
        "win" => 0.08,
        "lin" => 0.2,
        _ => 0.0,
    }
}

fn p_builtin_cam(os_key: &str) -> f64 {
    match os_key {
        "win" => 0.78,
        "lin" => 0.45,
        _ => 0.0,
    }
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().unwrap_or(&EMPTY)
}

fn weighted_choice<'a>(rng: &mut PyRandom, items: &[&'a Value]) -> Option<&'a Value> {
    let weights: Vec<Value> = items
        .iter()
        .map(|item| item.get("w").cloned().unwrap_or(Value::from(1)))
        .collect();
    let mut x = rng.random() * py_sum(&weights);
    for (item, weight) in items.iter().zip(&weights) {
        x -= number(weight);
        if x < 0.0 {
            return Some(item);
        }
    }
    items.last().copied()
}

struct Devices {
    mics: Vec<(String, String)>,
    outs: Vec<(String, String)>,
    cams: Vec<(String, String)>,
    counter: u32,
}

impl Devices {
    fn group(&mut self) -> String {
        self.counter += 1;
        format!("hw-{}", self.counter)
    }

    fn add(&mut self, item: &Value, group: &str, with_mics: bool) {
        if with_mics {
            for mic in list(item, "mics").iter().filter_map(Value::as_str) {
                self.mics.push((mic.to_owned(), group.to_owned()));
            }
        }
        for out in list(item, "outs").iter().filter_map(Value::as_str) {
            self.outs.push((out.to_owned(), group.to_owned()));
        }
        if pycompat::truthy(&item["cam"]) {
            let camera_group = self.group();
            self.cams.push((pycompat::str(&item["cam"]), camera_group));
        }
    }
}

fn unzip(pairs: &[(String, String)]) -> (Value, Value) {
    (
        pairs.iter().map(|(label, _)| label.as_str()).collect(),
        pairs.iter().map(|(_, group)| group.as_str()).collect(),
    )
}

// One machine's media devices for `os_key` (win, mac or lin): the mediaDevices:* config keys,
// counts plus aligned label and group lists
pub fn draw_media_devices(os_key: &str, rng: &mut PyRandom) -> Vec<(String, Value)> {
    let catalogue = if pycompat::truthy(&MEDIA_DEVICES[os_key]) {
        &MEDIA_DEVICES[os_key]
    } else {
        &MEDIA_DEVICES["win"]
    };
    let mut devices = Devices {
        mics: Vec::new(),
        outs: Vec::new(),
        cams: Vec::new(),
        counter: 0,
    };

    // 1. the machine's own sound card (+ built-in camera on macOS models)
    let cards: Vec<&Value> = list(catalogue, "cards").iter().collect();
    let card = weighted_choice(rng, &cards);
    let no_mic = rng.random() < p_no_mic(os_key);
    let card_group = devices.group();
    if let Some(card) = card {
        devices.add(card, &card_group, !no_mic);
    }

    // 2. a built-in laptop camera (Windows/Linux); rare on a mic-less tower
    let p_cam = p_builtin_cam(os_key);
    if rng.random() < if no_mic { p_cam * 0.3 } else { p_cam } {
        let builtin: Vec<&Value> = list(catalogue, "cameras")
            .iter()
            .filter(|camera| !pycompat::truthy(&camera["mic"]))
            .collect();
        if !builtin.is_empty()
            && let Some(camera) = weighted_choice(rng, &builtin)
        {
            let group = devices.group();
            devices.cams.push((pycompat::str(&camera["cam"]), group));
        }
    }

    // 3. a headset / USB microphone
    if rng.random() < catalogue["p_headset"].as_f64().unwrap_or(0.0) {
        let headsets: Vec<&Value> = list(catalogue, "headsets").iter().collect();
        if let Some(headset) = weighted_choice(rng, &headsets) {
            let group = devices.group();
            devices.add(headset, &group, true);
        }
    }

    // 4. display audio (HDMI/DP)
    if rng.random() < catalogue["p_display"].as_f64().unwrap_or(0.0) {
        let displays: Vec<&Value> = list(catalogue, "displays").iter().collect();
        if let Some(display) = weighted_choice(rng, &displays) {
            let group = devices.group();
            devices.add(display, &group, true);
        }
    }

    // 5. an external webcam, usually with its own microphone
    if rng.random() < catalogue["p_extra_camera"].as_f64().unwrap_or(0.0) {
        let cameras = list(catalogue, "cameras");
        let with_mic: Vec<&Value> = cameras
            .iter()
            .filter(|camera| pycompat::truthy(&camera["mic"]))
            .collect();
        let external = if with_mic.is_empty() {
            cameras.iter().collect()
        } else {
            with_mic
        };
        if let Some(camera) = weighted_choice(rng, &external) {
            let group = devices.group();
            devices
                .cams
                .push((pycompat::str(&camera["cam"]), group.clone()));
            if pycompat::truthy(&camera["mic"]) {
                devices.mics.push((pycompat::str(&camera["mic"]), group));
            }
        }
    }

    // 6. PulseAudio exposes a monitor source per output as a capture device
    if pycompat::truthy(&catalogue["monitor_sources"]) {
        let monitors: Vec<(String, String)> = devices
            .outs
            .iter()
            .map(|(label, group)| (format!("Monitor of {label}"), group.clone()))
            .collect();
        devices.mics.extend(monitors);
    }

    let (mic_labels, mic_groups) = unzip(&devices.mics);
    let (cam_labels, cam_groups) = unzip(&devices.cams);
    let (out_labels, out_groups) = unzip(&devices.outs);
    vec![
        ("mediaDevices:enabled".to_owned(), Value::Bool(true)),
        (
            "mediaDevices:micros".to_owned(),
            Value::from(devices.mics.len()),
        ),
        (
            "mediaDevices:webcams".to_owned(),
            Value::from(devices.cams.len()),
        ),
        (
            "mediaDevices:speakers".to_owned(),
            Value::from(devices.outs.len()),
        ),
        ("mediaDevices:microphoneLabels".to_owned(), mic_labels),
        ("mediaDevices:microphoneGroups".to_owned(), mic_groups),
        ("mediaDevices:webcamLabels".to_owned(), cam_labels),
        ("mediaDevices:webcamGroups".to_owned(), cam_groups),
        ("mediaDevices:speakerLabels".to_owned(), out_labels),
        ("mediaDevices:speakerGroups".to_owned(), out_groups),
    ]
}

// Give the identity a plausible set of media devices, drawn from the common desktop population
// for its OS and seeded by the identity. Nothing is drawn when the config already has any
// mediaDevices: key.
pub fn set_media_devices_defaults(config: &mut Config, salt: u64) {
    if config.keys().any(|key| key.starts_with("mediaDevices:")) {
        return;
    }

    let platform = config
        .get("navigator.platform")
        .map_or_else(String::new, pycompat::str);
    let os_key = if platform.starts_with("Win") {
        "win"
    } else if platform.starts_with("Mac") {
        "mac"
    } else {
        "lin"
    };
    let mut rng = PyRandom::seed(u64::from(identity_seed(config, salt)));
    for (key, value) in draw_media_devices(os_key, &mut rng) {
        config.set(&key, value);
    }
}
