use std::path::PathBuf;

use brr_core::{
    Artifact, ArtifactId, ArtifactKind, BackendKind, CaptchaResult, CaptchaSolve, CaptchaSolveId,
    CaptchaType, Engine, Error, Event, Extension, ExtensionId, ExtensionSource, ExtensionVersion,
    Page, ProfileId, Subject, SubjectType,
};
use brr_store::{
    Connection, Store, artifact, artifact::Filter, captcha_solve, event, extension, retention,
};
use serde_json::json;

#[allow(clippy::expect_used)]
fn artifact_at(id: &str, kind: ArtifactKind, profile_id: ProfileId, t: i64, size: u64) -> Artifact {
    Artifact {
        id: id.parse::<ArtifactId>().expect("valid ulid in test"),
        public_id: format!("{t:0>32}"),
        kind,
        profile_id: Some(profile_id),
        instance_id: None,
        tab_id: None,
        mime: "image/png".to_string(),
        size_bytes: size,
        backend: BackendKind::Local,
        location: format!("artifacts/{t}/shot.png"),
        sha256: "00".repeat(32),
        created_at: t,
        filename: "shot.png".to_string(),
    }
}

fn seeded(c: &Connection) -> Result<(ProfileId, Vec<Artifact>), Error> {
    let (p, q) = (ProfileId::new(), ProfileId::new());
    let rows = vec![
        artifact_at(
            "01ARZ3NDEKTSV4RRFFQ69G5FA1",
            ArtifactKind::Screenshot,
            p,
            1,
            100,
        ),
        artifact_at("01ARZ3NDEKTSV4RRFFQ69G5FA2", ArtifactKind::Pdf, p, 2, 200),
        artifact_at(
            "01ARZ3NDEKTSV4RRFFQ69G5FA3",
            ArtifactKind::Screenshot,
            q,
            3,
            400,
        ),
    ];

    for a in &rows {
        artifact::insert(c, a)?;
    }

    Ok((p, rows))
}

#[tokio::test]
async fn list_filters_and_pages() -> Result<(), Error> {
    let store = Store::open_in_memory()?;

    store
        .run(|c| {
            let (p, rows) = seeded(c)?;
            let page = Page::new(None, None);

            let filter = Filter {
                kind: Some(ArtifactKind::Screenshot),
                ..Filter::default()
            };
            let (found, next) = artifact::list(c, &filter, &page)?;
            assert_eq!(found, vec![rows[2].clone(), rows[0].clone()]);
            assert_eq!(next, None);

            let filter = Filter {
                profile_id: Some(p),
                ..Filter::default()
            };
            let (first, cursor) = artifact::list(c, &filter, &Page::new(Some(1), None))?;
            assert_eq!(first, vec![rows[1].clone()]);

            let (second, _) = artifact::list(c, &filter, &Page::new(Some(1), cursor))?;
            assert_eq!(second, vec![rows[0].clone()]);

            Ok(())
        })
        .await
}

#[tokio::test]
async fn usage_and_public_id() -> Result<(), Error> {
    let store = Store::open_in_memory()?;

    store
        .run(|c| {
            let (_, rows) = seeded(c)?;

            assert_eq!(
                artifact::usage_by_kind(c)?,
                vec![
                    (ArtifactKind::Pdf, 1, 200),
                    (ArtifactKind::Screenshot, 2, 500)
                ]
            );
            assert_eq!(artifact::by_public_id(c, &rows[1].public_id)?, rows[1]);
            assert!(matches!(
                artifact::by_public_id(c, &rows[1].public_id[..31]),
                Err(Error::NotFound { .. })
            ));
            let mut paged: Vec<Artifact> = Vec::new();
            loop {
                let after = paged.last().map(|last| (last.created_at, last.id));
                let page = artifact::oldest_page(c, after, 2)?;
                if page.is_empty() {
                    break;
                }
                paged.extend(page);
            }
            assert_eq!(paged, rows);

            artifact::delete(c, &rows[0].id)?;
            assert!(matches!(
                artifact::get(c, &rows[0].id),
                Err(Error::NotFound { .. })
            ));

            Ok(())
        })
        .await
}

#[tokio::test]
async fn events_by_age() -> Result<(), Error> {
    let store = Store::open_in_memory()?;

    store
        .run(|c| {
            for (seq, ts) in [(1, 30), (2, 10), (3, 20)] {
                event::insert(
                    c,
                    &Event {
                        seq,
                        ts,
                        kind: "instance.state".to_string(),
                        subject: Subject {
                            kind: SubjectType::Instance,
                            id: "x".to_string(),
                        },
                        actor_client_id: None,
                        data: json!({}),
                    },
                )?;
            }

            assert_eq!(retention::events_before(c, 25)?, 2);
            assert_eq!(retention::delete_events_before(c, 25)?, 2);
            assert_eq!(retention::events_before(c, 100)?, 1);
            assert_eq!(retention::captcha_solves_before(c, 100)?, 0);

            Ok(())
        })
        .await
}

#[tokio::test]
async fn retention_extension_dirs_and_captcha_delete() -> Result<(), Error> {
    let store = Store::open_in_memory()?;

    store
        .run(|c| {
            let ext = Extension {
                id: ExtensionId::new(),
                engine: Engine::Chromium,
                source: ExtensionSource::Cws,
                source_ref: "abc".to_string(),
                name: "abc".to_string(),
                versions: Vec::new(),
            };
            extension::insert(c, &ext)?;

            for v in ["1.0.0", "2.0.0"] {
                let version = ExtensionVersion {
                    version: v.to_string(),
                    path: PathBuf::from(format!("/ext/{v}")),
                    size_bytes: 10,
                    fetched_at: 0,
                    last_referenced_at: None,
                };
                extension::add_version(c, &ext.id, &version)?;
            }

            assert_eq!(
                retention::extension_dirs(c)?,
                vec![
                    (ext.id, "abc".to_string(), PathBuf::from("/ext/1.0.0")),
                    (ext.id, "abc".to_string(), PathBuf::from("/ext/2.0.0")),
                ]
            );

            for ts in [10, 20, 30] {
                captcha_solve::insert(
                    c,
                    &CaptchaSolve {
                        id: CaptchaSolveId::new(),
                        ts,
                        profile_id: None,
                        tab_id: None,
                        site: "https://example.com".to_string(),
                        captcha_type: CaptchaType::Turnstile,
                        provider: "2captcha".to_string(),
                        cost_usd_micros: 1,
                        duration_ms: 5,
                        result: CaptchaResult::Solved,
                        error: None,
                    },
                )?;
            }

            assert_eq!(retention::captcha_solves_before(c, 25)?, 2);
            assert_eq!(retention::delete_captcha_solves_before(c, 25)?, 2);
            assert_eq!(captcha_solve::spent_since(c, 0)?, (1, 1));

            Ok(())
        })
        .await
}
