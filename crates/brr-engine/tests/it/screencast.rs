use std::time::Duration;

use brr_core::{Display, Size, WaitUntil};
use brr_engine::page::{Page, Screencast};
use brr_engine::{Driver, Launched};
use tempfile::TempDir;

use crate::{camoufox, chromium};

const MAX: Size = Size {
    width: 1280,
    height: 720,
};
const QUALITY: u8 = 60;
const FRAMES: usize = 5;
const FRAMES_WITHIN: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(30);
const ANIMATED: &str = "data:text/html,<style>@keyframes s{from{background:red}to{background:blue}}body{animation:s .2s infinite alternate;margin:0;height:100vh}</style><p>x</p>";

async fn frames(mut frames: Screencast) {
    let got = tokio::time::timeout(FRAMES_WITHIN, async {
        let mut got = Vec::new();
        while got.len() < FRAMES {
            got.push(frames.next().await.expect("the screencast ended"));
        }
        got
    })
    .await
    .expect("too few frames in time");

    for frame in got {
        assert!(
            frame.width > 0 && frame.width <= MAX.width,
            "{}",
            frame.width
        );
        assert!(frame.metadata.device_width > 0.0);
    }
}

// Dropping a stream frees the Tab's one screencast slot at once, and the restarted stream
// carries frames: the stop of the first must not land after the start of the second.
async fn restarts_after_drop(page: &Page) {
    let mut again = page.start_screencast(MAX, QUALITY).await.unwrap();
    tokio::time::timeout(FRAMES_WITHIN, again.next())
        .await
        .expect("the restarted screencast sent no frame")
        .expect("the restarted screencast ended");
}

async fn poll_page(driver: &Driver, target_id: &brr_core::TargetId) -> Page {
    tokio::time::timeout(FRAMES_WITHIN, async {
        loop {
            if let Ok(page) = driver.page(target_id) {
                return page;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .expect("the Tab never attached")
}

async fn run(launched: Launched) {
    let Launched {
        driver,
        mut events,
        first_target_id,
        ..
    } = launched;
    tokio::spawn(async move { while events.recv().await.is_some() {} });
    driver
        .navigate(
            &first_target_id,
            ANIMATED,
            WaitUntil::Load,
            NAVIGATE_TIMEOUT,
        )
        .await
        .unwrap();
    let page = driver.page(&first_target_id).unwrap();

    let receiver = page.start_screencast(MAX, QUALITY).await.unwrap();
    let second = page.start_screencast(MAX, QUALITY).await.err().unwrap();
    assert!(
        matches!(second, brr_core::Error::InvalidArgument { field, .. } if field == "screencast")
    );
    frames(receiver).await;
    restarts_after_drop(&page).await;

    // A stream ends with its Tab.
    let other = driver.open_tab(ANIMATED).await.unwrap();
    let other_page = poll_page(&driver, &other).await;
    let mut doomed = other_page.start_screencast(MAX, QUALITY).await.unwrap();
    driver.close_tab(&other).await.unwrap();
    let ended = tokio::time::timeout(FRAMES_WITHIN, async {
        while doomed.next().await.is_some() {}
    })
    .await;
    assert!(ended.is_ok(), "the stream outlived its Tab");

    driver.close().await.unwrap();
}

#[tokio::test]
async fn chromium_frames() {
    let Some(chrome) = chromium::chrome() else {
        return;
    };
    let dir = TempDir::new().unwrap();
    let spec = chromium::spec(chrome, dir.path(), brr_core::InstanceId::new());

    run(Driver::launch(spec).await.unwrap()).await;
}

#[tokio::test]
async fn camoufox_frames() {
    let Some(assets) = camoufox::assets() else {
        return;
    };
    let dir = TempDir::new().unwrap();
    let spec = camoufox::spec(assets, dir.path(), Display::Headless, None);

    run(Driver::launch(spec).await.unwrap()).await;
}
