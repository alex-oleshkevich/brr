use tokio::sync::mpsc;

use test_case::test_case;

use super::{Frame, WRITE_TIMEOUT, deliver, frame, start_after};

fn sample() -> axum::response::sse::Event {
    frame(1, "tab.opened", "{}")
}

#[tokio::test(start_paused = true)]
async fn deliver_gives_up_on_a_reader_that_stopped_draining() {
    let (tx, mut rx) = mpsc::channel::<Frame>(1);

    assert!(deliver(&tx, sample()).await);

    let started = tokio::time::Instant::now();
    assert!(!deliver(&tx, sample()).await);
    assert!(started.elapsed() >= WRITE_TIMEOUT);

    // The reader is still attached: it only drains what was queued before the cut-off.
    assert!(rx.recv().await.is_some());
}

#[tokio::test(start_paused = true)]
async fn deliver_stops_when_the_reader_is_gone() {
    let (tx, rx) = mpsc::channel::<Frame>(1);
    drop(rx);

    assert!(!deliver(&tx, sample()).await);
}

#[tokio::test(start_paused = true)]
async fn deliver_waits_for_room_so_the_lagged_marker_is_not_dropped() {
    let (tx, mut rx) = mpsc::channel::<Frame>(1);
    assert!(deliver(&tx, sample()).await);

    let reader = tokio::spawn(async move {
        tokio::time::sleep(WRITE_TIMEOUT / 2).await;
        rx.recv().await;
        rx.recv().await
    });

    assert!(deliver(&tx, sample()).await);
    assert!(reader.await.unwrap().is_some());
}

#[test_case(Some(3), 10 => 3; "a cursor from this run resumes after itself")]
#[test_case(Some(99), 10 => 10; "a cursor above anything issued counts as now")]
#[test_case(None, 10 => 10; "no cursor starts at now")]
fn start_after_clamps_to_the_last_seq(resume: Option<i64>, last_seq: i64) -> i64 {
    start_after(resume, last_seq)
}
