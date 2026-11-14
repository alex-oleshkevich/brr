use axum::Router;
use axum::body::Body;
use axum::routing::get;
use tokio::net::TcpListener;

use super::{Adblock, MAX_LIST_BYTES, fetch_list};

#[tokio::test]
async fn blocks_listed() {
    let adblock = Adblock::start();
    let script = "https://ads.example/a.js";
    let source = "https://site.test/";
    assert!(!adblock.should_block(script, source, "Script").await);

    adblock.replace(vec!["||ads.example^".into()]);

    assert!(adblock.should_block(script, source, "Script").await);
    assert!(
        !adblock
            .should_block("https://cdn.test/a.js", source, "Script")
            .await
    );
}

#[tokio::test]
async fn cosmetic() {
    let adblock = Adblock::start();
    adblock.replace(vec!["site.test###banner".into()]);

    let css = adblock
        .cosmetic_css("https://site.test/", Vec::new(), Vec::new())
        .await
        .unwrap();

    assert!(css.contains("#banner{display:none!important}"), "{css}");
    assert_eq!(
        adblock
            .cosmetic_css("https://other.test/", Vec::new(), Vec::new())
            .await,
        None
    );
}

#[tokio::test]
async fn fetch_capped() {
    let list = (0..10)
        .map(|n| format!("||ads{n}.example^\n"))
        .collect::<String>();
    let served = list.clone();
    let router = Router::new()
        .route("/list", get(move || async move { served }))
        .route(
            "/huge",
            get(|| async {
                let chunk = vec![b'a'; 1 << 20];
                let chunks = (0..MAX_LIST_BYTES / (1 << 20) + 8)
                    .map(move |_| Ok::<_, std::convert::Infallible>(chunk.clone()));
                Body::from_stream(futures::stream::iter(chunks))
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await });
    let http = reqwest::Client::new();

    let err = fetch_list(&http, &format!("{base}/huge"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("exceeds 32 MiB"), "{err}");

    assert_eq!(
        fetch_list(&http, &format!("{base}/list")).await.unwrap(),
        list
    );
}

#[tokio::test]
async fn cosmetic_rejects_unsafe_selectors() {
    let adblock = Adblock::start();
    let list =
        "site.test###ok\nsite.test##a{color:red}\nsite.test##div[\nsite.test#?#div:has-text(ad)\n";
    adblock.replace(vec![list.into()]);

    let css = adblock
        .cosmetic_css("https://site.test/", Vec::new(), Vec::new())
        .await
        .unwrap();

    assert_eq!(css, "#ok{display:none!important}");
}

#[tokio::test]
async fn cosmetic_generic_rules_follow_the_page_names() {
    let adblock = Adblock::start();
    adblock.replace(vec![
        "##.ad-banner\n###sponsor\n##.unused\nsite.test#@#.ad-banner\n".into(),
    ]);
    let names = |names: &[&str]| names.iter().map(ToString::to_string).collect::<Vec<_>>();

    let css = adblock
        .cosmetic_css(
            "https://other.test/",
            names(&["ad-banner", "plain"]),
            names(&["sponsor"]),
        )
        .await
        .unwrap();
    assert!(css.contains(".ad-banner"), "{css}");
    assert!(css.contains("#sponsor"), "{css}");
    assert!(!css.contains(".unused"), "{css}");

    let excepted = adblock
        .cosmetic_css("https://site.test/", names(&["ad-banner"]), Vec::new())
        .await;
    assert_eq!(excepted, None, "an exception rule wins");
    assert_eq!(
        adblock
            .cosmetic_css("https://other.test/", Vec::new(), Vec::new())
            .await,
        None,
        "no page names, no generic rules"
    );
}
