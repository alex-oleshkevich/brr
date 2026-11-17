#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::{
    fs,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use axum::{Router, routing::get};
use brr_core::{
    AdmissionMode, ConflictReason, EndReason, Error, Event, IdleOverrides, InstanceId,
    InstanceState, Profile, ProxyChoice,
};
use brr_daemon::{
    App, ops,
    services::{instances::LaunchRequest, proxies},
};
use brr_store::{pool, proxy};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpSocket, TcpStream},
    time,
};

use common::{KillOnDrop, add_profile, boot, chrome, ctx, register_build, wait_for};

const USER: &str = "u";
const PASSWORD: &str = "secret-pw-1234";
const BASIC_AUTH: &str = "Basic dTpzZWNyZXQtcHctMTIzNA==";
const EXIT_IP: &str = "203.0.113.9";
const TRACE: &str = "ip=203.0.113.9\nloc=NL\n";
const OUTBOUND_FROM: &str = "127.0.0.2";
const CONNECT_RETRY: Duration = Duration::from_millis(50);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    let op = ops::find(name).unwrap();

    ops::dispatch(ctx(app), op, input).await
}

async fn serve(router: Router) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    addr
}

async fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.unwrap());
    }

    String::from_utf8(head).unwrap()
}

// An HTTP proxy that wants user u and connects out from `from`.
async fn upstream(from: &'static str) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let (client, _) = listener.accept().await.unwrap();
            tokio::spawn(serve_upstream(client, from));
        }
    });

    addr
}

async fn serve_upstream(mut client: TcpStream, from: &str) {
    let head = read_head(&mut client).await;
    let auth = head
        .lines()
        .find_map(|line| line.strip_prefix("Proxy-Authorization: "));
    if auth != Some(BASIC_AUTH) {
        client
            .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n")
            .await
            .ok();

        return;
    }

    let mut parts = head.lines().next().unwrap().split(' ');
    let (method, target) = (parts.next().unwrap(), parts.next().unwrap());
    let url = target.strip_prefix("http://");
    let authority = url.map_or(target, |url| url.split('/').next().unwrap());
    let socket = TcpSocket::new_v4().unwrap();
    socket
        .bind(SocketAddr::new(from.parse::<IpAddr>().unwrap(), 0))
        .unwrap();
    let Some(target_addr) = tokio::net::lookup_host(authority)
        .await
        .ok()
        .and_then(|mut found| found.find(SocketAddr::is_ipv4))
    else {
        client
            .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
            .await
            .ok();

        return;
    };
    let Ok(mut origin) = socket.connect(target_addr).await else {
        return;
    };

    if method == "CONNECT" {
        client.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
    } else {
        let path = &url.unwrap()[authority.len()..];
        origin
            .write_all(
                format!(
                    "{method} {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }
    copy_bidirectional(&mut client, &mut origin).await.ok();
}

async fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();

    listener.local_addr().unwrap().port()
}

async fn app_with_trace() -> (tempfile::TempDir, Arc<App>) {
    app_with_trace_and(&[]).await
}

async fn app_with_trace_and(env: &[(&str, &str)]) -> (tempfile::TempDir, Arc<App>) {
    let trace = serve(Router::new().route("/trace", get(|| async { TRACE }))).await;
    let url = format!("http://{trace}/trace");
    let mut all = vec![("BRR_PROXY__CHECK_URL", url.as_str())];
    all.extend_from_slice(env);
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &all)).await.unwrap();

    (dir, app)
}

async fn create_proxy(app: &Arc<App>, port: u16) -> Value {
    call(
        app,
        "proxy.create",
        json!({"scheme": "http", "host": "127.0.0.1", "port": port, "username": USER, "password": PASSWORD}),
    )
    .await
    .unwrap()
}

async fn events_of(app: &Arc<App>, kind: &str) -> Vec<Event> {
    let listed = call(app, "events.list", json!({"kinds": kind}))
        .await
        .unwrap();

    serde_json::from_value(listed["events"].clone()).unwrap()
}

async fn wait_health_events(app: &Arc<App>, count: usize) {
    wait_for("proxy.health events", async || {
        events_of(app, "proxy.health").await.len() >= count
    })
    .await;
}

async fn follower(app: &Arc<App>, name: &str, choice: Value) -> Profile {
    let created = call(
        app,
        "profile.create",
        json!({"name": name, "template": "chromium-headless"}),
    )
    .await
    .unwrap();
    let mut settings = created["settings"].clone();
    settings["proxy"] = choice;

    serde_json::from_value(
        call(
            app,
            "profile.update",
            json!({"profile": name, "settings": settings}),
        )
        .await
        .unwrap(),
    )
    .unwrap()
}

