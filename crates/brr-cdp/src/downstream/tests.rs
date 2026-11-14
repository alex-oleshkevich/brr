use std::path::PathBuf;
use std::time::Duration;

use brr_proto::spawn;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::mpsc::Receiver;

use super::{DOWNSTREAM_QUEUE, Downstream, Outbound};
use crate::tests::{Peer, TIMEOUT, connect};
use crate::{Cdp, CdpEvent};

const RESULT_BYTES: usize = 2 * 1024 * 1024;

async fn attach(cdp: &Cdp, peer: &mut Peer, session: &str) -> (Downstream, Receiver<Outbound>) {
    let attach = tokio::spawn({
        let cdp = cdp.clone();
        async move { cdp.attach_downstream().await.unwrap() }
    });
    let request = peer.read().await;
    assert_eq!(request["method"], "Target.attachToBrowserTarget");
    peer.send(json!({ "id": request["id"], "result": { "sessionId": session } }))
        .await;

    attach.await.unwrap()
}

async fn next(rx: &mut Receiver<Outbound>) -> Value {
    match tokio::time::timeout(TIMEOUT, rx.recv()).await.unwrap() {
        Some(Outbound::Frame(text)) => serde_json::from_str(&text).unwrap(),
        other => panic!("expected a frame, got {other:?}"),
    }
}

