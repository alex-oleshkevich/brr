use brr_content::Adblock;

use super::{ADBLOCK_VERDICT_TIMEOUT, adblock_verdict};
use crate::chromium::fetch::{PausedRequest, Verdict};

const AD_URL: &str = "https://site.test/ad.js";
const SLOW_LIST_RULES: usize = 500_000;

fn request(url: &str) -> PausedRequest {
    PausedRequest {
        url: url.to_owned(),
        resource_type: "Script".to_owned(),
        frame_id: "main".to_owned(),
        source_url: None,
    }
}

#[tokio::test(start_paused = true)]
async fn verdict_timeout_continues() {
    let adblock = Adblock::start();
    // The thread is busy building this when the request arrives.
    let list = (0..SLOW_LIST_RULES)
        .map(|n| format!("||host{n}.test^$script"))
        .collect::<Vec<_>>()
        .join("\n");
    adblock.replace(vec![list]);

    let started = tokio::time::Instant::now();
    let verdict = adblock_verdict(&adblock, &request(AD_URL)).await;

    assert_eq!(verdict, Verdict::Continue);
    assert_eq!(started.elapsed(), ADBLOCK_VERDICT_TIMEOUT);
}

#[tokio::test]
async fn verdict_fails_a_request_the_lists_block() {
    let adblock = Adblock::start();
    adblock.replace(vec!["/ad.js$script".to_owned()]);

    assert_eq!(
        adblock_verdict(&adblock, &request(AD_URL)).await,
        Verdict::Fail
    );
    assert_eq!(
        adblock_verdict(&adblock, &request("https://site.test/ok.js")).await,
        Verdict::Continue
    );
}
