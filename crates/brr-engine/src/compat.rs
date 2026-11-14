use std::collections::HashSet;
use std::future::Future;
use std::time::Duration;

use brr_core::capture::{ImageFormat, ResourceType};
use brr_core::{Cookie, Error, InstanceId, SameSite, Size, WaitUntil};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::page::{ClickOptions, Clip, MouseButton, Page, ScreenshotOptions, Target};
use crate::procs::kill_marked;
use crate::{Driver, LaunchSpec, RequestRules};

// The scan reads every process's environ: it must not run on an async worker.
async fn sweep_marked(marked: &HashSet<InstanceId>) {
    let ids = marked.clone();
    let _ = tokio::task::spawn_blocking(move || kill_marked(&ids)).await;
}

const STEP_BUDGET: Duration = Duration::from_secs(60);
const FRAME_WAIT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const ACTION_TIMEOUT: Duration = Duration::from_secs(10);
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(30);
const EVAL_TIMEOUT: Duration = Duration::from_secs(10);
const SCREENCAST_SIZE: Size = Size {
    width: 640,
    height: 480,
};
const SCREENCAST_QUALITY: u8 = 50;
const REQUEST_BYTES: usize = 4096;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const PNG_SIZE_AT: usize = 16;
const PIXEL_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xFC, 0xFF, 0xFF, 0x3F,
    0x00, 0x05, 0xFE, 0x02, 0xFE, 0xA7, 0x35, 0x81, 0x84, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];
const FORM_PAGE: &str = "data:text/html,<title>start</title><button onclick=\"document.title='clicked'\">Go</button><input id=i onkeydown=\"if (event.key == 'Enter') document.title = 'entered'\"><canvas id=c width=64 height=64></canvas><script>document.getElementById('c').getContext('2d').fillRect(0,0,32,32)</script>";
const LOCAL_PAGE: &str = "<title>local</title><img src='/pixel.png'><p>ok</p>";
const PIXEL_PATH: &str = "/pixel.png";
const LAUNCH: &str = "launch";
const NAVIGATE: &str = "navigate";
const SNAPSHOT: &str = "snapshot";
const INPUT: &str = "input";
const SCREENCAST: &str = "screencast";
const COOKIES: &str = "cookies";
const INTERCEPTION: &str = "interception";
const SCREENSHOT: &str = "screenshot";
const STEP_COUNT: usize = 8;

#[derive(Debug, Clone)]
pub struct CompatReport {
    pub passed: bool,
    pub steps: Vec<(String, Result<(), String>)>,
}

fn failure(error: impl std::fmt::Display) -> String {
    error.to_string()
}

// A step that outlasts its budget fails like any other.
async fn within<T>(name: &str, work: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    tokio::time::timeout(STEP_BUDGET, work)
        .await
        .unwrap_or_else(|_| Err(format!("{name} did not finish within {STEP_BUDGET:?}")))
}

async fn eval(page: &Page, expression: &str) -> Result<serde_json::Value, String> {
    page.evaluate(expression, EVAL_TIMEOUT)
        .await
        .map_err(failure)
}

async fn poll_until(mut done: impl FnMut() -> bool, what: &str) -> Result<(), String> {
    tokio::time::timeout(ACTION_TIMEOUT, async {
        while !done() {
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .map_err(|_| format!("{what} did not happen in time"))
}

// A tiny responder on 127.0.0.1: the cookie and interception steps need a web origin.
async fn serve() -> Result<(String, JoinHandle<()>), Error> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| Error::Internal {
            message: error.to_string(),
        })?;
    let address = listener.local_addr().map_err(|error| Error::Internal {
        message: error.to_string(),
    })?;

    let accepting = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(respond(stream));
        }
    });

    Ok((address.to_string(), accepting))
}

async fn respond(mut stream: TcpStream) {
    let mut request = [0_u8; REQUEST_BYTES];
    let read = stream.read(&mut request).await.unwrap_or(0);
    let request = String::from_utf8_lossy(&request[..read]);
    let path = request.split_whitespace().nth(1).unwrap_or("/");
    let (kind, body): (&str, &[u8]) = if path == PIXEL_PATH {
        ("image/png", PIXEL_PNG)
    } else {
        ("text/html", LOCAL_PAGE.as_bytes())
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body).await;
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(&PNG_SIGNATURE) {
        return None;
    }
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|word| u32::from_be_bytes([word[0], word[1], word[2], word[3]]))
    };

    Some((word(PNG_SIZE_AT)?, word(PNG_SIZE_AT + 4)?))
}

