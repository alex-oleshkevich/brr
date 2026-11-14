use brr_core::{
    Backup, BackupId, CaptchaResult, CaptchaSolve, CaptchaSolveId, CaptchaType, CredentialId,
    CredentialSelectors, Error, Event, LastLogin, LoginCheck, LoginCheckId, LoginStatus, Page,
    Profile, ProfileId, ProfileKind, ProfileSettings, Subject, SubjectType,
};
use brr_store::{
    Connection, Store, backup, captcha_solve, credential, credential::CredentialRow, event,
    login_check, profile,
};
use serde_json::json;
use tempfile::TempDir;

const NOW: i64 = 1_700_000_000_000;

#[allow(clippy::unwrap_used)]
fn open() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("brr.db")).unwrap();

    (dir, store)
}

fn run<T: Send + 'static>(
    store: &Store,
    f: impl FnOnce(&mut Connection) -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| Error::Internal {
            message: e.to_string(),
        })?
        .block_on(store.run(f))
}

fn profile_named(name: &str) -> Profile {
    Profile {
        id: ProfileId::new(),
        name: name.to_string(),
        kind: ProfileKind::Persistent,
        template_id: None,
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

fn credential_at(site: &str) -> CredentialRow {
    CredentialRow {
        id: CredentialId::new(),
        site: site.to_string(),
        login_url: None,
        username_enc: b"user-secret".to_vec(),
        password_enc: b"hunter2-sealed".to_vec(),
        totp_enc: None,
        notes_enc: None,
        selectors: CredentialSelectors::default(),
        created_at: NOW,
        updated_at: NOW,
    }
}

fn check_for(profile_id: &ProfileId, site: &str, status: LoginStatus) -> LoginCheck {
    LoginCheck {
        id: LoginCheckId::new(),
        profile_id: *profile_id,
        site: site.to_string(),
        url: format!("https://{site}/"),
        expect_selector: None,
        expect_text: None,
        credential_id: None,
        auto_relogin: false,
        last_status: status,
        last_checked_at: None,
    }
}

fn solve_at(ts: i64, cost: u64) -> CaptchaSolve {
    CaptchaSolve {
        id: CaptchaSolveId::new(),
        ts,
        profile_id: None,
        tab_id: None,
        site: "https://example.com".to_string(),
        captcha_type: CaptchaType::Turnstile,
        provider: "2captcha".to_string(),
        cost_usd_micros: cost,
        duration_ms: 5,
        result: CaptchaResult::Solved,
        error: None,
    }
}

fn login_event(seq: i64, ts: i64, credential_id: &CredentialId, ok: bool) -> Event {
    Event {
        seq,
        ts,
        kind: "login.performed".to_string(),
        subject: Subject {
            kind: SubjectType::Tab,
            id: "tab".to_string(),
        },
        actor_client_id: None,
        data: json!({"credential_id": credential_id, "ok": ok}),
    }
}

#[test]
fn credential_links() {
    let (_dir, store) = open();

    run(&store, |c| {
        let (a, b) = (profile_named("a"), profile_named("b"));
        profile::insert(c, &a)?;
        profile::insert(c, &b)?;

        let cred = credential_at("example.com");
        credential::insert(c, &cred)?;
        let tx = c.transaction().map_err(|e| Error::Internal {
            message: e.to_string(),
        })?;
        credential::set_profiles(&tx, &cred.id, &[a.id, b.id])?;
        tx.commit().map_err(|e| Error::Internal {
            message: e.to_string(),
        })?;

        let mut check = check_for(&a.id, "example.com", LoginStatus::Ok);
        check.credential_id = Some(cred.id);
        login_check::insert(c, &check)?;

        credential::set_profiles(c, &cred.id, &[b.id])?;
        assert_eq!(credential::profiles(c, &cred.id)?, vec![b.id]);
        assert!(!credential::is_linked(c, &cred.id, &a.id)?);
        assert!(credential::is_linked(c, &cred.id, &b.id)?);

        credential::delete(c, &cred.id)?;
        assert!(credential::profiles(c, &cred.id)?.is_empty());
        assert_eq!(login_check::get(c, &check.id)?.credential_id, None);
        assert!(matches!(
            credential::get(c, &cred.id),
            Err(Error::NotFound { .. })
        ));

        Ok(())
    })
    .expect("credential links");
}

#[test]
fn credential_crud_and_paging() {
    let (_dir, store) = open();

    run(&store, |c| {
        let rows: Vec<CredentialRow> = ["a.com", "b.com", "c.com"].map(credential_at).into();

        for row in &rows {
            credential::insert(c, row)?;
        }

        let mut changed = rows[0].clone();
        changed.site = "z.com".to_string();
        changed.selectors.username_selector = Some("#u".to_string());
        credential::update(c, &changed)?;
        assert_eq!(
            credential::get(c, &changed.id)?.selectors,
            changed.selectors
        );

        let (first, cursor) = credential::list(c, &Page::new(Some(2), None))?;
        assert_eq!(
            first.iter().map(|r| r.site.as_str()).collect::<Vec<_>>(),
            ["b.com", "c.com"]
        );

        let (rest, cursor) = credential::list(c, &Page::new(Some(2), cursor))?;
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].site, "z.com");
        assert_eq!(cursor, None);
        assert_eq!(credential::all(c)?.len(), 3);

        Ok(())
    })
    .expect("credential crud");
}

