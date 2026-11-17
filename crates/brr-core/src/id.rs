use std::{
    fmt,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::error::Error;

const ULID_PATTERN: &str = "^[0-9A-HJKMNP-TV-Z]{26}$";
const CLIENT_ID_MAX_LEN: usize = 64;
const CLIENT_ID_EXTRA_CHARS: &str = "._:-";
const ID_FIELD: &str = "id";
const CLIENT_ID_FIELD: &str = "client_id";

macro_rules! ulid_ids {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
        pub struct $name(ulid::Ulid);

        impl $name {
            pub fn new() -> $name {
                $name(ulid::Ulid::generate())
            }
        }

        impl Default for $name {
            fn default() -> $name {
                $name::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(s: &str) -> Result<$name, Error> {
                ulid::Ulid::from_string(s).map($name).map_err(|_| Error::InvalidArgument {
                    field: ID_FIELD.to_string(),
                    message: format!("not a valid {}; expected a 26-character ULID", stringify!($name)),
                })
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<$name, D::Error> {
                String::deserialize(deserializer)?.parse().map_err(de::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({ "type": "string", "pattern": ULID_PATTERN })
            }
        }
    )+};
}

ulid_ids!(
    InstanceId,
    TabId,
    ProfileId,
    TemplateId,
    BuildId,
    LeaseId,
    ExtensionId,
    CredentialId,
    LoginCheckId,
    ProxyId,
    PoolId,
    ArtifactId,
    BackupId,
    AssetId,
    CaptchaSolveId,
    HostId,
);

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ClientId(String);

impl ClientId {
    pub fn new() -> ClientId {
        ClientId(ulid::Ulid::generate().to_string())
    }

    pub fn parse(s: &str) -> Result<ClientId, Error> {
        let valid_len = (1..=CLIENT_ID_MAX_LEN).contains(&s.len());
        let valid_chars = s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || CLIENT_ID_EXTRA_CHARS.contains(c));

        if valid_len && valid_chars {
            return Ok(ClientId(s.to_string()));
        }

        Err(Error::InvalidArgument {
            field: CLIENT_ID_FIELD.to_string(),
            message: format!(
                "must be 1 to {CLIENT_ID_MAX_LEN} characters from A-Z a-z 0-9 . _ : -"
            ),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ClientId {
    fn default() -> ClientId {
        ClientId::new()
    }
}

impl fmt::Display for ClientId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for ClientId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ClientId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ClientId, D::Error> {
        let s = String::deserialize(deserializer)?;

        ClientId::parse(&s).map_err(de::Error::custom)
    }
}

impl JsonSchema for ClientId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ClientId".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "minLength": 1,
            "maxLength": CLIENT_ID_MAX_LEN,
            "pattern": "^[A-Za-z0-9._:-]+$",
        })
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct TargetId(pub String);

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or_default()
}