type Steps = Vec<(String, Result<(), String>)>;

// Records one step: its failure is returned with its name, its success is kept in the report.
async fn step(
    steps: &mut Steps,
    name: &'static str,
    work: impl Future<Output = Result<(), String>>,
) -> Result<(), (&'static str, String)> {
    within(name, work)
        .await
        .map_err(|message| (name, message))?;
    steps.push((name.to_owned(), Ok(())));

    Ok(())
}

// R3: the checks a Camoufox release must pass before it becomes latest-compatible. Steps run in
// order and stop at the first failure; the browser is closed whatever happens.
pub async fn run_compat(mut spec: LaunchSpec) -> CompatReport {
    // A fresh id marks this run's processes, so none can be mistaken for a user's Instance.
    spec.instance_id = InstanceId::new();
    let marked = HashSet::from([spec.instance_id]);
    let mut steps: Steps = Vec::new();

    let launched = within(LAUNCH, async {
        Driver::launch(spec).await.map_err(failure)
    })
    .await;
    let launched = match launched {
        Ok(launched) => {
            steps.push((LAUNCH.to_owned(), Ok(())));
            launched
        }
        Err(message) => {
            // A launch that failed or timed out after the spawn leaves the browser running.
            sweep_marked(&marked).await;
            steps.push((LAUNCH.to_owned(), Err(message)));

            return CompatReport {
                passed: false,
                steps,
            };
        }
    };

    let mut events = launched.events;
    // The queue is bounded: an undrained one would stall the browser's own event task.
    tokio::spawn(async move { while events.recv().await.is_some() {} });

    if let Err((name, message)) =
        checks(&launched.driver, &launched.first_target_id, &mut steps).await
    {
        steps.push((name.to_owned(), Err(message)));
    }

    // close() kills a browser that ignores it; the scan catches what it left in other processes.
    let _ = launched.driver.close().await;
    sweep_marked(&marked).await;

    let passed = steps.len() == STEP_COUNT && steps.iter().all(|(_, result)| result.is_ok());

    CompatReport { passed, steps }
}

async fn checks(
    driver: &Driver,
    target_id: &brr_core::TargetId,
    steps: &mut Steps,
) -> Result<(), (&'static str, String)> {
    let mut page = None;
    step(steps, NAVIGATE, async {
        driver
            .navigate(target_id, FORM_PAGE, WaitUntil::Load, NAVIGATE_TIMEOUT)
            .await
            .map_err(failure)?;
        page = Some(driver.page(target_id).map_err(failure)?);

        Ok(())
    })
    .await?;
    let page = page.ok_or((NAVIGATE, "the Tab has no page".to_owned()))?;

    let mut snapshot = String::new();
    step(steps, SNAPSHOT, async {
        snapshot = page.snapshot().await.map_err(failure)?;
        snapshot
            .contains("button")
            .then_some(())
            .ok_or_else(|| "the snapshot has no button".to_owned())
    })
    .await?;

    step(steps, INPUT, async {
        let button = ref_of(&snapshot, "button").ok_or("the snapshot has no button ref")?;
        let click = ClickOptions {
            button: MouseButton::Left,
            modifiers: Vec::new(),
            click_count: 1,
        };
        page.click(&Target::Ref(button), &click, ACTION_TIMEOUT)
            .await
            .map_err(failure)?;
        if eval(&page, "document.title").await? != json!("clicked") {
            return Err("the click did not reach the page".to_owned());
        }

        let input = Target::Selector("#i".to_owned());
        page.fill(&input, "hello", ACTION_TIMEOUT)
            .await
            .map_err(failure)?;
        page.press_key(Some(&input), "Enter", ACTION_TIMEOUT)
            .await
            .map_err(failure)?;
        if eval(&page, "document.title").await? != json!("entered") {
            return Err("typing did not reach the page".to_owned());
        }

        Ok(())
    })
    .await?;

    step(steps, SCREENCAST, async {
        let mut screencast = page
            .start_screencast(SCREENCAST_SIZE, SCREENCAST_QUALITY)
            .await
            .map_err(failure)?;
        // A still page may send nothing; a repaint makes sure it does.
        eval(&page, "document.body.style.background = 'red'").await?;
        let frame = tokio::time::timeout(FRAME_WAIT, screencast.next())
            .await
            .map_err(|_| "no frame within the wait".to_owned())?
            .ok_or("the screencast ended")?;

        (frame.width > 0 && frame.height > 0)
            .then_some(())
            .ok_or_else(|| "the frame has no size".to_owned())
    })
    .await?;

    // The cookie and interception steps need a web origin.
    let mut server = None;
    let mut local = String::new();
    let outcome = serve_and_run(driver, target_id, &page, steps, &mut server, &mut local).await;
    if let Some(server) = server {
        server.abort();
    }

    outcome
}

