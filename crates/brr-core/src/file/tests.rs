use schemars::schema_for;
use serde_json::json;

use super::{FILE_MARK, FileOut, TextOut, TextUpload, Upload};

#[test]
fn upload_is_base64_in_json() {
    let upload = Upload(vec![0, 1, 2]);

    assert_eq!(serde_json::to_value(&upload).unwrap(), json!("AAEC"));
    assert_eq!(
        serde_json::from_value::<Upload>(json!("AAEC")).unwrap(),
        upload
    );
    assert!(serde_json::from_value::<Upload>(json!("not base64!")).is_err());
}

#[test]
fn debug_hides_content() {
    assert_eq!(format!("{:?}", Upload(vec![0, 1, 2])), "Upload(3 bytes)");
    assert_eq!(
        format!("{:?}", FileOut(vec![9; 5].into())),
        "FileOut(5 bytes)"
    );

    let text = format!("{:?}", TextUpload("hunter2".to_string()));

    assert_eq!(text, "TextUpload(7 bytes)");
    assert!(!text.contains("hunter2"));
}

#[test]
fn text_types_are_plain_strings() {
    assert_eq!(
        serde_json::to_value(TextOut("a,b".to_string())).unwrap(),
        json!("a,b")
    );
    assert_eq!(
        serde_json::from_value::<TextUpload>(json!("a,b")).unwrap(),
        TextUpload("a,b".to_string())
    );
}

#[test]
fn schemas_carry_direction_marks() {
    let upload = serde_json::to_value(schema_for!(Upload)).unwrap();
    let out = serde_json::to_value(schema_for!(FileOut)).unwrap();
    let text_in = serde_json::to_value(schema_for!(TextUpload)).unwrap();
    let text_out = serde_json::to_value(schema_for!(TextOut)).unwrap();

    assert_eq!(upload["type"], "string");
    assert_eq!(upload["contentEncoding"], "base64");
    assert_eq!(upload[FILE_MARK], "in");
    assert_eq!(out["contentEncoding"], "base64");
    assert_eq!(out[FILE_MARK], "out");
    assert_eq!(text_in[FILE_MARK], "in");
    assert!(text_in.get("contentEncoding").is_none());
    assert_eq!(text_out[FILE_MARK], "out");
}

#[test]
fn mark_sits_on_the_property() {
    #[derive(schemars::JsonSchema)]
    struct Params {
        #[allow(dead_code)]
        file: Upload,
    }

    let schema = serde_json::to_value(schema_for!(Params)).unwrap();

    assert_eq!(schema["properties"]["file"][FILE_MARK], "in");
}
