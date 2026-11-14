use std::time::Duration;

use brr_proto::spawn;
use serde_json::json;
use tokio::process::Command;

const TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn events_arrive_then_channel_closes_on_exit() {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(r#"printf '{"method":"X.y","params":{"a":1}}\0' >&4; exit 0"#);

    let mut spawned = spawn(command).unwrap();

    let event = tokio::time::timeout(TIMEOUT, spawned.events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.method, "X.y");
    assert_eq!(event.params, json!({"a": 1}));
    assert!(event.session_id.is_none());

    let end = tokio::time::timeout(TIMEOUT, spawned.events.recv())
        .await
        .unwrap();
    assert!(end.is_none());
    spawned.child.wait().await.unwrap();
}

#[tokio::test]
async fn call_round_trips_over_fd3_and_fd4() {
    // The first call id is always 1; reading one byte of the request is enough to wait for it.
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(
        r#"dd bs=1 count=1 <&3 >/dev/null 2>&1; printf '{"id":1,"result":{"ok":true}}\0' >&4"#,
    );

    let spawned = spawn(command).unwrap();

    let result = tokio::time::timeout(TIMEOUT, spawned.mux.call("A.b", json!({}), None))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, json!({"ok": true}));
}
