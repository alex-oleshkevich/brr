use brr_core::TabId;
use brr_engine::page::FrameMetadata;
use test_case::test_case;

use super::{Producers, to_css};

fn metadata(page_scale_factor: f64) -> FrameMetadata {
    FrameMetadata {
        device_width: 1280.0,
        device_height: 720.0,
        page_scale_factor,
        offset_top: 0.0,
    }
}

#[test_case((1280, 720), 1.0, (640.0, 360.0); "frame as large as the page")]
#[test_case((640, 360), 1.0, (1280.0, 720.0); "frame half the page")]
#[test_case((1280, 720), 2.0, (320.0, 180.0); "zoomed page")]
#[test_case((1280, 720), 0.0, (640.0, 360.0); "no scale factor reported")]
fn frame_pixels_to_css(frame: (u32, u32), scale: f64, expected: (f64, f64)) {
    assert_eq!(to_css(640.0, 360.0, frame, &metadata(scale)), expected);
}

#[tokio::test]
async fn finished_producers_leave_the_ending_map() {
    let mut aborted = tokio::spawn(std::future::pending::<()>());
    aborted.abort();
    let _ = (&mut aborted).await;
    let running = tokio::spawn(std::future::pending::<()>());
    let (ended_tab, running_tab) = (TabId::new(), TabId::new());
    let mut producers = Producers::default();
    producers.ending.insert(ended_tab, aborted);
    producers.ending.insert(running_tab, running);

    producers.drop_finished();

    assert!(!producers.ending.contains_key(&ended_tab));
    assert!(producers.ending.contains_key(&running_tab));
}
