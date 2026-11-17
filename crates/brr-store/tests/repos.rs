use std::path::PathBuf;

use brr_core::{
    AdmissionMode, Build, BuildChoice, BuildId, BuildSource, BuildState, Capability, Client,
    ClientId, ClientKind, ConflictReason, Display, Engine, Error, Event, Instance, InstanceId,
    InstanceKind, InstanceState, Lease, LeaseId, Order, Page, Profile, ProfileId, ProfileKind,
    ProfileSettings, ProxyId, ReleaseReason, SandboxMode, Subject, SubjectType, TabId, Template,
    TemplateId, Traffic,
};
use brr_store::{
    Store, build, client, event,
    instance::{self, TrafficGroup},
    lease, profile, secret, template,
};
use serde_json::json;
use tempfile::TempDir;
use test_case::test_case;

const TABLES: [&str; 26] = [
    "build",
    "asset",
    "template",
    "template_extension",
    "profile",
    "profile_tag",
    "extension",
    "extension_version",
    "profile_extension",
    "credential",
    "credential_profile",
    "login_check",
    "proxy",
    "proxy_pool",
    "pool_member",
    "pool_sticky",
    "host",
    "instance",
    "tab",
    "client",
    "lease",
    "artifact",
    "backup",
    "captcha_solve",
    "event",
    "secret",
];
const FIRST_INSTANCE_ID: &str = "01JD0000000000000000000001";
const SECOND_INSTANCE_ID: &str = "01JD0000000000000000000002";
const NOW: i64 = 1_700_000_000_000;
const ALL_STATES: [InstanceState; 14] = [
    InstanceState::Queued,
    InstanceState::Starting,
    InstanceState::Running,
    InstanceState::Idle,
    InstanceState::Reaping,
    InstanceState::Stopping,
    InstanceState::Stopped,
    InstanceState::Killed,
    InstanceState::Crashed,
    InstanceState::Failed,
    InstanceState::Attaching,
    InstanceState::Attached,
    InstanceState::Detached,
    InstanceState::Lost,
];

#[allow(clippy::unwrap_used)]
fn open() -> (TempDir, PathBuf, Store) {
    let dir = TempDir::new().unwrap();
    let db_file = dir.path().join("data").join("brr.db");
    let store = Store::open(&db_file).unwrap();

    (dir, db_file, store)
}

fn profile_named(name: &str) -> Profile {
    Profile {
        id: ProfileId::new(),
        name: name.to_string(),
        kind: ProfileKind::Persistent,
        template_id: None,
        template_name: None,
        settings: ProfileSettings::default(),
        pinned: false,
        notes: String::new(),
        tags: Vec::new(),
        data_dir: None,
        disk_quota_bytes: None,
        backup_schedule: None,
        backup_keep: 7,
        created_at: NOW,
        updated_at: NOW,
        capabilities: Vec::new(),
        login_status: None,
        builtin: false,
    }
}

fn instance_for(profile_id: Option<ProfileId>, state: InstanceState) -> Instance {
    Instance {
        id: InstanceId::new(),
        kind: InstanceKind::Managed,
        profile_id,
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
        host_id: None,
        build: None,
        tab_count: 0,
        lease_count: 0,
        rss_bytes: None,
        capabilities: Vec::new(),
    }
}

fn build_named(version: &str, state: BuildState, installed_at: Option<i64>) -> Build {
    Build {
        id: BuildId::new(),
        source: BuildSource::Cft,
        engine: Engine::Chromium,
        name: "Chrome for Testing".to_string(),
        version: version.to_string(),
        channel: Some("stable".to_string()),
        path: PathBuf::from(format!("/builds/{version}/chrome")),
        state,
        sha256: None,
        size_bytes: 1_000_000,
        capabilities: vec![Capability::Pdf, Capability::Screencast],
        installed_at,
        last_referenced_at: None,
    }
}

fn lease_on(tab_id: Option<TabId>, instance_id: InstanceId) -> Lease {
    Lease {
        id: LeaseId::new(),
        client_id: ClientId::new(),
        client_kind: ClientKind::Mcp,
        client_label: "agent".to_string(),
        instance_id,
        tab_id,
        exclusive: true,
        acquired_at: NOW,
        released_at: None,
        release_reason: None,
    }
}