#[tokio::test]
async fn root_commands_use_own_browser_session() {
    let (cdp, _events, mut peer) = connect(TIMEOUT);
    let (one, mut rx1) = attach(&cdp, &mut peer, "B1").await;
    let (two, mut rx2) = attach(&cdp, &mut peer, "B2").await;

    one.send(r#"{"id":1,"method":"Target.getTargets"}"#);
    let first = peer.read().await;
    assert_eq!(first["method"], "Target.getTargets");
    assert_eq!(first["sessionId"], "B1");

    two.send(r#"{"id":1,"method":"Target.getTargets"}"#);
    let second = peer.read().await;
    assert_eq!(second["sessionId"], "B2");
    assert_ne!(first["id"], second["id"]);

    peer.send(json!({ "id": second["id"], "result": { "targetInfos": [2] }, "sessionId": "B2" }))
        .await;
    peer.send(json!({ "id": first["id"], "result": { "targetInfos": [1] }, "sessionId": "B1" }))
        .await;

    assert_eq!(
        next(&mut rx1).await,
        json!({ "id": 1, "result": { "targetInfos": [1] } })
    );
    assert_eq!(
        next(&mut rx2).await,
        json!({ "id": 1, "result": { "targetInfos": [2] } })
    );
}

#[tokio::test]
async fn child_sessions_isolated() {
    let (cdp, _events, mut peer) = connect(TIMEOUT);
    let (one, mut rx1) = attach(&cdp, &mut peer, "B1").await;
    let (two, mut rx2) = attach(&cdp, &mut peer, "B2").await;

    peer.send(json!({
        "method": "Target.attachedToTarget",
        "sessionId": "B1",
        "params": { "sessionId": "C", "targetInfo": {}, "waitingForDebugger": true },
    }))
    .await;
    let attached = next(&mut rx1).await;
    assert_eq!(attached["method"], "Target.attachedToTarget");
    assert!(attached.get("sessionId").is_none());

    one.send(r#"{"id":2,"sessionId":"C","method":"Runtime.runIfWaitingForDebugger"}"#);
    let forwarded = peer.read().await;
    assert_eq!(forwarded["sessionId"], "C");
    assert_eq!(forwarded["method"], "Runtime.runIfWaitingForDebugger");
    peer.send(json!({ "id": forwarded["id"], "result": {}, "sessionId": "C" }))
        .await;
    assert_eq!(
        next(&mut rx1).await,
        json!({ "id": 2, "result": {}, "sessionId": "C" })
    );

    two.send(r#"{"id":3,"sessionId":"C","method":"Runtime.enable"}"#);
    assert_eq!(
        next(&mut rx2).await,
        json!({
            "id": 3,
            "error": { "code": -32001, "message": "Session with given id not found." },
            "sessionId": "C",
        })
    );

    peer.send(
        json!({ "method": "Page.loadEventFired", "sessionId": "C", "params": { "timestamp": 1 } }),
    )
    .await;
    assert_eq!(
        next(&mut rx1).await,
        json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1 }, "sessionId": "C" })
    );

    // Nothing reached Client 2 or the fake: the next frame the fake sees is this probe.
    one.send(r#"{"id":4,"method":"Probe"}"#);
    assert_eq!(peer.read().await["method"], "Probe");
    assert!(rx2.try_recv().is_err());
}

#[tokio::test]
async fn intercepted_methods() {
    let (cdp, _events, mut peer) = connect(TIMEOUT);
    let (one, mut rx1) = attach(&cdp, &mut peer, "B1").await;

    // C is a page session of Client 1; D is a browser-level session nested under it.
    for (parent, child) in [("B1", "C"), ("C", "D")] {
        peer.send(json!({
            "method": "Target.attachedToTarget",
            "sessionId": parent,
            "params": { "sessionId": child, "targetInfo": {}, "waitingForDebugger": false },
        }))
        .await;
        next(&mut rx1).await;
    }

    one.send(r#"{"id":1,"method":"Browser.close"}"#);
    assert_eq!(next(&mut rx1).await, json!({ "id": 1, "result": {} }));
    assert_eq!(rx1.recv().await, Some(Outbound::BrowserClose));

    for (id, session) in [(2, "C"), (3, "D")] {
        one.send(&format!(
            r#"{{"id":{id},"sessionId":"{session}","method":"Browser.close"}}"#
        ));
        assert_eq!(
            next(&mut rx1).await,
            json!({ "id": id, "result": {}, "sessionId": session })
        );
        assert_eq!(rx1.recv().await, Some(Outbound::BrowserClose));
    }

    one.send(r#"{"id":4,"method":"Browser.setDownloadBehavior","params":{"behavior":"deny"}}"#);
    assert_eq!(next(&mut rx1).await, json!({ "id": 4, "result": {} }));
    one.send(r#"{"id":5,"sessionId":"C","method":"Page.setDownloadBehavior","params":{"behavior":"allow"}}"#);
    assert_eq!(
        next(&mut rx1).await,
        json!({ "id": 5, "result": {}, "sessionId": "C" })
    );

    one.send(r#"{"id":6,"method":"Probe"}"#);
    assert_eq!(peer.read().await["method"], "Probe");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_precede_later_responses() {
    let (cdp, _events, mut peer) = connect(TIMEOUT);
    let (one, mut rx1) = attach(&cdp, &mut peer, "B1").await;

    for id in 1..=50_u64 {
        let session = format!("C{id}");
        one.send(&format!(
            r#"{{"id":{id},"method":"Target.attachToTarget"}}"#
        ));
        let request = peer.read().await;
        peer.send(json!({
            "method": "Target.attachedToTarget",
            "sessionId": "B1",
            "params": { "sessionId": session, "targetInfo": {}, "waitingForDebugger": false },
        }))
        .await;
        peer.send(json!({
            "id": request["id"],
            "result": { "sessionId": session },
            "sessionId": "B1",
        }))
        .await;

        assert_eq!(next(&mut rx1).await["method"], "Target.attachedToTarget");
        assert_eq!(next(&mut rx1).await["id"], id);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn responses_precede_later_events() {
    let (cdp, _events, mut peer) = connect(TIMEOUT);
    let (one, mut rx1) = attach(&cdp, &mut peer, "B1").await;

    for id in 1..=200_u64 {
        one.send(&format!(r#"{{"id":{id},"method":"Page.enable"}}"#));
        let request = peer.read().await;
        peer.send(json!({ "id": request["id"], "result": {}, "sessionId": "B1" }))
            .await;
        peer.send(
            json!({ "method": "Target.targetCreated", "sessionId": "B1", "params": { "n": id } }),
        )
        .await;

        assert_eq!(next(&mut rx1).await["id"], id);
        assert_eq!(next(&mut rx1).await["params"]["n"], id);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn commands_reach_the_browser_in_order() {
    let (cdp, _events, mut peer) = connect(TIMEOUT);
    let (one, _rx1) = attach(&cdp, &mut peer, "B1").await;

    for id in 1..=200_u64 {
        one.send(&format!(
            r#"{{"id":{id},"method":"Probe","params":{{"n":{id}}}}}"#
        ));
    }
    for id in 1..=200_u64 {
        assert_eq!(peer.read().await["params"]["n"], id);
    }
}

#[tokio::test]
async fn slow_client_dropped() {
    let (cdp, mut events, mut peer) = connect(TIMEOUT);
    let (_one, mut rx1) = attach(&cdp, &mut peer, "B1").await;

    for _ in 0..=DOWNSTREAM_QUEUE {
        peer.send(json!({ "method": "Target.targetCreated", "sessionId": "B1", "params": {} }))
            .await;
    }
    peer.send(json!({ "method": "Target.targetCreated", "sessionId": "B1", "params": {} }))
        .await;
    peer.send(json!({
        "method": "Target.targetDestroyed",
        "params": { "targetId": "T1" },
    }))
    .await;

    // The dropped Client's later frames must not leak into brr's decoder.
    let event = tokio::time::timeout(TIMEOUT, events.recv()).await.unwrap();
    assert!(matches!(event, Some(CdpEvent::TargetDestroyed { .. })));

    let mut received = 0;
    while rx1.recv().await.is_some() {
        received += 1;
    }
    assert_eq!(received, DOWNSTREAM_QUEUE);
}

#[allow(clippy::disallowed_macros)]
#[tokio::test]
async fn downstream_live() {
    let Some(chrome) = std::env::var_os("BRR_TEST_CHROME").map(PathBuf::from) else {
        eprintln!("skipped: BRR_TEST_CHROME is not set");
        return;
    };
    let profile = tempfile::tempdir().unwrap();
    let mut command = Command::new(chrome);
    command
        .args(["--headless", "--remote-debugging-pipe", "--no-first-run"])
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .kill_on_drop(true);
    let spawned = spawn(command).unwrap();
    let (cdp, mut events) = Cdp::start(spawned.mux, spawned.events);
    cdp.call(
        None,
        "Target.setDiscoverTargets",
        json!({ "discover": true }),
    )
    .await
    .unwrap();

    let (client, mut rx) = cdp.attach_downstream().await.unwrap();
    client.send(r#"{"id":1,"method":"Target.createTarget","params":{"url":"about:blank"}}"#);
    let target_id = loop {
        let frame = next(&mut rx).await;
        if frame["id"] == 1 {
            break frame["result"]["targetId"].as_str().unwrap().to_owned();
        }
    };
    client.send(r#"{"id":2,"method":"Target.getTargets"}"#);
    let listed = loop {
        let frame = next(&mut rx).await;
        if frame["id"] == 2 {
            break frame;
        }
    };
    let infos = listed["result"]["targetInfos"].as_array().unwrap();
    assert!(
        infos
            .iter()
            .any(|info| info["targetId"] == target_id.as_str())
    );

    let created = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(CdpEvent::TargetCreated(info)) = events.recv().await
                && info.target_id.0 == target_id
            {
                return;
            }
        }
    })
    .await;
    assert!(created.is_ok());
    client.close().await;
}

#[tokio::test]
async fn browser_exit_ends_clients() {
    let (cdp, _events, mut peer) = connect(TIMEOUT);
    let (_one, mut rx1) = attach(&cdp, &mut peer, "B1").await;

    drop(peer);

    assert!(
        tokio::time::timeout(TIMEOUT, rx1.recv())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn close_detaches_then_removes_routes() {
    let (cdp, mut events, mut peer) = connect(TIMEOUT);
    let (one, mut rx1) = attach(&cdp, &mut peer, "B1").await;

    let closing = tokio::spawn(one.close());
    let detach = peer.read().await;
    assert_eq!(detach["method"], "Target.detachFromTarget");
    assert_eq!(detach["params"]["sessionId"], "B1");
    peer.send(json!({ "method": "Target.targetCreated", "sessionId": "B1", "params": {} }))
        .await;
    peer.send(json!({ "id": detach["id"], "result": {} })).await;
    closing.await.unwrap();

    assert_eq!(next(&mut rx1).await["method"], "Target.targetCreated");
    assert!(
        tokio::time::timeout(TIMEOUT, rx1.recv())
            .await
            .unwrap()
            .is_none()
    );

    // Routes are gone: a later frame on B1 is brr's again.
    peer.send(json!({ "method": "Target.targetDestroyed", "sessionId": "B1", "params": { "targetId": "T1" } }))
        .await;
    assert!(matches!(
        tokio::time::timeout(TIMEOUT, events.recv()).await.unwrap(),
        Some(CdpEvent::TargetDestroyed { .. })
    ));
}

#[tokio::test]
async fn dropping_downstream_detaches_and_removes_routes() {
    let (cdp, mut events, mut peer) = connect(TIMEOUT);
    let (one, mut rx1) = attach(&cdp, &mut peer, "B1").await;

    drop(one);
    let detach = peer.read().await;
    assert_eq!(detach["method"], "Target.detachFromTarget");
    assert_eq!(detach["params"]["sessionId"], "B1");
    peer.send(json!({ "id": detach["id"], "result": {} })).await;

    assert!(
        tokio::time::timeout(TIMEOUT, rx1.recv())
            .await
            .unwrap()
            .is_none()
    );
    peer.send(json!({ "method": "Target.targetDestroyed", "sessionId": "B1", "params": { "targetId": "T1" } }))
        .await;
    assert!(matches!(
        tokio::time::timeout(TIMEOUT, events.recv()).await.unwrap(),
        Some(CdpEvent::TargetDestroyed { .. })
    ));
}

#[tokio::test]
async fn large_results_are_forwarded_whole() {
    let (cdp, _events, mut peer) = connect(TIMEOUT);
    let (one, mut rx1) = attach(&cdp, &mut peer, "B1").await;
    let image = "x".repeat(RESULT_BYTES);

    one.send(&json!({ "id": 7, "method": "Page.captureScreenshot", "params": {} }).to_string());
    let request = peer.read().await;
    peer.send(json!({ "id": request["id"], "result": { "data": image } }))
        .await;

    let response = next(&mut rx1).await;
    assert_eq!(response["id"], 7);
    assert_eq!(response["result"]["data"], image);
}
