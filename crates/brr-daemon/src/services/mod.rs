pub mod admission;
pub mod artifacts;
pub mod backups;
pub mod bookmarks;
pub mod builds;
pub mod camoufox;
pub mod captcha;
pub mod clients;
pub mod compat;
pub mod content;
pub mod cookies;
pub mod credentials;
pub mod doctor;
pub mod downloads;
pub mod events;
pub mod extensions;
pub mod gc;
pub mod host;
pub mod instances;
pub mod leases;
pub mod login;
pub mod login_checks;
pub mod oneshot;
pub mod profiles;
pub mod proxies;
pub mod quota;
pub mod reaper;
pub mod relay;
pub mod retention;
pub mod screencast;
pub mod secrets;
pub mod storage;
pub mod supervisor;

// A check interval of 0 would spin.
pub(crate) const MIN_CHECK_S: u64 = 1;

// Event kinds several services wait on.
pub(crate) const STATE_CHANGED: &str = "instance.state_changed";
pub(crate) const SETTINGS_CHANGED: &str = "settings.changed";
pub(crate) const LIMITS_CHANGED: &str = "limits.changed";

// Sleeps to the deadline; with none, never wakes.
pub(crate) async fn until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}