fn port_of(app: &Arc<App>, instance_id: &InstanceId) -> u16 {
    proxies::instance_upstream(app, instance_id).unwrap().port
}

#[tokio::test]
async fn create_checks_and_hides_password() {
    let (_dir, app) = app_with_trace().await;
    let upstream = upstream(OUTBOUND_FROM).await;

    let created = create_proxy(&app, upstream.port()).await;
    wait_health_events(&app, 1).await;

    assert_eq!(created["password"], json!({"set": true, "hint": "…1234"}));
    let health = &events_of(&app, "proxy.health").await[0].data;
    assert_eq!(health["healthy"], true);
    assert_eq!(health["exit_ip"], EXIT_IP);
    let listed = call(&app, "proxy.list", json!({})).await.unwrap();
    assert_eq!(listed["proxies"][0]["healthy"], true);
    assert!(!listed.to_string().contains(PASSWORD));
    assert!(!created.to_string().contains(PASSWORD));
    for kind in ["proxy.health"] {
        let events = serde_json::to_string(&events_of(&app, kind).await).unwrap();
        assert!(!events.contains(PASSWORD));
    }
    let proxy_id = serde_json::from_value(created["id"].clone()).unwrap();
    let sealed = app
        .store
        .run(move |c| proxy::password_enc(c, &proxy_id))
        .await
        .unwrap()
        .unwrap();
    assert!(
        !sealed
            .windows(PASSWORD.len())
            .any(|window| window == PASSWORD.as_bytes())
    );
}

#[tokio::test]
async fn failed_check_records_a_plain_reason() {
    let (_dir, app) = app_with_trace().await;

    let created = create_proxy(&app, closed_port().await).await;
    wait_health_events(&app, 1).await;

    let listed = call(&app, "proxy.list", json!({})).await.unwrap();
    let shown = &listed["proxies"][0];
    assert_eq!(shown["id"], created["id"]);
    assert_eq!(shown["healthy"], false);
    assert!(!shown["last_error"].as_str().unwrap().is_empty());
    assert!(shown["checked_at"].is_i64());
}

#[tokio::test]
async fn create_rejects_duplicates_and_bad_hosts() {
    let (_dir, app) = app_with_trace().await;
    create_proxy(&app, 9).await;

    let duplicate = call(
        &app,
        "proxy.create",
        json!({"scheme": "http", "host": "127.0.0.1", "port": 9, "username": USER}),
    )
    .await;
    let bad_host = call(
        &app,
        "proxy.create",
        json!({"scheme": "http", "host": "bad host", "port": 9}),
    )
    .await;

    match duplicate {
        Err(Error::Conflict {
            reason: ConflictReason::NameTaken,
            detail,
        }) => assert!(detail.unwrap().contains("127.0.0.1:9")),
        other => panic!("expected NameTaken, got {other:?}"),
    }
    assert!(matches!(
        bad_host,
        Err(Error::InvalidArgument { field, .. }) if field == "host"
    ));
}

#[tokio::test]
async fn import_counts_duplicates_and_bad_lines() {
    let (_dir, app) = app_with_trace().await;

    let first = call(
        &app,
        "proxy.import",
        json!({"scheme": "http", "text": "a.example:8080\nbad\nb.example:81:u:pw\n"}),
    )
    .await
    .unwrap();
    let again = call(
        &app,
        "proxy.import",
        json!({"scheme": "http", "text": "a.example:8080"}),
    )
    .await
    .unwrap();

    assert_eq!(
        first,
        json!({"imported": 2, "duplicates": 0, "invalid_lines": [2]})
    );
    assert_eq!(
        again,
        json!({"imported": 0, "duplicates": 1, "invalid_lines": []})
    );
}

#[tokio::test]
async fn import_accepts_url_and_colon_forms() {
    let (_dir, app) = app_with_trace().await;

    let out = call(
        &app,
        "proxy.import",
        json!({
            "scheme": "http",
            "text": "socks5://u:p@a.example:1080\nb.example:81:u:p\nc.example:82\nftp://d.example:1\n",
        }),
    )
    .await
    .unwrap();

    assert_eq!(
        out,
        json!({"imported": 3, "duplicates": 0, "invalid_lines": [4]})
    );
}

