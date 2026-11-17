use std::{fs, path::PathBuf};

use brr_core::{Build, BuildId, BuildSource, BuildState, ConflictReason, Engine, Error};
use test_case::test_case;

use super::{ensure_no_downgrade, last_log_line, owner::exit_error};

const LAST_VERSION: &str = "131.0.6778.85\n";
const WAYLAND_LOG: &str = "[9:9:1116/101010.1:ERROR:bus.cc(407)] Failed to connect to the bus\n\
    [9:9:1116/101010.2:FATAL:ozone_platform_wayland.cc(282)] Failed to connect to Wayland display\n\
    #0 0x55d5c8 <unknown>\n\n";
const WAYLAND_LINE: &str = "Failed to connect to Wayland display";
const PLAIN_LOG: &str =
    "starting\nError: no \x1b[31mDISPLAY\x1b[0m environment variable specified\n";
const PLAIN_LINE: &str = "Error: no [31mDISPLAY[0m environment variable specified";
const X_LOG: &str = "[7:7:1116/1.1:ERROR:ozone_platform_x11.cc(244)] Missing X server or $DISPLAY\n\
    [7:7:1116/1.2:ERROR:env.cc(257)] The platform failed to initialize.  Exiting.\n";
const X_LINE: &str = "Missing X server or $DISPLAY";

#[test_case(WAYLAND_LOG, Some(WAYLAND_LINE); "chromium fatal line without its prefix")]
#[test_case(X_LOG, Some(X_LINE); "cause before a generic exit line")]
#[test_case(PLAIN_LOG, Some(PLAIN_LINE); "last line without control characters")]
#[test_case("\n \n", None; "blank log")]
fn last_log_line_finds_the_reason(log: &str, expected: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("browser.log");
    fs::write(&path, log).unwrap();

    assert_eq!(last_log_line(&path).as_deref(), expected);
}

#[test_case(Some(0), None, None; "clean exit")]
#[test_case(Some(3), None, Some("browser exited with code 3"); "exit code")]
#[test_case(None, Some(9), Some("browser killed by signal 9"); "signal")]
#[test_case(None, None, None; "unknown")]
fn exit_error_describes_the_exit(code: Option<i32>, signal: Option<i32>, expected: Option<&str>) {
    assert_eq!(exit_error(code, signal).as_deref(), expected);
}

fn build_row(source: BuildSource, version: &str) -> Build {
    Build {
        id: BuildId::new(),
        source,
        engine: Engine::Chromium,
        name: "chrome".to_string(),
        version: version.to_string(),
        channel: None,
        path: PathBuf::from("/nonexistent"),
        state: BuildState::Installed,
        sha256: None,
        size_bytes: 0,
        capabilities: Vec::new(),
        installed_at: None,
        last_referenced_at: None,
    }
}

#[test_case("130.0.6723.58", true; "older browser is refused")]
#[test_case("131.0.6778.85", false; "same version passes")]
#[test_case("132.0.0.1", false; "newer browser passes")]
fn downgrade_check(build_version: &str, refused: bool) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Last Version"), LAST_VERSION).unwrap();

    let result = ensure_no_downgrade(dir.path(), &build_row(BuildSource::Cft, build_version));

    if refused {
        assert_eq!(
            result,
            Err(Error::Conflict {
                reason: ConflictReason::Downgrade,
                detail: None
            })
        );
    } else {
        assert_eq!(result, Ok(()));
    }
}

#[test]
fn downgrade_check_passes_without_last_version() {
    let dir = tempfile::tempdir().unwrap();

    assert_eq!(
        ensure_no_downgrade(dir.path(), &build_row(BuildSource::Cft, "1.0.0.0")),
        Ok(())
    );
}

#[test]
fn downgrade_check_skips_brave() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Last Version"), LAST_VERSION).unwrap();

    assert_eq!(
        ensure_no_downgrade(dir.path(), &build_row(BuildSource::Brave, "1.99.57")),
        Ok(())
    );
}