#[test]
fn credential_row_debug() {
    let row = credential_at("example.com");
    let printed = format!("{row:?}");

    assert!(printed.contains(&format!("{:?}", row.id)));
    assert!(printed.contains("example.com"));
    assert!(!printed.contains("hunter2"));
    // The sealed bytes as a list ("h", "u"); a bare "104" could appear inside the random id.
    assert!(!printed.contains("104, 117"));
}

#[test]
fn last_logins_newest() {
    let (_dir, store) = open();

    run(&store, |c| {
        let x = CredentialId::new();
        event::insert(c, &login_event(1, 1, &x, false))?;
        event::insert(c, &login_event(2, 2, &x, true))?;

        let logins = credential::last_logins(c)?;
        assert_eq!(logins.len(), 1);
        assert_eq!(logins.get(&x), Some(&LastLogin { ok: true, at: 2 }));

        Ok(())
    })
    .expect("last logins");
}

#[test]
fn profile_statuses() {
    let (_dir, store) = open();

    run(&store, |c| {
        let (p, q, none) = (
            profile_named("p"),
            profile_named("q"),
            profile_named("none"),
        );

        for prof in [&p, &q, &none] {
            profile::insert(c, prof)?;
        }

        for (prof, status) in [
            (&p, LoginStatus::Ok),
            (&p, LoginStatus::Failed),
            (&q, LoginStatus::Ok),
            (&q, LoginStatus::Unknown),
        ] {
            login_check::insert(c, &check_for(&prof.id, "example.com", status))?;
        }

        let statuses = login_check::profile_statuses(c)?;
        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses.get(&p.id), Some(&LoginStatus::Failed));
        assert_eq!(statuses.get(&q.id), Some(&LoginStatus::Unknown));
        assert_eq!(statuses.get(&none.id), None);

        Ok(())
    })
    .expect("profile statuses");
}

#[test]
fn login_check_crud() {
    let (_dir, store) = open();

    run(&store, |c| {
        let p = profile_named("p");
        profile::insert(c, &p)?;

        let (b, a) = (
            check_for(&p.id, "b.com", LoginStatus::Unknown),
            check_for(&p.id, "a.com", LoginStatus::Unknown),
        );
        login_check::insert(c, &b)?;
        login_check::insert(c, &a)?;
        login_check::set_status(c, &a.id, LoginStatus::Ok, NOW)?;

        let checks = login_check::for_profile(c, &p.id)?;
        assert_eq!(checks[0].id, a.id);
        assert_eq!(checks[0].last_status, LoginStatus::Ok);
        assert_eq!(checks[0].last_checked_at, Some(NOW));

        let mut edited = a.clone();
        edited.url = "https://a.com/home".to_string();
        login_check::update(c, &edited)?;
        assert_eq!(login_check::get(c, &a.id)?.last_status, LoginStatus::Ok);

        login_check::delete(c, &a.id)?;
        assert!(matches!(
            login_check::get(c, &a.id),
            Err(Error::NotFound { .. })
        ));

        Ok(())
    })
    .expect("login check crud");
}

#[test]
fn captcha_spend_and_paging() {
    let (_dir, store) = open();

    run(&store, |c| {
        for (ts, cost) in [(10, 1000), (20, 2000), (30, 3000)] {
            captcha_solve::insert(c, &solve_at(ts, cost))?;
        }

        assert_eq!(captcha_solve::spent_since(c, 15)?, (5000, 2));

        let (first, cursor) = captcha_solve::list(c, &Page::new(Some(2), None))?;
        assert_eq!(first.iter().map(|s| s.ts).collect::<Vec<_>>(), [30, 20]);

        let (rest, cursor) = captcha_solve::list(c, &Page::new(Some(2), cursor))?;
        assert_eq!(rest.iter().map(|s| s.ts).collect::<Vec<_>>(), [10]);
        assert_eq!(cursor, None);

        Ok(())
    })
    .expect("captcha spend");
}

#[test]
fn backups_newest_first() {
    let (_dir, store) = open();

    run(&store, |c| {
        let p = profile_named("p");
        profile::insert(c, &p)?;

        let make = |at| Backup {
            id: BackupId::new(),
            profile_id: p.id,
            profile_name: p.name.clone(),
            profile_deleted: false,
            backend: "local".to_string(),
            location: format!("backups/{at}"),
            size_bytes: 9,
            created_at: at,
        };
        let (old, new) = (make(1), make(2));
        backup::insert(c, &old)?;
        backup::insert(c, &new)?;

        assert_eq!(backup::get(c, &old.id)?, old);
        assert_eq!(
            backup::for_profile(c, &p.id)?,
            vec![new.clone(), old.clone()]
        );

        backup::delete(c, &new.id)?;
        assert_eq!(backup::for_profile(c, &p.id)?, vec![old]);

        Ok(())
    })
    .expect("backups");
}

#[test]
fn credential_paging_with_duplicate_site() {
    let (_dir, store) = open();

    run(&store, |c| {
        let mut rows = [
            credential_at("a.com"),
            credential_at("a.com"),
            credential_at("a.com"),
        ];
        rows.sort_by_key(|r| r.id);

        for row in &rows {
            credential::insert(c, row)?;
        }

        let mut seen = Vec::new();
        let mut cursor = None;

        loop {
            let (page, next) = credential::list(c, &Page::new(Some(1), cursor))?;
            seen.extend(page.iter().map(|r| r.id));
            cursor = next;

            if cursor.is_none() {
                break;
            }
        }

        assert_eq!(seen, rows.map(|r| r.id));

        Ok(())
    })
    .expect("duplicate site paging");
}
