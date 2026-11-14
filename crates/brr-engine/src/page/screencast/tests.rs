#![allow(clippy::unwrap_used)]

use base64::Engine;
use brr_core::Error;
use test_case::test_case;

use super::{FrameMetadata, PageState, jpeg_size};

const WIDTH: u16 = 1280;
const HEIGHT: u16 = 720;
const METADATA: FrameMetadata = FrameMetadata {
    device_width: 1.0,
    device_height: 1.0,
    page_scale_factor: 1.0,
    offset_top: 0.0,
};

// SOI, an APP0 segment, a DHT segment (marker 0xC4 sits in the SOF range), then the frame header.
fn jpeg(sof: u8) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
    bytes.extend([0xFF, 0xC4, 0x00, 0x05, 0x01, 0x02, 0x03]);
    bytes.extend([0xFF, sof, 0x00, 0x0B, 0x08]);
    bytes.extend(HEIGHT.to_be_bytes());
    bytes.extend(WIDTH.to_be_bytes());
    bytes.extend([0x01, 0x01, 0x11, 0x00, 0xFF, 0xD9]);

    bytes
}

#[test_case(0xC0; "baseline")]
#[test_case(0xC2; "progressive")]
fn the_size_comes_from_the_frame_header(sof: u8) {
    assert_eq!(
        jpeg_size(&jpeg(sof)),
        Some((u32::from(WIDTH), u32::from(HEIGHT)))
    );
}

#[test]
fn a_truncated_jpeg_has_no_size() {
    let bytes = jpeg(0xC0);

    assert_eq!(jpeg_size(&bytes[..bytes.len() - 9]), None);
    assert_eq!(jpeg_size(&[0xFF, 0xD8]), None);
}

#[test]
fn something_else_has_no_size() {
    assert_eq!(jpeg_size(b"\x89PNG\r\n\x1a\n"), None);
}

fn encoded() -> String {
    base64::engine::general_purpose::STANDARD.encode(jpeg(0xC0))
}

fn at(offset_top: f64) -> FrameMetadata {
    FrameMetadata {
        offset_top,
        ..METADATA
    }
}

#[test]
fn a_second_screencast_is_refused_until_the_first_is_released() {
    let state = PageState::new(10, 10);

    let first = state.claim_screencast().unwrap();
    let error = state.claim_screencast().unwrap_err();
    assert!(matches!(error, Error::InvalidArgument { field, .. } if field == "screencast"));

    drop(first);
    state.release_screencast();
    assert!(state.claim_screencast().is_ok());
}

#[tokio::test]
async fn a_slow_reader_gets_the_latest_frame_only() {
    let state = PageState::new(10, 10);
    let mut frames = state.claim_screencast().unwrap();

    for offset in [1.0, 2.0, 3.0] {
        state.screencast_frame(encoded(), at(offset));
    }

    frames.changed().await.unwrap();
    let frame = frames.borrow_and_update().clone().unwrap();
    assert_eq!(frame.metadata.offset_top, 3.0);
    assert_eq!(
        (frame.width, frame.height),
        (u32::from(WIDTH), u32::from(HEIGHT))
    );
    assert!(!frames.has_changed().unwrap());
}

#[tokio::test]
async fn releasing_the_stream_ends_it() {
    let state = PageState::new(10, 10);
    let mut frames = state.claim_screencast().unwrap();

    state.release_screencast();

    assert!(frames.changed().await.is_err());
}

#[tokio::test]
async fn a_frame_without_a_readable_header_is_dropped() {
    let state = PageState::new(10, 10);
    let frames = state.claim_screencast().unwrap();

    state.screencast_frame("bm90IGEganBlZw==".to_owned(), METADATA);

    assert!(!frames.has_changed().unwrap());
}

#[test]
fn a_frame_of_non_ascii_text_does_not_panic() {
    let state = PageState::new(10, 10);
    let _frames = state.claim_screencast().unwrap();

    state.screencast_frame("é".repeat(100_000), METADATA);
}