#[tokio::test]
async fn pool_selection() {
    let (_dir, app) = app_with_trace().await;
    let live_addr = upstream(OUTBOUND_FROM).await;
    let other_addr = upstream(OUTBOUND_FROM).await;
    let dead = create_proxy(&app, closed_port().await).await;
    let live = create_proxy(&app, live_addr.port()).await;
    let other = create_proxy(&app, other_addr.port()).await;
    let made = |name: &str, mode: &str, ids: Vec<&Value>| {
        call(
            &app,
            "pool.create",
            json!({"name": name, "mode": mode, "proxy_ids": ids}),
        )
    };
    let sticky = made("sticky", "sticky", vec![&dead["id"], &live["id"]])
        .await
        .unwrap();
    let broken = made("broken", "sticky", vec![&dead["id"]]).await.unwrap();
    let rotating = made("rotating", "rotate", vec![&live["id"], &other["id"]])
        .await
        .unwrap();
    let sticky_profile = follower(&app, "sticky-user", json!({"pool_id": sticky["id"]})).await;
    let broken_profile = follower(&app, "broken-user", json!({"pool_id": broken["id"]})).await;
    let rotating_profile =
        follower(&app, "rotating-user", json!({"pool_id": rotating["id"]})).await;

    let (first, second) = (InstanceId::new(), InstanceId::new());
    proxies::for_launch(&app, &first, &sticky_profile, false)
        .await
        .unwrap()
        .unwrap();
    proxies::for_launch(&app, &second, &sticky_profile, false)
        .await
        .unwrap()
        .unwrap();
    let held = {
        let (pool_id, profile_id) = (
            serde_json::from_value(sticky["id"].clone()).unwrap(),
            sticky_profile.id,
        );
        app.store
            .run(move |c| pool::sticky(c, &pool_id, &profile_id))
            .await
            .unwrap()
    };
    let none = proxies::for_launch(&app, &InstanceId::new(), &broken_profile, false).await;

    assert_eq!(port_of(&app, &first), live_addr.port());
    assert_eq!(port_of(&app, &second), live_addr.port());
    assert_eq!(
        held,
        Some(serde_json::from_value(live["id"].clone()).unwrap())
    );
    assert_eq!(none.err(), Some(Error::ProxyUnhealthy));

    let ids: Vec<InstanceId> = (0..4).map(|_| InstanceId::new()).collect();
    for id in &ids {
        proxies::for_launch(&app, id, &rotating_profile, false)
            .await
            .unwrap();
    }
    let ports: Vec<u16> = ids.iter().map(|id| port_of(&app, id)).collect();
    assert_ne!(ports[0], ports[1]);
    assert_eq!(ports[0], ports[2]);
    assert_eq!(ports[1], ports[3]);
}

#[tokio::test]
async fn release_closes_the_forwarder_port() {
    let (_dir, app) = app_with_trace().await;
    let upstream = upstream(OUTBOUND_FROM).await;
    let created = create_proxy(&app, upstream.port()).await;
    let profile = follower(&app, "single", json!({"proxy_id": created["id"]})).await;
    let instance_id = InstanceId::new();

    let url = proxies::for_launch(&app, &instance_id, &profile, false)
        .await
        .unwrap()
        .unwrap();
    let addr = url.strip_prefix("http://").unwrap().to_owned();
    TcpStream::connect(&addr).await.unwrap();
    proxies::release(&app, &instance_id);

    time::timeout(CLOSE_TIMEOUT, async {
        while TcpStream::connect(&addr).await.is_ok() {
            time::sleep(CONNECT_RETRY).await;
        }
    })
    .await
    .unwrap();
    assert!(proxies::instance_upstream(&app, &instance_id).is_none());
}

#[tokio::test]
async fn delete_and_validate() {
    let (_dir, app) = app_with_trace().await;
    let created = create_proxy(&app, 9).await;
    follower(&app, "user", json!({"proxy_id": created["id"]})).await;

    let deleted = call(&app, "proxy.delete", json!({"proxy_id": created["id"]})).await;
    let unknown = call(
        &app,
        "profile.update",
        json!({"profile": "user", "settings": {
            "build": {"channel": "cft:stable"},
            "display": "headless",
            "proxy": {"proxy_id": brr_core::ProxyId::new()},
        }}),
    )
    .await;

    match deleted {
        Err(Error::Conflict {
            reason: ConflictReason::Referenced,
            detail,
        }) => assert!(detail.unwrap().contains("user")),
        other => panic!("expected Referenced, got {other:?}"),
    }
    assert!(matches!(
        unknown,
        Err(Error::InvalidArgument { field, .. }) if field == "settings.proxy"
    ));
}

