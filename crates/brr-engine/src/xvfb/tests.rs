use std::path::Path;

use brr_core::{Error, Size};
use test_case::test_case;

use super::{DEFAULT_VIRTUAL_SCREEN, args, write_auth_file};

const AUTH: &str = "/tmp/xvfb.xauth";

#[test]
fn args_name_the_display_fd_and_the_screen() {
    let screen = Size {
        width: 1280,
        height: 800,
    };
    let args = args(&screen, Path::new(AUTH)).unwrap();

    let text: Vec<&str> = args.iter().filter_map(|arg| arg.to_str()).collect();
    let position = text.iter().position(|arg| *arg == "-displayfd").unwrap();
    assert_eq!(text[position + 1], "3");
    let position = text.iter().position(|arg| *arg == "-screen").unwrap();
    assert_eq!(&text[position + 1..position + 3], ["0", "1280x800x24"]);
    let position = text.iter().position(|arg| *arg == "-auth").unwrap();
    assert_eq!(text[position + 1], AUTH);
}

#[test]
fn the_auth_file_is_private_and_holds_a_fresh_cookie() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::TempDir::new().unwrap();
    let first = dir.path().join("first");
    let second = dir.path().join("second");

    write_auth_file(&first).unwrap();
    write_auth_file(&second).unwrap();

    let mode = std::fs::metadata(&first).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let (a, b) = (
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap(),
    );
    assert!(a.windows(18).any(|window| window == b"MIT-MAGIC-COOKIE-1"));
    assert_ne!(a, b);
}

#[test_case(1, 1; "one by one")]
#[test_case(1920, 1; "flat")]
#[test_case(0, 1080; "empty")]
fn tiny_screens_are_refused(width: u32, height: u32) {
    let error = args(&Size { width, height }, Path::new(AUTH)).unwrap_err();

    assert!(matches!(error, Error::InvalidArgument { field, .. } if field == "screen"));
}

#[test]
fn the_default_screen_is_accepted() {
    assert!(args(&DEFAULT_VIRTUAL_SCREEN, Path::new(AUTH)).is_ok());
}
