use axum::{
    body::to_bytes,
    response::{
        IntoResponse,
        sse::{Event as SseEvent, Sse},
    },
};
use brr_core::{Event, Subject, SubjectType};
use futures::stream;
use serde_json::json;

use super::{FRAME_END, Frame, LAGGED, parse_frame};

async fn server_frames(events: Vec<SseEvent>) -> Vec<String> {
    let body = Sse::new(stream::iter(
        events.into_iter().map(Ok::<_, std::convert::Infallible>),
    ))
    .into_response()
    .into_body();
    let bytes = to_bytes(body, usize::MAX).await.unwrap();

    String::from_utf8(bytes.to_vec())
        .unwrap()
        .split(std::str::from_utf8(FRAME_END).unwrap())
        .filter(|frame| !frame.is_empty())
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn the_servers_lagged_frame_is_recognised() {
    let frames = server_frames(vec![SseEvent::default().event(LAGGED).data(LAGGED)]).await;

    assert_eq!(frames.len(), 1);
    assert!(matches!(parse_frame(&frames[0]), Frame::Lagged));
}

#[tokio::test]
async fn the_servers_event_frame_is_parsed_with_its_data_space_trimmed() {
    let sent = Event {
        seq: 7,
        ts: 1,
        kind: "tab.opened".to_string(),
        subject: Subject {
            kind: SubjectType::Tab,
            id: "t".to_string(),
        },
        actor_client_id: None,
        data: json!({}),
    };
    let json = serde_json::to_string(&sent).unwrap();
    let frames = server_frames(vec![
        SseEvent::default().id("7").event("tab.opened").data(json),
    ])
    .await;

    match parse_frame(&frames[0]) {
        Frame::Event(event) => assert_eq!(event, sent),
        _ => panic!("expected an event"),
    }
}

#[tokio::test]
async fn a_keep_alive_comment_is_not_an_event() {
    assert!(matches!(parse_frame(": connected"), Frame::Other));
}
