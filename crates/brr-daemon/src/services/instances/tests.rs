use std::{fs, path::PathBuf};

use brr_core::{Build, BuildId, BuildSource, BuildState, ConflictReason, Engine, Error};
use test_case::test_case;

use super::{ensure_no_downgrade, owner::exit_error};

const LAST_VERSION: &str = "131.0.6778.85\n";

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
