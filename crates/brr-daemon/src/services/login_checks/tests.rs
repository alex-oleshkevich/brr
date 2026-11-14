use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use brr_core::{
    CredentialId, EventData, InstanceId, InstanceState, InstanceTrigger, LoginCheckId, ProfileId,
};
use futures::FutureExt;

use super::{BackoffKey, LoginChecks, RunProfile};

const INTERVAL: Duration = Duration::from_secs(60);

fn changed(
    instance_id: InstanceId,
    profile_id: Option<ProfileId>,
    from: InstanceState,
    to: InstanceState,
) -> EventData {
    EventData::InstanceStateChanged {
        instance_id,
        profile_id,
        from: Some(from),
        to,
        trigger: InstanceTrigger::Ready,
        reason: None,
    }
}

fn counting() -> (RunProfile, Arc<AtomicU32>) {
    let runs = Arc::new(AtomicU32::new(0));
    let counter = runs.clone();
    let run: RunProfile = Arc::new(move |_, _| {
        counter.fetch_add(1, Ordering::SeqCst);

        async { true }.boxed()
    });

    (run, runs)
}

// Lets the spawned tasks run up to the point where they wait on the paused clock.
async fn advance(by: Duration) {
    tokio::task::yield_now().await;
    tokio::time::advance(by).await;
    tokio::task::yield_now().await;
}

#[tokio::test(start_paused = true)]
async fn interval_lifecycle() {
    let checks = LoginChecks::default();
    let (run, runs) = counting();
    let (instance_id, profile_id) = (InstanceId::new(), Some(ProfileId::new()));
    let event = |from, to| changed(instance_id, profile_id, from, to);

    checks.on_event(
        &run,
        &event(InstanceState::Starting, InstanceState::Running),
        INTERVAL,
    );
    advance(Duration::ZERO).await;
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the check at launch");

    advance(INTERVAL).await;
    assert_eq!(runs.load(Ordering::SeqCst), 2, "+1 min");
    advance(INTERVAL).await;
    assert_eq!(runs.load(Ordering::SeqCst), 3, "+2 min");

    checks.on_event(
        &run,
        &event(InstanceState::Running, InstanceState::Stopping),
        INTERVAL,
    );
    advance(INTERVAL * 5).await;
    assert_eq!(runs.load(Ordering::SeqCst), 3, "stopped");
}

#[tokio::test(start_paused = true)]
async fn only_a_launch_starts_the_checks() {
    let checks = LoginChecks::default();
    let (run, runs) = counting();
    let instance_id = InstanceId::new();

    checks.on_event(
        &run,
        &changed(
            instance_id,
            Some(ProfileId::new()),
            InstanceState::Idle,
            InstanceState::Running,
        ),
        INTERVAL,
    );
    checks.on_event(
        &run,
        &changed(
            instance_id,
            None,
            InstanceState::Starting,
            InstanceState::Running,
        ),
        INTERVAL,
    );
    advance(INTERVAL * 2).await;

    assert_eq!(runs.load(Ordering::SeqCst), 0);
}

fn key(credential_id: CredentialId, credential_updated_at: i64) -> BackoffKey {
    BackoffKey {
        url: "https://a.test/account".to_owned(),
        expect_selector: None,
        expect_text: Some("hi".to_owned()),
        credential_id,
        credential_updated_at,
    }
}

#[test]
fn third_failure_suspends_until_the_credential_changes() {
    let checks = LoginChecks::default();
    let (id, credential, profile_id) = (LoginCheckId::new(), CredentialId::new(), ProfileId::new());

    for attempt in 1..=3 {
        assert!(checks.relogin_allowed(&id, &key(credential, 1)));
        let reached = checks.relogin_outcome(&id, &profile_id, key(credential, 1), false);
        assert_eq!(reached, (attempt == 3).then_some(3));
    }
    assert!(
        !checks.relogin_allowed(&id, &key(credential, 1)),
        "suspended"
    );
    assert!(
        checks.relogin_allowed(&id, &key(credential, 2)),
        "credential changed"
    );
    assert_eq!(
        checks.relogin_outcome(&id, &profile_id, key(credential, 2), false),
        None,
        "count restarted"
    );
}

#[test]
fn success_and_check_edits_reset_the_count() {
    let checks = LoginChecks::default();
    let (id, credential, profile_id) = (LoginCheckId::new(), CredentialId::new(), ProfileId::new());

    checks.relogin_outcome(&id, &profile_id, key(credential, 1), false);
    checks.relogin_outcome(&id, &profile_id, key(credential, 1), false);
    checks.relogin_outcome(&id, &profile_id, key(credential, 1), true);
    assert_eq!(
        checks.relogin_outcome(&id, &profile_id, key(credential, 1), false),
        None,
        "reset by a success"
    );

    checks.relogin_outcome(&id, &profile_id, key(credential, 1), false);
    checks.relogin_outcome(&id, &profile_id, key(credential, 1), false);
    let mut edited = key(credential, 1);
    edited.url = "https://a.test/other".to_owned();
    assert_eq!(
        checks.relogin_outcome(&id, &profile_id, edited, false),
        None,
        "reset by an edit"
    );
}

#[test]
fn deleted_checks_lose_their_backoff() {
    let checks = LoginChecks::default();
    let (kept, deleted, other) = (
        LoginCheckId::new(),
        LoginCheckId::new(),
        LoginCheckId::new(),
    );
    let (profile_id, other_profile_id, credential) =
        (ProfileId::new(), ProfileId::new(), CredentialId::new());
    for (id, profile_id) in [
        (&kept, &profile_id),
        (&deleted, &profile_id),
        (&other, &other_profile_id),
    ] {
        checks.relogin_outcome(id, profile_id, key(credential, 1), false);
    }

    checks.forget_deleted(&profile_id, &HashSet::from([kept]));

    let backoff = checks.backoff.lock().unwrap();
    assert!(backoff.contains_key(&kept));
    assert!(!backoff.contains_key(&deleted));
    assert!(
        backoff.contains_key(&other),
        "another Profile's check stays"
    );
}