fn event_at(seq: i64, subject: Subject, data: serde_json::Value) -> Event {
    Event {
        seq,
        ts: NOW + seq,
        kind: "instance.state".to_string(),
        subject,
        actor_client_id: None,
        data,
    }
}

fn instance_subject(instance_id: &InstanceId) -> Subject {
    Subject {
        kind: SubjectType::Instance,
        id: instance_id.to_string(),
    }
}

fn query_for(page: Page) -> event::Query {
    event::Query {
        since: None,
        kinds: Vec::new(),
        subject: None,
        instance_id: None,
        from_ts: None,
        to_ts: None,
        order: Order::Asc,
        page,
    }
}

fn seqs(events: &[Event]) -> Vec<i64> {
    events.iter().map(|e| e.seq).collect()
}

#[tokio::test]
async fn schema_has_every_table() {
    let (_dir, _db_file, store) = open();

    let (journal_mode, tables) = store
        .run(|c| {
            let journal_mode: String = c
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .unwrap();
            let mut stmt = c
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
                .unwrap();
            let tables: Vec<String> = stmt
                .query_map([], |row| row.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();

            Ok((journal_mode, tables))
        })
        .await
        .unwrap();

    let mut expected: Vec<String> = TABLES.iter().map(|t| t.to_string()).collect();
    expected.sort();

    assert_eq!(journal_mode, "wal");
    assert_eq!(tables, expected);
}

#[tokio::test]
async fn profile_name_unique() {
    let (_dir, _db_file, store) = open();

    let second = store
        .run(|c| {
            profile::insert(c, &profile_named("work"))?;
            profile::insert(c, &profile_named("work"))
        })
        .await;

    assert_eq!(
        second,
        Err(Error::Conflict {
            reason: ConflictReason::NameTaken,
            detail: None
        })
    );
}

#[tokio::test]
async fn template_name_unique() {
    let (_dir, _db_file, store) = open();
    let new_template = |name: &str| Template {
        id: TemplateId::new(),
        name: name.to_string(),
        builtin: false,
        description: None,
        settings: ProfileSettings::default(),
        extensions: Vec::new(),
        created_at: NOW,
        updated_at: NOW,
    };

    let second = store
        .run(move |c| {
            template::insert(c, &new_template("stealth"))?;
            template::insert(c, &new_template("stealth"))
        })
        .await;

    assert_eq!(
        second,
        Err(Error::Conflict {
            reason: ConflictReason::NameTaken,
            detail: None
        })
    );
}

#[tokio::test]
async fn profile_tags_and_build_columns() {
    let (_dir, _db_file, store) = open();
    let build_id = BuildId::new();
    let mut by_channel = profile_named("a");
    by_channel.tags = vec!["shop".to_string(), "eu".to_string()];
    let mut by_build = profile_named("b");
    by_build.settings.build = BuildChoice::BuildId(build_id);
    by_build.tags = vec!["eu".to_string()];
    let (a, b) = (by_channel.clone(), by_build.clone());

    let (stored, eu, shop, columns) = store
        .run(move |c| {
            profile::insert(c, &a)?;
            profile::insert(c, &b)?;

            let stored = profile::get(c, &a.id)?;
            let (eu, _) = profile::list(c, Some("eu"), &Page::new(None, None))?;
            let (shop, _) = profile::list(c, Some("shop"), &Page::new(None, None))?;
            let columns: Vec<(Option<String>, Option<String>)> = c
                .prepare("SELECT build_id, channel FROM profile ORDER BY name")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect();

            let mut retagged = a.clone();
            retagged.tags = vec!["new".to_string()];
            profile::update(c, &retagged)?;
            assert_eq!(profile::get(c, &a.id)?.tags, vec!["new".to_string()]);

            Ok((stored, eu, shop, columns))
        })
        .await
        .unwrap();

    let mut sorted_tags = by_channel.tags.clone();
    sorted_tags.sort();
    assert_eq!(stored.tags, sorted_tags);
    assert_eq!(
        eu.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![by_channel.id, by_build.id]
    );
    assert_eq!(
        shop.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![by_channel.id]
    );
    assert_eq!(
        columns,
        vec![
            (None, Some("cft:stable".to_string())),
            (Some(build_id.to_string()), None)
        ]
    );
}

#[tokio::test]
async fn lease_active_tab_unique() {
    let (_dir, _db_file, store) = open();
    let instance_id = InstanceId::new();
    let tab_id = TabId::new();
    let first = lease_on(Some(tab_id), instance_id);
    let second = lease_on(Some(tab_id), instance_id);
    let (first_id, second_clone) = (first.id, second.clone());

    let (blocked, after_release) = store
        .run(move |c| {
            lease::insert(c, &first)?;
            let blocked = lease::insert(c, &second_clone);
            lease::release(c, &first_id, ReleaseReason::Released, NOW)?;
            let after_release = lease::insert(c, &second_clone);

            Ok((blocked, after_release))
        })
        .await
        .unwrap();

    assert!(
        matches!(&blocked, Err(Error::Internal { message }) if message.contains("lease.tab_id")),
        "{blocked:?}"
    );
    assert_eq!(after_release, Ok(()));
    let active = store.run(|c| lease::active(c)).await.unwrap();
    assert_eq!(
        active.iter().map(|l| l.id).collect::<Vec<_>>(),
        vec![second.id]
    );
}

#[tokio::test]
async fn event_query() {
    let (_dir, _db_file, store) = open();
    let x = InstanceId::new();
    let y = InstanceId::new();

    let events = vec![
        event_at(1, instance_subject(&x), json!({})),
        event_at(
            2,
            instance_subject(&y),
            json!({ "instance_id": y.to_string() }),
        ),
        event_at(
            3,
            instance_subject(&y),
            json!({ "instance_id": x.to_string() }),
        ),
        event_at(
            4,
            Subject {
                kind: SubjectType::Tab,
                id: TabId::new().to_string(),
            },
            json!({ "instance_id": x.to_string() }),
        ),
        event_at(5, instance_subject(&y), json!({})),
    ];

    let (for_x, since, newest, next_page) = store
        .run(move |c| {
            for e in &events {
                event::insert(c, e)?;
            }

            let (for_x, _) = event::list(
                c,
                &event::Query {
                    instance_id: Some(x),
                    ..query_for(Page::new(None, None))
                },
            )?;
            let (since, _) = event::list(
                c,
                &event::Query {
                    since: Some(3),
                    order: Order::Desc,
                    ..query_for(Page::new(None, None))
                },
            )?;
            let (newest, cursor) = event::list(
                c,
                &event::Query {
                    order: Order::Desc,
                    ..query_for(Page::new(Some(2), None))
                },
            )?;
            let (next_page, _) = event::list(
                c,
                &event::Query {
                    order: Order::Desc,
                    ..query_for(Page::new(Some(2), cursor.clone()))
                },
            )?;
            assert_eq!(event::max_seq(c)?, 5);

            Ok((for_x, since, (newest, cursor), next_page))
        })
        .await
        .unwrap();

    assert_eq!(seqs(&for_x), vec![1, 3, 4]);
    assert_eq!(seqs(&since), vec![4, 5]);
    assert_eq!(seqs(&newest.0), vec![5, 4]);
    assert_eq!(newest.1, Some("4".to_string()));
    assert_eq!(seqs(&next_page), vec![3, 2]);
}

#[tokio::test]
async fn event_filters() {
    let (_dir, _db_file, store) = open();
    let x = InstanceId::new();
    let mut tab_event = event_at(
        2,
        Subject {
            kind: SubjectType::Tab,
            id: TabId::new().to_string(),
        },
        json!({}),
    );
    tab_event.kind = "tab.opened".to_string();
    let events = vec![
        event_at(1, instance_subject(&x), json!({})),
        tab_event.clone(),
    ];
    let tab_subject = tab_event.subject.clone();

    let (by_kind, by_subject, by_time) = store
        .run(move |c| {
            for e in &events {
                event::insert(c, e)?;
            }

            let (by_kind, _) = event::list(
                c,
                &event::Query {
                    kinds: vec!["tab.opened".to_string()],
                    ..query_for(Page::new(None, None))
                },
            )?;
            let (by_subject, _) = event::list(
                c,
                &event::Query {
                    subject: Some(tab_subject),
                    ..query_for(Page::new(None, None))
                },
            )?;
            let (by_time, _) = event::list(
                c,
                &event::Query {
                    from_ts: Some(NOW + 1),
                    to_ts: Some(NOW + 1),
                    ..query_for(Page::new(None, None))
                },
            )?;

            Ok((by_kind, by_subject, by_time))
        })
        .await
        .unwrap();

    assert_eq!(seqs(&by_kind), vec![2]);
    assert_eq!(seqs(&by_subject), vec![2]);
    assert_eq!(seqs(&by_time), vec![1]);
}

#[tokio::test]
async fn active_for_profile() {
    let (_dir, _db_file, store) = open();
    let p = profile_named("work");
    let profile_id = p.id;
    let mut running = instance_for(Some(profile_id), InstanceState::Running);
    let running_id = running.id;
    let running_copy = running.clone();

    let (found, after_stop) = store
        .run(move |c| {
            profile::insert(c, &p)?;
            instance::insert(c, &running_copy)?;
            let found = instance::active_for_profile(c, &profile_id)?;

            running.state = InstanceState::Stopped;
            instance::update(c, &running)?;

            Ok((found, instance::active_for_profile(c, &profile_id)?))
        })
        .await
        .unwrap();

    assert_eq!(found.map(|i| i.id), Some(running_id));
    assert_eq!(after_stop, None);
}

#[test_case(Some(SandboxMode::Docker); "docker")]
#[test_case(None; "none")]
#[tokio::test]
async fn instance_sandbox_round_trips(sandbox: Option<SandboxMode>) -> Result<(), Error> {
    let (_dir, _db_file, store) = open();
    let mut saved = instance_for(None, InstanceState::Running);
    saved.sandbox = sandbox;
    let id = saved.id;

    let loaded = store
        .run(move |c| {
            instance::insert(c, &saved)?;
            instance::get(c, &id)
        })
        .await?;

    assert_eq!(loaded.sandbox, sandbox);

    Ok(())
}

#[tokio::test]
async fn instance_traffic_round_trips_and_stays_null_when_unmeasured() -> Result<(), Error> {
    let (_dir, _db_file, store) = open();
    let mut measured = instance_for(None, InstanceState::Running);
    measured.traffic = Some(Traffic {
        sent_bytes: 10,
        received_bytes: 2_000,
    });
    let unmeasured = instance_for(None, InstanceState::Running);
    let (measured_id, unmeasured_id) = (measured.id, unmeasured.id);

    let (first, second) = store
        .run(move |c| {
            instance::insert(c, &measured)?;
            instance::insert(c, &unmeasured)?;
            Ok((
                instance::get(c, &measured_id)?,
                instance::get(c, &unmeasured_id)?,
            ))
        })
        .await?;

    assert_eq!(
        first.traffic.map(|t| (t.sent_bytes, t.received_bytes)),
        Some((10, 2_000))
    );
    assert_eq!(second.traffic, None);

    let set = store
        .run(move |c| {
            let totals = Traffic {
                sent_bytes: 5,
                received_bytes: 7,
            };
            instance::set_traffic(c, &unmeasured_id, &totals)?;
            instance::get(c, &unmeasured_id)
        })
        .await?;

    assert_eq!(
        set.traffic.map(|t| (t.sent_bytes, t.received_bytes)),
        Some((5, 7))
    );

    Ok(())
}

#[tokio::test]
async fn traffic_totals_sum_ended_measured_runs_per_proxy_within_the_window() -> Result<(), Error> {
    let (_dir, _db_file, store) = open();
    let proxy_a = ProxyId::new();
    let proxy_b = ProxyId::new();
    let ended = |proxy: ProxyId, at: i64, bytes: Option<(u64, u64)>| {
        let mut row = instance_for(None, InstanceState::Stopped);
        row.proxy_id = Some(proxy);
        row.ended_at = Some(at);
        row.traffic = bytes.map(|(sent_bytes, received_bytes)| Traffic {
            sent_bytes,
            received_bytes,
        });
        row
    };
    let rows = [
        ended(proxy_a, 100, Some((1, 10))),
        ended(proxy_a, 200, Some((2, 20))),
        ended(proxy_b, 300, Some((4, 40))),
        ended(proxy_b, 50, Some((8, 80))),
        ended(proxy_b, 300, None),
    ];

    let (totals, mut ends) = store
        .run(move |c| {
            rows.iter().try_for_each(|row| instance::insert(c, row))?;
            Ok((
                instance::traffic_totals(c, TrafficGroup::Proxy, 100)?,
                instance::traffic_ends(c, 100, 250)?,
            ))
        })
        .await?;

    let summary: Vec<_> = totals
        .iter()
        .map(|t| (t.id.clone(), t.sent_bytes, t.received_bytes, t.runs))
        .collect();
    assert_eq!(
        summary,
        vec![
            (proxy_b.to_string(), 4, 40, 1),
            (proxy_a.to_string(), 3, 30, 2)
        ]
    );
    ends.sort_unstable();
    assert_eq!(ends, vec![(100, 11), (200, 22)]);

    Ok(())
}

#[tokio::test]
async fn non_final_matches_state_machine() {
    let (_dir, _db_file, store) = open();

    let kept = store
        .run(|c| {
            for state in ALL_STATES {
                instance::insert(c, &instance_for(None, state))?;
            }

            Ok(instance::non_final(c)?
                .into_iter()
                .map(|i| i.state)
                .collect::<Vec<_>>())
        })
        .await
        .unwrap();

    for state in ALL_STATES {
        assert_eq!(kept.contains(&state), !state.is_final(), "{state:?}");
    }
}

#[tokio::test]
async fn instance_list_is_newest_first_and_filters() {
    let (_dir, _db_file, store) = open();
    let p = profile_named("work");
    let profile_id = p.id;
    let mut first = instance_for(Some(profile_id), InstanceState::Running);
    let mut second = instance_for(None, InstanceState::Stopped);
    first.id = FIRST_INSTANCE_ID.parse().unwrap();
    second.id = SECOND_INSTANCE_ID.parse().unwrap();
    let (first_id, second_id) = (first.id, second.id);

    let (all, page_one, page_two, running, mine) = store
        .run(move |c| {
            profile::insert(c, &p)?;
            instance::insert(c, &first)?;
            instance::insert(c, &second)?;

            let filter = instance::Filter::default();
            let (all, _) = instance::list(c, &filter, &Page::new(None, None))?;
            let (page_one, cursor) = instance::list(c, &filter, &Page::new(Some(1), None))?;
            let (page_two, _) = instance::list(c, &filter, &Page::new(Some(1), cursor))?;
            let (running, _) = instance::list(
                c,
                &instance::Filter {
                    states: vec![InstanceState::Running],
                    ..instance::Filter::default()
                },
                &Page::new(None, None),
            )?;
            let (mine, _) = instance::list(
                c,
                &instance::Filter {
                    profile_id: Some(profile_id),
                    ..instance::Filter::default()
                },
                &Page::new(None, None),
            )?;

            Ok((all, page_one, page_two, running, mine))
        })
        .await
        .unwrap();

    let ids = |rows: &[Instance]| rows.iter().map(|i| i.id).collect::<Vec<_>>();
    assert_eq!(ids(&all), vec![second_id, first_id]);
    assert_eq!(ids(&page_one), vec![second_id]);
    assert_eq!(ids(&page_two), vec![first_id]);
    assert_eq!(ids(&running), vec![first_id]);
    assert_eq!(ids(&mine), vec![first_id]);
}

#[tokio::test]
async fn reopen_keeps_data() {
    let (_dir, db_file, store) = open();
    let p = profile_named("work");
    let profile_id = p.id;
    store.run(move |c| profile::insert(c, &p)).await.unwrap();
    drop(store);

    let reopened = Store::open(&db_file).unwrap();
    let again = Store::open(&db_file).unwrap();

    let from_reopened = reopened.run(move |c| profile::get(c, &profile_id)).await;
    let from_again = again.run(move |c| profile::get(c, &profile_id)).await;

    assert_eq!(from_reopened.map(|p| p.name), Ok("work".to_string()));
    assert_eq!(from_again.map(|p| p.id), Ok(profile_id));
}

#[tokio::test]
async fn builds_page_by_install_time_and_fail_when_interrupted() {
    let (_dir, _db_file, store) = open();
    let old = build_named("1", BuildState::Installed, Some(NOW));
    let new = build_named("2", BuildState::Installed, Some(NOW + 10));
    let pending = build_named("3", BuildState::Installing, None);
    let (old_id, new_id, pending_id) = (old.id, new.id, pending.id);
    let new_copy = new.clone();
    let new_path = new.path.clone();

    let (page_one, page_two, rest, interrupted, state, found, by_path, in_order) = store
        .run(move |c| {
            build::insert(c, &old)?;
            build::insert(c, &new)?;
            build::insert(c, &pending)?;

            let (page_one, cursor) = build::list(c, &Page::new(Some(2), None))?;
            let (page_two, cursor) = build::list(c, &Page::new(Some(2), cursor))?;
            let interrupted = build::mark_interrupted(c)?;
            let state = build::get(c, &pending_id)?.state;
            let found = build::find(c, BuildSource::Cft, "2")?;
            let by_path = build::find_by_path(c, &new_path)?;
            let in_order = build::all(c)?.into_iter().map(|b| b.id).collect::<Vec<_>>();

            Ok((
                page_one,
                page_two,
                cursor,
                interrupted,
                state,
                found,
                by_path,
                in_order,
            ))
        })
        .await
        .unwrap();

    let ids = |rows: &[Build]| rows.iter().map(|b| b.id).collect::<Vec<_>>();
    assert_eq!(ids(&page_one), vec![new_id, old_id]);
    assert_eq!(ids(&page_two), vec![pending_id]);
    assert_eq!(rest, None);
    assert_eq!(interrupted, 1);
    assert_eq!(state, BuildState::Failed);
    assert_eq!(found, Some(new_copy.clone()));
    assert_eq!(by_path, Some(new_copy));
    assert_eq!(in_order, vec![old_id, new_id, pending_id]);
}

#[tokio::test]
async fn client_upsert_revives_ended_client() {
    let (_dir, _db_file, store) = open();
    let client_id = ClientId::parse("ci-runner").unwrap();
    let new_client = |label: &str, created_at: i64| Client {
        id: client_id.clone(),
        kind: ClientKind::Rest,
        label: label.to_string(),
        current_tab_id: None,
        created_at,
        ended_at: None,
    };
    let (first, second) = (new_client("old", NOW), new_client("new", NOW + 5));
    let id = client_id.clone();

    let (ended, revived, active) = store
        .run(move |c| {
            client::upsert(c, &first)?;
            client::end(c, &id, NOW + 1)?;
            let ended = client::get(c, &id)?;
            client::upsert(c, &second)?;

            Ok((ended, client::get(c, &id)?, client::active(c)?))
        })
        .await
        .unwrap();

    assert_eq!(ended.ended_at, Some(NOW + 1));
    assert_eq!(revived, new_client("new", NOW + 5));
    assert_eq!(active.len(), 1);
}

#[tokio::test]
async fn secrets_round_trip() {
    let (_dir, _db_file, store) = open();

    let (value, replaced, missing, names) = store
        .run(|c| {
            secret::put(c, "vault", &[1, 2, 3], NOW)?;
            let value = secret::get(c, "vault")?;
            secret::put(c, "vault", &[9], NOW + 1)?;
            let replaced = secret::get(c, "vault")?;
            secret::put(c, "other", &[0], NOW)?;
            let names = secret::names(c)?;
            secret::delete(c, "vault")?;

            Ok((value, replaced, secret::get(c, "vault")?, names))
        })
        .await
        .unwrap();

    assert_eq!(value, Some(vec![1, 2, 3]));
    assert_eq!(replaced, Some(vec![9]));
    assert_eq!(missing, None);
    assert_eq!(names, vec!["other".to_string(), "vault".to_string()]);
}

#[test_case("settings", "{not json" ; "undecodable settings")]
#[test_case("kind", "gone" ; "unknown enum name")]
#[tokio::test]
async fn corrupt_row_is_internal_error(column: &'static str, value: &'static str) {
    let (_dir, _db_file, store) = open();
    let p = profile_named("work");
    let profile_id = p.id;

    let result = store
        .run(move |c| {
            profile::insert(c, &p)?;
            c.execute(&format!("UPDATE profile SET {column} = ?1"), [value])
                .map_err(|e| Error::Internal {
                    message: e.to_string(),
                })?;

            profile::get(c, &profile_id)
        })
        .await;

    let Err(Error::Internal { message }) = result else {
        panic!("expected an internal error, got {result:?}");
    };
    assert!(
        message.starts_with(&format!("corrupt profile row {profile_id}")),
        "{message}"
    );
}

#[tokio::test]
async fn builtin_template_upsert_updates_in_place() {
    let (_dir, _db_file, store) = open();
    let changed = ProfileSettings {
        display: Display::Visible,
        ..ProfileSettings::default()
    };
    let expected = changed.clone();

    let (first, second, count) = store
        .run(move |c| {
            template::upsert_builtin(c, "default", "first", &ProfileSettings::default(), NOW)?;
            let first = template::find(c, "default")?;
            template::upsert_builtin(c, "default", "second", &changed, NOW + 1)?;
            let second = template::find(c, "default")?;
            let (all, _) = template::list(c, &Page::new(None, None))?;

            Ok((first, second, all.len()))
        })
        .await
        .unwrap();

    assert!(first.builtin);
    assert_eq!(second.id, first.id);
    assert_eq!(second.settings, expected);
    assert_eq!(first.description.as_deref(), Some("first"));
    assert_eq!(second.description.as_deref(), Some("second"));
    assert_eq!(count, 1);
}

#[tokio::test]
async fn missing_rows_are_not_found() {
    let store = Store::open_in_memory().unwrap();

    let result = store.run(|c| profile::get(c, &ProfileId::new())).await;

    assert!(matches!(result, Err(Error::NotFound { .. })));
}

const SEQ_RESERVATION: i64 = 50;
const DAY_MS: i64 = 86_400_000;

#[tokio::test]
async fn seq_high_water_survives_deleted_rows_and_reservations() {
    let (_dir, _db_file, store) = open();

    let (after_delete, after_reserve, after_smaller_reserve) = store
        .run(|c| {
            event::insert(
                c,
                &event_at(1, instance_subject(&InstanceId::new()), json!({})),
            )?;
            event::insert(
                c,
                &event_at(2, instance_subject(&InstanceId::new()), json!({})),
            )?;
            brr_store::retention::delete_events_before(c, i64::MAX)?;
            let after_delete = event::max_seq(c)?;

            event::reserve_seq(c, SEQ_RESERVATION)?;
            let after_reserve = event::max_seq(c)?;
            event::reserve_seq(c, 3)?;

            Ok((after_delete, after_reserve, event::max_seq(c)?))
        })
        .await
        .unwrap();

    assert_eq!(after_delete, 2);
    assert_eq!(after_reserve, SEQ_RESERVATION);
    assert_eq!(after_smaller_reserve, SEQ_RESERVATION);
}

#[tokio::test]
async fn reservation_on_empty_log_seeds_the_high_water() {
    let (_dir, _db_file, store) = open();

    let high_water = store
        .run(|c| {
            assert_eq!(event::max_seq(c)?, 0);
            event::reserve_seq(c, SEQ_RESERVATION)?;

            event::max_seq(c)
        })
        .await
        .unwrap();

    assert_eq!(high_water, SEQ_RESERVATION);
}

#[tokio::test]
async fn instance_event_filter_uses_indexes_and_matches_both_branches() {
    let (_dir, _db_file, store) = open();
    let target = InstanceId::new();
    let other = InstanceId::new();
    let mut tab_event = event_at(
        2,
        Subject {
            kind: SubjectType::Tab,
            id: TabId::new().to_string(),
        },
        json!({ "instance_id": target.to_string() }),
    );
    tab_event.kind = "tab.opened".to_string();
    let events = vec![
        event_at(1, instance_subject(&target), json!({})),
        tab_event,
        event_at(3, instance_subject(&other), json!({})),
    ];
    let target_id = target.to_string();

    let (found, plan) = store
        .run(move |c| {
            for e in &events {
                event::insert(c, e)?;
            }
            let (found, _) = event::list(
                c,
                &event::Query {
                    instance_id: Some(target),
                    ..query_for(Page::new(None, None))
                },
            )?;
            let plan: Vec<String> = c
                .prepare(
                    "EXPLAIN QUERY PLAN SELECT seq FROM event WHERE seq IN (SELECT seq FROM event WHERE subject_type = 'instance' AND subject_id = ? UNION SELECT seq FROM event WHERE json_extract(data, '$.instance_id') = ?)",
                )
                .unwrap()
                .query_map([&target_id, &target_id], |row| row.get::<_, String>(3))
                .unwrap()
                .map(Result::unwrap)
                .collect();

            Ok((found, plan))
        })
        .await
        .unwrap();

    assert_eq!(seqs(&found), vec![1, 2]);
    assert!(
        plan.iter().any(|step| step.contains("event_instance")),
        "{plan:?}"
    );
    assert!(
        plan.iter().any(|step| step.contains("event_subject")),
        "{plan:?}"
    );
}

#[tokio::test]
async fn open_sweeps_old_final_rows_only() {
    let (_dir, db_file, store) = open();
    let old = brr_core::now_ms() - brr_store::retention::FINISHED_ROWS_MAX_AGE_MS - DAY_MS;
    let recent = brr_core::now_ms() - DAY_MS;
    let mut old_stopped = instance_for(None, InstanceState::Stopped);
    old_stopped.ended_at = Some(old);
    let mut recent_stopped = instance_for(None, InstanceState::Stopped);
    recent_stopped.ended_at = Some(recent);
    let running = instance_for(None, InstanceState::Running);
    let mut old_promoting = instance_for(None, InstanceState::Stopped);
    old_promoting.ended_at = Some(old);
    old_promoting.promote_to_name = Some("kept".to_string());
    let old_lease = lease_on(None, old_stopped.id);
    let ids = (
        old_stopped.id,
        recent_stopped.id,
        running.id,
        old_promoting.id,
    );
    let (old_lease_id, client_id) = (old_lease.id, ClientId::parse("old-client").unwrap());
    let old_client = Client {
        id: client_id.clone(),
        kind: ClientKind::Rest,
        label: "old".to_string(),
        current_tab_id: None,
        created_at: old,
        ended_at: None,
    };

    store
        .run(move |c| {
            for i in [&old_stopped, &recent_stopped, &running, &old_promoting] {
                instance::insert(c, i)?;
            }
            lease::insert(c, &old_lease)?;
            lease::release(c, &old_lease_id, ReleaseReason::Released, old)?;
            client::upsert(c, &old_client)?;
            client::end(c, &client_id, old)
        })
        .await
        .unwrap();
    drop(store);

    let reopened = Store::open(&db_file).unwrap();
    let (remaining, leases, clients) = reopened
        .run(|c| {
            let remaining = instance::all_ids(c)?;
            let count = |table: &str| -> i64 {
                c.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
            };

            Ok((remaining, count("lease"), count("client")))
        })
        .await
        .unwrap();

    assert!(!remaining.contains(&ids.0));
    assert!(remaining.contains(&ids.1) && remaining.contains(&ids.2));
    assert!(
        remaining.contains(&ids.3),
        "a pending promotion outlives the sweep"
    );
    assert_eq!((leases, clients), (0, 0));
}
