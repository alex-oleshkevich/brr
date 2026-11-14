use std::fmt;

use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

#[cfg(test)]
mod tests;

pub const FILE_MARK: &str = "x-brr-file";

const MARK_IN: &str = "in";
const MARK_OUT: &str = "out";

// Debug prints only the length: uploads carry credentials CSVs and passphrase-protected archives.
macro_rules! file_types {
    ($($name:ident($inner:ty) { schema: $base:expr, mark: $mark:expr, ser: $ser:expr, de: $de:expr })+) => {$(
        #[derive(Clone, PartialEq)]
        pub struct $name(pub $inner);

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({} bytes)", stringify!($name), self.0.len())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&$ser(&self.0))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<$name, D::Error> {
                $de(String::deserialize(deserializer)?).map($name).map_err(de::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn inline_schema() -> bool {
                true
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                let mut schema: Schema = $base;
                schema.insert(FILE_MARK.into(), $mark.into());
                schema
            }
        }
    )+};
}

fn base64_schema() -> Schema {
    json_schema!({ "type": "string", "contentEncoding": "base64" })
}

fn text_schema() -> Schema {
    json_schema!({ "type": "string" })
}

fn to_base64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

fn from_base64(text: String) -> Result<Vec<u8>, base64::DecodeError> {
    STANDARD.decode(text)
}

fn bytes_from_base64(text: String) -> Result<Bytes, base64::DecodeError> {
    from_base64(text).map(Bytes::from)
}

fn to_text(text: &str) -> String {
    text.to_owned()
}

fn from_text(text: String) -> Result<String, std::convert::Infallible> {
    Ok(text)
}

file_types! {
    Upload(Vec<u8>) { schema: base64_schema(), mark: MARK_IN, ser: to_base64, de: from_base64 }
    FileOut(Bytes) { schema: base64_schema(), mark: MARK_OUT, ser: to_base64, de: bytes_from_base64 }
    TextUpload(String) { schema: text_schema(), mark: MARK_IN, ser: to_text, de: from_text }
    TextOut(String) { schema: text_schema(), mark: MARK_OUT, ser: to_text, de: from_text }
}