#[tokio::test]
async fn launch_through_upstream() {
    let Some(chrome) = chrome() else {
        return;
    };
    let (_dir, app) = app_with_trace().await;
    let build_id = register_build(&app, &chrome).await;
    let upstream = upstream(OUTBOUND_FROM).await;
    let created = create_proxy(&app, upstream.port()).await;
    let origin = origin_with_peer_title().await;

    let mut profile = add_profile(&app, build_id, "proxied", false, IdleOverrides::default()).await;
    profile.settings.proxy = Some(ProxyChoice::ProxyId(
        serde_json::from_value(created["id"].clone()).unwrap(),
    ));
    let stored = profile.clone();
    app.store
        .run(move |c| brr_store::profile::update(c, &stored))
        .await
        .unwrap();

    let (running, _) = app
        .instances
        .launch(
            &app,
            LaunchRequest {
                profile,
                display: None,
                priority: 0,
                admission: AdmissionMode::Queue,
                force: false,
            },
        )
        .await
        .unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    let tab = app
        .instances
        .open_tab(&running.id, Some(format!("http://{origin}/")))
        .await
        .unwrap();
    wait_for("the origin's view of the peer", async || {
        let view = app.instances.view(&running.id).unwrap();
        view.tabs
            .iter()
            .any(|t| t.id == tab.id && t.title == OUTBOUND_FROM)
    })
    .await;
    let forwarder = chrome_proxy_server(running.pid.unwrap());

    app.instances
        .stop(&app, &running.id, EndReason::Stop)
        .await
        .unwrap();

    time::timeout(CLOSE_TIMEOUT, async {
        while TcpStream::connect(&forwarder).await.is_ok() {
            time::sleep(CONNECT_RETRY).await;
        }
    })
    .await
    .unwrap();
}

async fn origin_with_peer_title() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let (mut client, peer) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let body = format!("<title>{}</title>", peer.ip());
                if read_head_lossy(&mut client).await {
                    client
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                            .as_bytes(),
                        )
                        .await
                        .ok();
                }
                client.shutdown().await.ok();
            });
        }
    });

    addr
}

async fn read_head_lossy(stream: &mut TcpStream) -> bool {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read_u8().await {
            Ok(byte) => head.push(byte),
            Err(_) => return false,
        }
    }

    true
}

fn chrome_proxy_server(pid: u32) -> String {
    let cmdline = fs::read(format!("/proc/{pid}/cmdline")).unwrap();

    String::from_utf8_lossy(&cmdline)
        .split(['\0', ' '])
        .find_map(|arg| arg.strip_prefix("--proxy-server=http://"))
        .unwrap_or_else(|| panic!("no proxy flag in {cmdline:?}"))
        .to_owned()
}

#[tokio::test]
async fn crash_with_auto_restart_keeps_the_forwarder() {
    let Some(chrome) = chrome() else {
        return;
    };
    let (_dir, app) = app_with_trace_and(&[("BRR_INSTANCE__AUTO_RESTART", "true")]).await;
    let build_id = register_build(&app, &chrome).await;
    let upstream = upstream(OUTBOUND_FROM).await;
    let created = create_proxy(&app, upstream.port()).await;
    let mut profile = add_profile(
        &app,
        build_id,
        "restarting",
        false,
        IdleOverrides::default(),
    )
    .await;
    profile.settings.proxy = Some(ProxyChoice::ProxyId(
        serde_json::from_value(created["id"].clone()).unwrap(),
    ));
    let (running, _) = app
        .instances
        .launch(
            &app,
            LaunchRequest {
                profile,
                display: None,
                priority: 0,
                admission: AdmissionMode::Queue,
                force: false,
            },
        )
        .await
        .unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    let first_pid = running.pid.unwrap();
    let forwarder = chrome_proxy_server(first_pid);

    kill(
        Pid::from_raw(i32::try_from(first_pid).unwrap()),
        Signal::SIGKILL,
    )
    .unwrap();
    wait_for("the restarted browser", async || {
        let view = app.instances.view(&running.id).unwrap();

        view.instance.restart_count == 1
            && view.instance.state == InstanceState::Running
            && view.instance.pid != Some(first_pid)
    })
    .await;

    assert!(proxies::instance_upstream(&app, &running.id).is_some());
    TcpStream::connect(&forwarder).await.unwrap();
    app.instances
        .stop(&app, &running.id, EndReason::Stop)
        .await
        .unwrap();
    assert!(proxies::instance_upstream(&app, &running.id).is_none());
}
