use std::path::Path;

use brr_builds::{CompatRecord, write_compat};
use brr_core::{Build, BuildId, BuildSource, BuildState, Engine};
use tempfile::TempDir;

use super::{Join, camoufox_join};

fn row(dir: &Path) -> Build {
    Build {
        id: BuildId::new(),
        source: BuildSource::Camoufox,
        engine: Engine::Camoufox,
        name: "Camoufox".to_owned(),
        version: "1".to_owned(),
        channel: None,
        path: dir.join("camoufox-bin"),
        state: BuildState::Installed,
        sha256: None,
        size_bytes: 0,
        capabilities: Vec::new(),
        installed_at: None,
        last_referenced_at: None,
    }
}

fn record(dir: &Path, passed: bool, brr_version: &str) {
    write_compat(
        dir,
        &CompatRecord {
            passed,
            checked_at: 0,
            failed_step: None,
            brr_version: brr_version.to_owned(),
        },
    )
    .unwrap();
}

#[test]
fn joins_by_record_of_this_version() {
    let dir = TempDir::new().unwrap();
    let build = row(dir.path());
    assert_eq!(camoufox_join(&build, "2"), Join::Regate);

    record(dir.path(), true, "2");
    assert_eq!(camoufox_join(&build, "2"), Join::Yes);
    assert_eq!(camoufox_join(&build, "3"), Join::Regate);

    record(dir.path(), false, "2");
    assert_eq!(camoufox_join(&build, "2"), Join::No);
}
