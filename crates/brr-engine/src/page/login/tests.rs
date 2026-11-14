use serde_json::json;

use super::LoginFields;

#[test]
fn login_fields_read_nulls_and_ignore_extras() {
    let fields: LoginFields = serde_json::from_value(json!({
        "username": "#u",
        "password": null,
        "totp": null,
        "submit": null,
        "passwordVisible": false,
    }))
    .unwrap();

    assert_eq!(fields.username.as_deref(), Some("#u"));
    assert_eq!(fields.password, None);
}
