use brr_core::Display;
use brr_engine::LaunchSpec;
use brr_engine::compat::run_compat;
use tempfile::TempDir;

use crate::camoufox;

const STEPS: usize = 8;

#[tokio::test]
async fn a_release_that_works_passes_every_step() {
    let Some(assets) = camoufox::assets() else {
        return;
    };
    let dir = TempDir::new().unwrap();

    let report = run_compat(camoufox::spec(assets, dir.path(), Display::Headless, None)).await;

    assert!(report.passed, "{:?}", report.steps);
    assert_eq!(report.steps.len(), STEPS);
    assert!(report.steps.iter().all(|(_, result)| result.is_ok()));
}

#[tokio::test]
async fn a_binary_that_cannot_launch_fails_at_launch() {
    let Some(assets) = camoufox::assets() else {
        return;
    };
    let dir = TempDir::new().unwrap();
    let spec: LaunchSpec = camoufox::spec(assets, dir.path(), Display::Headless, None);
    let broken = LaunchSpec {
        executable: "/bin/false".into(),
        ..spec
    };

    let report = run_compat(broken).await;

    assert!(!report.passed);
    let (name, result) = report.steps.last().unwrap();
    assert_eq!(name, "launch");
    assert!(result.is_err());
}
