use brr_cdp::SessionId;
use brr_core::Error;
use serde_json::json;
use test_case::test_case;

use super::frames::FrameTree;
use super::{Target, UTILITY_WORLD, UploadFile, parse_ref};

const MAIN: &str = "main";
const CHILD: &str = "child";

fn session() -> SessionId {
    SessionId("s1".to_owned())
}

fn tree_with_child() -> FrameTree {
    let mut tree = FrameTree::default();
    tree.seed(
        &session(),
        &json!({ "frame": { "id": MAIN }, "childFrames": [{ "frame": { "id": CHILD, "parentId": MAIN } }] }),
    );

    tree
}

#[test]
fn frame_tree_seeds_unique_increasing_seqs() {
    let tree = tree_with_child();

    assert_eq!(tree.main, MAIN);
    assert_eq!(tree.frames[MAIN].frame_seq, 0);
    assert_eq!(tree.frames[CHILD].frame_seq, 1);
    assert_eq!(tree.frames[CHILD].parent.as_deref(), Some(MAIN));
}

#[test]
fn frame_tree_navigation_renews_seq_and_drops_contexts() {
    let mut tree = tree_with_child();
    tree.context_created(&session(), CHILD, UTILITY_WORLD, false, 7);
    tree.context_created(&session(), CHILD, "", true, 8);
    tree.frames.get_mut(CHILD).unwrap().injected = Some("obj".to_owned());
    assert_eq!(tree.frames[CHILD].utility_context, Some(7));
    assert_eq!(tree.frames[CHILD].main_context, Some(8));

    let before = tree.frames[CHILD].frame_seq;
    tree.navigated(&session(), CHILD, Some(MAIN.to_owned()));

    let child = tree.frames[CHILD].clone();
    assert!(child.frame_seq > before);
    assert_eq!(child.utility_context, None);
    assert_eq!(child.main_context, None);
    assert_eq!(child.injected, None);

    tree.navigated(&session(), MAIN, None);
    assert!(tree.frames[MAIN].frame_seq > child.frame_seq);
}

#[test]
fn frame_tree_keeps_injected_for_a_context_it_already_knows() {
    let mut tree = tree_with_child();
    tree.context_created(&session(), CHILD, UTILITY_WORLD, false, 7);
    tree.frames.get_mut(CHILD).unwrap().injected = Some("obj".to_owned());

    tree.context_created(&session(), CHILD, UTILITY_WORLD, false, 7);
    assert_eq!(tree.frames[CHILD].injected.as_deref(), Some("obj"));

    tree.context_destroyed(&session(), 7);
    assert_eq!(tree.frames[CHILD].utility_context, None);
    assert_eq!(tree.frames[CHILD].injected, None);
}

#[test]
fn frame_tree_detach_removes_descendants_unless_swapped() {
    let mut tree = tree_with_child();
    tree.detached(CHILD, true);
    assert!(tree.frames.contains_key(CHILD));

    tree.detached(MAIN, false);
    assert!(tree.frames.is_empty());
}

#[test]
fn frame_tree_applies_cdp_events() {
    let mut tree = FrameTree::default();
    let session = session();
    tree.apply(
        &session,
        "Page.frameAttached",
        &json!({ "frameId": CHILD, "parentFrameId": MAIN }),
    );
    tree.apply(
        &session,
        "Runtime.executionContextCreated",
        &json!({ "context": { "id": 3, "name": UTILITY_WORLD, "auxData": { "frameId": CHILD, "isDefault": false } } }),
    );
    assert_eq!(tree.frames[CHILD].utility_context, Some(3));

    tree.apply(&session, "Runtime.executionContextsCleared", &json!({}));
    assert_eq!(tree.frames[CHILD].utility_context, None);
}

#[test]
fn frame_tree_adopts_a_frame_into_a_child_session() {
    let mut tree = tree_with_child();
    let child_session = SessionId("s2".to_owned());

    assert!(tree.adopt(CHILD, &child_session, None));
    tree.context_created(&session(), CHILD, UTILITY_WORLD, false, 9);
    assert_eq!(tree.frames[CHILD].utility_context, None);

    tree.session_detached(&child_session);
    assert!(!tree.frames.contains_key(CHILD));
}

#[test]
fn frame_tree_adopts_an_unannounced_frame_under_its_parent() {
    let mut tree = tree_with_child();
    let child_session = SessionId("s2".to_owned());

    assert!(!tree.adopt("late", &child_session, None));
    assert!(!tree.adopt("late", &child_session, Some("nowhere")));
    assert!(tree.adopt("late", &child_session, Some(MAIN)));
    assert_eq!(tree.frames["late"].parent.as_deref(), Some(MAIN));
    assert_eq!(tree.frames["late"].session_id, child_session);
}

#[test_case("e5", Some(None); "main frame")]
#[test_case("f12e5", Some(Some(12)); "child frame")]
#[test_case("f12", None; "no element")]
#[test_case("fe5", None; "no frame digits")]
#[test_case("x5", None; "garbage")]
fn parses_refs(text: &str, expected: Option<Option<u32>>) {
    assert_eq!(parse_ref(text), expected);
}

#[test_case(Some("e1"), None, true; "ref")]
#[test_case(None, Some("button"), true; "selector")]
#[test_case(Some("e1"), Some("button"), false; "both")]
#[test_case(None, None, false; "neither")]
fn target_takes_exactly_one(r#ref: Option<&str>, selector: Option<&str>, ok: bool) {
    let target = Target::from_input(r#ref.map(str::to_owned), selector.map(str::to_owned));

    match target {
        Ok(_) => assert!(ok),
        Err(error) => assert!(!ok && matches!(error, Error::InvalidArgument { .. })),
    }
}

#[test]
fn upload_debug_prints_the_length_not_the_bytes() {
    let file = UploadFile {
        name: "a.txt".to_owned(),
        mime: "text/plain".to_owned(),
        data: b"hunter2".to_vec(),
    };

    let text = format!("{file:?}");

    assert!(!text.contains("104"), "{text}");
    assert!(text.contains("len: 7"), "{text}");
}
