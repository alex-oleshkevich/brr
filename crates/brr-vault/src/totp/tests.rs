use brr_core::Error;

use super::{totp_check, totp_code};

const SECRET: &str = "JBSWY3DPEHPK3PXP";

// Reference values for SECRET at t=59, computed independently with Python's hmac/hashlib
// (RFC 4226 dynamic truncation of HMAC-SHA1 over counter 1).
const CODE_6: &str = "996554";
const CODE_8: &str = "41996554";

#[test]
fn base32_defaults() {
    let code = totp_code(SECRET, 59).expect("code");

    assert_eq!(code.expose(), CODE_6);
    assert_eq!(code.expose().len(), 6);
}

#[test]
fn base32_tolerates_spaces_and_case() {
    let spaced = totp_code("jbsw y3dp ehpk 3pxp", 59).expect("code");

    assert_eq!(spaced.expose(), totp_code(SECRET, 59).unwrap().expose());
}

#[test]
fn uri_params_win() {
    let uri = format!("otpauth://totp/x?secret={SECRET}&digits=8");
    let code = totp_code(&uri, 59).expect("code");

    assert_eq!(code.expose(), CODE_8);
    assert_eq!(code.expose().len(), 8);
}

#[test]
fn rejects_garbage() {
    for input in [
        "not base32!",
        "",
        "   ",
        "A",
        "otpauth://totp/x?secret=",
        "otpauth://totp/x",
        "otpauth://hotp/x?secret=JBSWY3DPEHPK3PXP",
        "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&digits=12",
    ] {
        let err = totp_check(input).expect_err(input);

        let Error::InvalidArgument { field, message } = err else {
            panic!("wrong error for {input}");
        };
        assert_eq!(field, "totp");
        assert!(input.trim().is_empty() || !message.contains(input));
    }
}