async fn serve_and_run(
    driver: &Driver,
    target_id: &brr_core::TargetId,
    page: &Page,
    steps: &mut Steps,
    server: &mut Option<JoinHandle<()>>,
    local: &mut String,
) -> Result<(), (&'static str, String)> {
    step(steps, COOKIES, async {
        let (origin, accepting) = serve().await.map_err(failure)?;
        *server = Some(accepting);
        *local = format!("http://{origin}/");
        driver
            .navigate(target_id, local, WaitUntil::Load, NAVIGATE_TIMEOUT)
            .await
            .map_err(failure)?;
        let cookie = Cookie {
            name: "compat".to_owned(),
            value: "1".to_owned(),
            domain: "127.0.0.1".to_owned(),
            path: "/".to_owned(),
            expires: -1.0,
            http_only: false,
            secure: false,
            same_site: SameSite::Lax,
        };
        page.set_cookies(std::slice::from_ref(&cookie))
            .await
            .map_err(failure)?;
        let read = page.cookies(&[]).await.map_err(failure)?;
        if !read
            .iter()
            .any(|read| read.name == cookie.name && read.value == cookie.value)
        {
            return Err("the cookie did not read back".to_owned());
        }
        page.clear_cookies().await.map_err(failure)?;
        if !page.cookies(&[]).await.map_err(failure)?.is_empty() {
            return Err("clearing left cookies".to_owned());
        }

        Ok(())
    })
    .await?;

    step(steps, INTERCEPTION, async {
        page.set_request_rules(RequestRules {
            patterns: Vec::new(),
            types: vec![ResourceType::Image],
        })
        .await
        .map_err(failure)?;
        let before = page.network_log().len();
        page.reload(WaitUntil::Load, NAVIGATE_TIMEOUT)
            .await
            .map_err(failure)?;
        let blocked = || {
            page.network_log()[before..]
                .iter()
                .any(|entry| entry.url.ends_with(PIXEL_PATH) && entry.failure.is_some())
        };
        let seen = poll_until(blocked, "the image being blocked").await;
        page.set_request_rules(RequestRules::default())
            .await
            .map_err(failure)?;

        seen
    })
    .await?;

    step(steps, SCREENSHOT, async {
        let image = page
            .screenshot(&ScreenshotOptions {
                clip: Clip::Viewport,
                format: ImageFormat::Png,
                quality: None,
                transparent: false,
            })
            .await
            .map_err(failure)?;
        let size = png_size(&image).ok_or("the screenshot is not a PNG")?;
        let viewport = eval(page, "[innerWidth, innerHeight]").await?;
        let expected = (
            viewport[0].as_u64().unwrap_or_default(),
            viewport[1].as_u64().unwrap_or_default(),
        );

        if (u64::from(size.0), u64::from(size.1)) == expected && expected.0 > 0 {
            Ok(())
        } else {
            Err(format!(
                "the screenshot is {size:?}, the viewport {expected:?}"
            ))
        }
    })
    .await
}

// `button "Go" [ref=e2]` -> `e2`.
fn ref_of(snapshot: &str, needle: &str) -> Option<String> {
    let line = snapshot.lines().find(|line| line.contains(needle))?;
    let start = line.find("[ref=")? + "[ref=".len();
    let end = line[start..].find(']')?;

    Some(line[start..start + end].to_owned())
}
