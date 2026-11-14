use std::net::IpAddr;

use brr_core::Error;
use serde_json::{Map, Value};

use crate::pycompat;

const SETTERS: [(&str, &str); 7] = [
    ("navigatorPlatform", "setNavigatorPlatform"),
    ("navigatorOscpu", "setNavigatorOscpu"),
    ("navigatorUserAgent", "setNavigatorUserAgent"),
    ("navigatorAppVersion", "setNavigatorAppVersion"),
    ("hardwareConcurrency", "setNavigatorHardwareConcurrency"),
    ("webglVendor", "setWebGLVendor"),
    ("webglRenderer", "setWebGLRenderer"),
];

// json.dumps() of a scalar with its defaults: ASCII-escaped
fn dumps(value: &Value) -> String {
    match value {
        Value::String(text) => {
            let mut out = String::from("\"");
            for ch in text.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    '\u{8}' => out.push_str("\\b"),
                    '\u{c}' => out.push_str("\\f"),
                    ' '..='~' => out.push(ch),
                    _ => {
                        let mut units = [0u16; 2];
                        for unit in ch.encode_utf16(&mut units) {
                            out.push_str(&format!("\\u{unit:04x}"));
                        }
                    }
                }
            }
            out.push('"');
            out
        }
        Value::Null => "null".to_owned(),
        Value::Bool(flag) => flag.to_string(),
        other => pycompat::str(other),
    }
}

fn present(values: &Map<String, Value>, key: &str) -> bool {
    values.get(key).is_some_and(|value| !value.is_null())
}

fn truthy(values: &Map<String, Value>, key: &str) -> bool {
    values.get(key).is_some_and(pycompat::truthy)
}

fn setter(function: &str, argument: &str) -> String {
    format!("  if (typeof w.{function} === \"function\") w.{function}({argument});")
}

// The JavaScript init script that calls the per-context window.setXxx() functions. Those
// self-destruct after their first call, so they must run via an init script.
pub fn build_init_script(values: &Map<String, Value>) -> Result<String, Error> {
    let mut lines = vec!["(function(v) {".to_owned(), "  var w = window;".to_owned()];

    for (key, function) in SETTERS {
        if present(values, key) {
            lines.push(setter(function, &dumps(&values[key])));
        }
    }

    // Screen dimensions need width and height together
    if truthy(values, "screenWidth") && truthy(values, "screenHeight") {
        let dimensions = format!(
            "{}, {}",
            pycompat::str(&values["screenWidth"]),
            pycompat::str(&values["screenHeight"])
        );
        lines.push(setter("setScreenDimensions", &dimensions));
        if truthy(values, "screenColorDepth") {
            lines.push(setter(
                "setScreenColorDepth",
                &pycompat::str(&values["screenColorDepth"]),
            ));
        }
    }

    // Timezone: only with an explicit value; MaskConfig handles the rest
    if truthy(values, "timezone") {
        lines.push(setter("setTimezone", &dumps(&values["timezone"])));
    }

    if truthy(values, "webrtcIP") {
        let text = pycompat::str(&values["webrtcIP"]);
        let ip: IpAddr = text.parse().map_err(|_| Error::InvalidArgument {
            field: "webrtcIP".to_owned(),
            message: format!("Invalid IP address: {text}"),
        })?;
        let function = if ip.is_ipv4() {
            "setWebRTCIPv4"
        } else {
            "setWebRTCIPv6"
        };
        lines.push(setter(function, &dumps(&values["webrtcIP"])));
    } else {
        lines.push(setter("setWebRTCIPv4", "\"\""));
    }

    // Font list and speech voice names, comma-separated
    if let Some(fonts) = values.get("fontList").and_then(Value::as_array)
        && !fonts.is_empty()
    {
        let joined: Vec<String> = fonts.iter().map(pycompat::str).collect();
        lines.push(setter(
            "setFontList",
            &dumps(&Value::from(joined.join(","))),
        ));
    }
    if let Some(voices) = values.get("speechVoices").and_then(Value::as_array)
        && !voices.is_empty()
    {
        let names: Vec<String> = voices
            .iter()
            .map(|voice| match voice {
                Value::Object(_) => pycompat::str(&voice["name"]),
                other => pycompat::str(other),
            })
            .collect();
        lines.push(setter(
            "setSpeechVoices",
            &dumps(&Value::from(names.join(","))),
        ));
    }

    lines.push("})();".to_owned());
    Ok(lines.join("\n"))
}
