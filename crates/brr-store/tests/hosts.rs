use brr_core::{
    AdmissionMode, ConflictReason, Display, Error, Host, HostId, HostStatus, Instance, InstanceId,
    InstanceKind, InstanceState,
};
use brr_store::{Store, host, instance};
use test_case::test_case;

const NOW: i64 = 1_700_000_000_000;
const NAME: &str = "box-1";
const RENAMED: &str = "box-2";
const DOCKER_URL: &str = "ssh://alex@box.example.com";
const MAX_INSTANCES: u32 = 4;

fn host_named(name: &str) -> Host {
    Host {
        id: HostId::new(),
        name: name.to_string(),
        docker_url: DOCKER_URL.to_string(),
        max_instances: MAX_INSTANCES,
        enabled: true,
        created_at: NOW,
        status: HostStatus::default(),
    }
}

fn instance_on(host_id: HostId, state: InstanceState) -> Instance {
    Instance {
        id: InstanceId::new(),
        kind: InstanceKind::Managed,
        profile_id: None,
        build_id: None,
        state,
        display: Display::Headless,
        priority: 5,
        admission: AdmissionMode::Queue,
        pid: None,
        log_path: None,
        cdp_url: None,
        data_dir: None,
        ephemeral: false,
        promote_to_name: None,
        restart_count: 0,
        last_activity_at: NOW,
        queued_at: NOW,
        started_at: None,
        ended_at: None,
        end_reason: None,
        error: None,
        profile_name: None,
        settings: None,
        template_id: None,
        template_name: None,
        extensions: None,
        proxy_id: None,
        sandbox: None,
        traffic: None,
        host_id: Some(host_id),
        build: None,
        tab_count: 0,
        lease_count: 0,
        rss_bytes: None,
        capabilities: Vec::new(),
    }
}

#[tokio::test]
async fn host_crud_round_trips() -> Result<(), Error> {
    let store = Store::open_in_memory()?;
    let saved = host_named(NAME);
    let host_id = saved.id;
    let expected = saved.clone();

    let (by_id, by_name, all) = store
        .run(move |c| {
            host::insert(c, &saved)?;
            Ok((host::get(c, &host_id)?, host::find(c, NAME)?, host::all(c)?))
        })
        .await?;

    assert_eq!(by_id, expected);
    assert_eq!(by_name, expected);
    assert_eq!(all, vec![expected.clone()]);

    let mut changed = expected;
    changed.name = RENAMED.to_string();
    changed.enabled = false;
    let stored = changed.clone();
    let (updated, deleted, gone) = store
        .run(move |c| {
            host::update(c, &stored)?;
            let updated = host::find(c, RENAMED)?;
            host::delete(c, &host_id)?;

            Ok((updated, host::all(c)?, host::get(c, &host_id)))
        })
        .await?;

    assert_eq!(updated, changed);
    assert_eq!(deleted, Vec::new());
    assert!(matches!(gone, Err(Error::NotFound { .. })));

    Ok(())
}

#[tokio::test]
async fn host_name_unique() -> Result<(), Error> {
    let store = Store::open_in_memory()?;

    let second = store
        .run(|c| {
            host::insert(c, &host_named(NAME))?;
            host::insert(c, &host_named(NAME))
        })
        .await;

    assert!(matches!(
        second,
        Err(Error::Conflict {
            reason: ConflictReason::NameTaken,
            ..
        })
    ));

    Ok(())
}

#[test_case(InstanceState::Running, 1; "running counts")]
#[test_case(InstanceState::Stopped, 0; "stopped does not")]
#[tokio::test]
async fn host_counts_its_open_runs(state: InstanceState, expected: u32) -> Result<(), Error> {
    let store = Store::open_in_memory()?;
    let saved = host_named(NAME);
    let host_id = saved.id;
    let run = instance_on(host_id, state);
    let instance_id = run.id;

    let (counted, loaded) = store
        .run(move |c| {
            host::insert(c, &saved)?;
            instance::insert(c, &run)?;
            Ok((host::get(c, &host_id)?, instance::get(c, &instance_id)?))
        })
        .await?;

    assert_eq!(counted.status.running, expected);
    assert_eq!(loaded.host_id, Some(host_id));

    Ok(())
}

#[tokio::test]
async fn deleting_a_host_clears_its_runs() -> Result<(), Error> {
    let store = Store::open_in_memory()?;
    let saved = host_named(NAME);
    let host_id = saved.id;
    let run = instance_on(host_id, InstanceState::Stopped);
    let instance_id = run.id;

    let loaded = store
        .run(move |c| {
            host::insert(c, &saved)?;
            instance::insert(c, &run)?;
            host::delete(c, &host_id)?;
            instance::get(c, &instance_id)
        })
        .await?;

    assert_eq!(loaded.host_id, None);

    Ok(())
}
