use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex, MutexGuard, PoisonError, Weak},
    time::Duration,
};

use brr_core::{
    ArtifactKind, Capability, Display, EndReason, Error, Event, InstanceId, InstanceState, Profile,
    ProfileId, ProfileKind, ProxyChoice, ProxyId, Settings, Tab, TabCloseReason, TabId, TabState,
    WaitUntil,
    capture::{CaptureParams, ImageFormat, OneshotKind},
    now_ms,
};
use brr_engine::{
    ContextId, RequestRules,
    page::{Cleanup, Clip, EmulateOptions, Page, ScreenshotOptions, Target, WaitCondition},
};
use brr_store::{Store, profile, template};
use tokio::{
    sync::{broadcast, oneshot},
    task::AbortHandle,
    time::{self, Instant},
};
use tracing::warn;

use crate::{
    app::{App, CallClient},
    internal,
    ops::invalid,
    services::{
        SETTINGS_CHANGED, STATE_CHANGED,
        artifacts::{self, ArtifactOwner, Delivery},
        events::{self, Published},
        instances::LaunchRequest,
        profiles::{DEFAULT_BACKUP_KEEP, resolve_build},
        until,
    },
};

#[cfg(test)]
mod tests;

const NO_RESTART: &str = "no_restart";
const TEMPLATE: &str = "chromium-headless";
// A member that crashes for good soon after it started is not replaced at once: the pool waits
// POOL_BACKOFF_BASE, doubling per consecutive failure up to POOL_BACKOFF_MAX.
const POOL_BACKOFF_BASE: Duration = Duration::from_secs(15);
const POOL_BACKOFF_MAX: Duration = Duration::from_secs(10 * 60);
const POOL_STABLE: Duration = Duration::from_secs(5 * 60);
const MAX_BACKOFF_DOUBLINGS: u32 = 6;

#[derive(Default)]
pub struct Oneshot {
    pool: Mutex<Pool>,
}

// Only the Instances this service launched: the oneshot Profile may be launched by others too.
#[derive(Default)]
struct Pool {
    members: HashMap<InstanceId, Member>,
    // Set at shutdown: a member that ends then is not replaced.
    closed: bool,
    crashes: u32,
    refill_at: Option<Instant>,
}

struct Member {
    profile_id: ProfileId,
    contexts: usize,
    linger: Option<AbortHandle>,
    started: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    // A context in an Instance of the pool.
    Pooled,
    // An Instance of its own, for a request with its own proxy.
    Dedicated,
    // A Tab in a Persistent Profile's Instance.
    Persistent,
}

// What a request holds, so one cleanup can undo whatever part of it was reached.
struct Held {
    instance_id: InstanceId,
    origin: Origin,
    context_id: Option<ContextId>,
    tab_id: Option<TabId>,
}

// Dropping it cleans up too: a client that disconnects mid-request drops the whole request.
struct Session {
    app: Arc<App>,
    held: Option<Held>,
}

impl Session {
    // Its own task: a request dropped while it disposes must not leave the counts stuck.
    async fn finish(&mut self) {
        if let Some(held) = self.held.take() {
            let app = Arc::clone(&self.app);

            if let Err(error) = tokio::spawn(async move { cleanup(&app, held).await }).await {
                warn!(%error, "oneshot cleanup did not finish");
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(held) = self.held.take() {
            let app = Arc::clone(&self.app);

            tokio::spawn(async move { cleanup(&app, held).await });
        }
    }
}

// Runs `work` to the end even when the caller is dropped, and hands what it made to `undo`
// then: a launch or an open must not leave a browser or a Tab nobody owns.
pub(crate) async fn detached<T, U, Fut>(
    work: impl Future<Output = Result<T, Error>> + Send + 'static,
    undo: U,
) -> Result<T, Error>
where
    T: Send + 'static,
    U: FnOnce(T) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (reply, answer) = oneshot::channel();

    tokio::spawn(events::keep_actor(async move {
        if let Err(Ok(made)) = reply.send(work.await) {
            undo(made).await;
        }
    }));

    answer.await.map_err(internal)?
}

pub async fn ensure_profile(store: &Store, settings: &Settings) -> Result<(), Error> {
    let name = settings.oneshot.profile.clone();

    store
        .run(move |c| {
            match profile::find(c, &name) {
                Ok(_) => return Ok(()),
                Err(Error::NotFound { .. }) => {}
                Err(error) => return Err(error),
            }

            let template = template::find(c, TEMPLATE)?;
            let now = now_ms();

            profile::insert(
                c,
                &Profile {
                    id: ProfileId::new(),
                    name,
                    kind: ProfileKind::Ephemeral,
                    template_id: Some(template.id),
                    settings: template.settings,
                    pinned: false,
                    notes: String::new(),
                    tags: Vec::new(),
                    data_dir: None,
                    disk_quota_bytes: None,
                    backup_schedule: None,
                    backup_keep: DEFAULT_BACKUP_KEEP,
                    created_at: now,
                    updated_at: now,
                    capabilities: Vec::new(),
                    login_status: None,
                    builtin: false,
                },
            )
        })
        .await
}

// Keeps `pool_size` Instances running; a tracked Instance that ends is replaced. Held weakly: a
// strong reference would keep the daemon lock after the last owner drops the App.
pub fn spawn_pool(app: &Arc<App>) {
    let events = app.events.subscribe();

    tokio::spawn(keep_pool(Arc::downgrade(app), events));
}

async fn keep_pool(app: Weak<App>, mut events: broadcast::Receiver<Arc<Published>>) {
    loop {
        let Some(strong) = app.upgrade() else {
            return;
        };
        fill(&strong).await;
        drop(strong);

        loop {
            let retry_at = match app.upgrade() {
                Some(strong) => strong.oneshot.lock().backing_off(Instant::now()),
                None => return,
            };
            let received = tokio::select! {
                received = events.recv() => received,
                // The backoff is over: the pool is filled again.
                () = until(retry_at) => break,
            };
            let Some(strong) = app.upgrade() else {
                return;
            };

            match received {
                Ok(published) => {
                    if changes_pool(&strong, &published.event).await {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => break,
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}

// Whether the event ends a tracked Instance (it is forgotten then) or may have changed the pool
// size (the members beyond it are stopped).
async fn changes_pool(app: &Arc<App>, event: &Event) -> bool {
    match event.kind.as_str() {
        STATE_CHANGED => {
            let ended = serde_json::from_value::<InstanceState>(event.data["to"].clone())
                .is_ok_and(InstanceState::is_final);
            let crashed = event.data["trigger"] == NO_RESTART;

            ended
                && serde_json::from_value::<InstanceId>(event.data["instance_id"].clone())
                    .is_ok_and(|instance_id| {
                        app.oneshot
                            .lock()
                            .forget(&instance_id, crashed, Instant::now())
                    })
        }
        SETTINGS_CHANGED => {
            trim(app).await;

            true
        }
        _ => false,
    }
}

async fn oneshot_profile(app: &Arc<App>) -> Option<Profile> {
    let name = app.settings().oneshot.profile.clone();

    match app.store.run(move |c| profile::find(c, &name)).await {
        Ok(found) => Some(found),
        Err(error) => {
            warn!(%error, "oneshot profile not found");

            None
        }
    }
}

async fn fill(app: &Arc<App>) {
    let Some(profile) = oneshot_profile(app).await else {
        return;
    };

    app.oneshot.lock().retain_alive(|instance_id| {
        app.instances
            .view(instance_id)
            .is_some_and(|view| !view.instance.state.is_final())
    });

    while !app.oneshot.lock().closed
        && app.oneshot.lock().backing_off(Instant::now()).is_none()
        && app.oneshot.lock().count(&profile.id) < app.settings().oneshot.pool_size as usize
    {
        match launch_instance(app, profile.clone(), None).await {
            Ok((instance_id, _)) => {
                app.oneshot.lock().track(instance_id, profile.id, 0);
                // A pooled Instance waits for requests on purpose: the idle reaper must not
                // end it just to have it launched again.
                app.instances.set_pool_member(&instance_id, true).await;
            }
            Err(error) => {
                warn!(%error, "oneshot pool instance not launched");

                return;
            }
        }
    }
}

// The pool shrank: members above its size that have no context go.
async fn trim(app: &Arc<App>) {
    let Some(profile) = oneshot_profile(app).await else {
        return;
    };
    let pool_size = app.settings().oneshot.pool_size as usize;
    let surplus = app.oneshot.lock().trim(&profile.id, pool_size);

    for instance_id in surplus {
        stop_instance(app, &instance_id).await;
    }
}

pub async fn run(
    app: &Arc<App>,
    client: &CallClient,
    kind: OneshotKind,
    params: CaptureParams,
) -> Result<Delivery, Error> {
    params.validate(kind)?;

    let limit = params.timeout();
    let mut session = Session {
        app: Arc::clone(app),
        held: None,
    };
    let result = time::timeout(limit, execute(app, client, kind, &params, &mut session)).await;
    session.finish().await;

    result.map_err(|_| Error::Timeout {
        after_ms: u64::try_from(limit.as_millis()).unwrap_or(u64::MAX),
    })?
}

async fn find_profile(app: &Arc<App>, name: String) -> Result<(Profile, Vec<Capability>), Error> {
    app.store
        .run(move |c| {
            let found = profile::find(c, &name)?;
            let capabilities = resolve_build(c, &found.settings.build)?.capabilities;

            Ok((found, capabilities))
        })
        .await
}

// The built-in Profile may have been deleted since the daemon started: it is made again once.
async fn resolve_profile(
    app: &Arc<App>,
    params: &CaptureParams,
) -> Result<(Profile, Vec<Capability>), Error> {
    if let Some(name) = &params.profile {
        return find_profile(app, name.clone()).await;
    }

    let name = app.settings().oneshot.profile.clone();
    match find_profile(app, name.clone()).await {
        Err(Error::NotFound { .. }) => {
            ensure_profile(&app.store, &app.settings()).await?;

            find_profile(app, name).await
        }
        found => found,
    }
}

// Before anything is launched, like a Tab tool checks it before it takes a Lease.
fn check_request(
    kind: OneshotKind,
    profile: &Profile,
    capabilities: &[Capability],
    params: &CaptureParams,
) -> Result<(), Error> {
    let needed = match kind {
        OneshotKind::Pdf => Some(Capability::Pdf),
        OneshotKind::Mhtml => Some(Capability::Mhtml),
        _ => None,
    };
    if let Some(capability) = needed
        && !capabilities.contains(&capability)
    {
        return Err(Error::EngineUnsupported { capability });
    }

    if profile.kind == ProfileKind::Persistent {
        if params.proxy.is_some() {
            return Err(invalid(
                "proxy",
                "a Persistent profile keeps its own proxy; leave proxy out or use an Ephemeral profile",
            ));
        }
        if params.cookies.is_some() {
            return Err(invalid(
                "cookies",
                "cookies apply only to ephemeral one-shots; import them into the profile instead",
            ));
        }
    }

    Ok(())
}

async fn execute(
    app: &Arc<App>,
    client: &CallClient,
    kind: OneshotKind,
    params: &CaptureParams,
    session: &mut Session,
) -> Result<Delivery, Error> {
    let (profile, capabilities) = resolve_profile(app, params).await?;
    check_request(kind, &profile, &capabilities, params)?;

    let (instance_id, tab) = match profile.kind {
        ProfileKind::Ephemeral => open_ephemeral(app, client, &profile, params, session).await?,
        ProfileKind::Persistent => open_persistent(app, client, &profile, session).await?,
    };
    let page = app.instances.page(&instance_id, &tab.id).await?;
    let limit = params.timeout();

    prepare(app, &page, &profile, params).await?;
    app.instances
        .navigate(
            &instance_id,
            &tab.id,
            params.url.clone(),
            params.wait_until.unwrap_or(WaitUntil::Load),
            limit,
        )
        .await?;
    if let Some(selector) = &params.wait_for {
        page.wait_for(&WaitCondition::Selector(selector.clone()), limit)
            .await?;
    }
    if let Some(delay) = params.delay {
        time::sleep(Duration::from_millis(delay)).await;
    }
    let cleanup = Cleanup {
        hide: params.hide.as_deref(),
        remove: params.remove.as_deref(),
        css: params.css.as_deref(),
        js: params.js.as_deref(),
    };
    if cleanup != Cleanup::default() {
        page.cleanup(&cleanup).await?;
    }

    let captured = capture(&page, kind, params).await?;

    artifacts::deliver(
        app,
        captured.kind,
        ArtifactOwner {
            profile_id: Some(profile.id),
            instance_id: Some(instance_id),
            tab_id: Some(tab.id),
        },
        captured.filename,
        captured.mime,
        captured.bytes,
        params.return_mode.unwrap_or_default(),
    )
    .await
}

// Emulation comes before the navigation: timezone, user agent and theme shape the first paint
// and what the server answers.
pub async fn prepare(
    app: &Arc<App>,
    page: &Page,
    profile: &Profile,
    params: &CaptureParams,
) -> Result<(), Error> {
    page.emulate(&EmulateOptions {
        viewport: Some(params.screen()),
        scale: params.scale,
        device: params.device.clone(),
        timezone: params.tz.clone(),
        locale: params.locale.clone(),
        user_agent: params.ua.clone(),
        color_scheme: params.theme,
        reduced_motion: params.reduced_motion,
        ..EmulateOptions::default()
    })
    .await?;

    if let Some(headers) = &params.headers {
        page.set_extra_headers(&headers.0).await?;
    }
    if let Some(cookies) = &params.cookies {
        page.set_cookies(&cookies.0).await?;
    }
    if let Some(block) = &params.block {
        page.set_request_rules(RequestRules {
            patterns: Vec::new(),
            types: block.0.clone(),
        })
        .await?;
    }
    if params.block_ads.is_some() || params.dismiss_cookies.is_some() {
        // The resolved values, so block_ads=false turns blocking off on a Profile that has it on.
        let options = app
            .content
            .options(
                app,
                params.block_ads.unwrap_or(profile.settings.block_ads),
                params
                    .dismiss_cookies
                    .unwrap_or(profile.settings.dismiss_cookies),
            )
            .await;
        page.set_content(options).await?;
    }

    Ok(())
}

struct Captured {
    kind: ArtifactKind,
    filename: &'static str,
    mime: &'static str,
    bytes: Vec<u8>,
}

async fn capture(
    page: &Page,
    kind: OneshotKind,
    params: &CaptureParams,
) -> Result<Captured, Error> {
    Ok(match kind {
        OneshotKind::Screenshot => {
            let format = params.format.unwrap_or(ImageFormat::Png);
            let clip = match (&params.selector, &params.area) {
                (Some(selector), _) => Clip::Element(Target::Selector(selector.clone())),
                (None, Some(area)) => Clip::Area(area.clone()),
                (None, None) if params.full_page == Some(true) => Clip::FullPage,
                (None, None) => Clip::Viewport,
            };
            let bytes = page
                .screenshot(&ScreenshotOptions {
                    clip,
                    format,
                    quality: params.quality,
                    transparent: params.transparent.unwrap_or(false),
                })
                .await?;

            Captured {
                kind: ArtifactKind::Screenshot,
                filename: image_filename(format),
                mime: format.mime(),
                bytes,
            }
        }
        OneshotKind::Pdf => Captured {
            kind: ArtifactKind::Pdf,
            filename: "pdf.pdf",
            mime: "application/pdf",
            bytes: page.pdf(&params.pdf()).await?,
        },
        OneshotKind::Mhtml => Captured {
            kind: ArtifactKind::Mhtml,
            filename: "mhtml.mhtml",
            mime: "multipart/related",
            bytes: page.mhtml().await?,
        },
        OneshotKind::Markdown => {
            let html = page.html().await?;
            let url = page.url().await?;
            let markdown = tokio::task::spawn_blocking(move || brr_content::markdown(&html, &url))
                .await
                .map_err(internal)??;

            Captured {
                kind: ArtifactKind::Markdown,
                filename: "markdown.md",
                mime: "text/markdown",
                bytes: markdown.into_bytes(),
            }
        }
        OneshotKind::Content => Captured {
            kind: ArtifactKind::Html,
            filename: "content.html",
            mime: "text/html",
            bytes: page.html().await?.into_bytes(),
        },
        OneshotKind::Links => Captured {
            // The artifact kinds have no links; it is stored as html only above the inline cap.
            kind: ArtifactKind::Html,
            filename: "links.json",
            mime: "application/json",
            bytes: serde_json::to_vec(&page.links().await?).map_err(internal)?,
        },
    })
}

fn image_filename(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "screenshot.png",
        ImageFormat::Jpeg => "screenshot.jpg",
        ImageFormat::Webp => "screenshot.webp",
    }
}

// A new browser context in a pooled Instance; a request with its own proxy gets an Instance of
// its own, stopped when it is done.
async fn open_ephemeral(
    app: &Arc<App>,
    client: &CallClient,
    profile: &Profile,
    params: &CaptureParams,
    session: &mut Session,
) -> Result<(InstanceId, Tab), Error> {
    let (instance_id, origin) = match &params.proxy {
        Some(proxy) => {
            let proxy_id: ProxyId = proxy
                .parse()
                .map_err(|_| invalid("proxy", "use the id of a proxy"))?;
            let mut own = profile.clone();
            own.settings.proxy = Some(ProxyChoice::ProxyId(proxy_id));

            (launch_dedicated(app, own).await?, Origin::Dedicated)
        }
        None => match app.oneshot.reserve(app, profile.id) {
            Some(instance_id) => (instance_id, Origin::Pooled),
            None => (launch_pooled(app, profile.clone()).await?, Origin::Pooled),
        },
    };
    session.held = Some(Held {
        instance_id,
        origin,
        context_id: None,
        tab_id: None,
    });

    let slot = app.admission.check_tab(app)?;
    let (context_id, tab) = open_context(app, instance_id).await?;
    drop(slot);
    if let Some(held) = &mut session.held {
        held.context_id = Some(context_id);
    }
    app.leases
        .acquire_tab(app, client, &instance_id, &tab.id)
        .await?;

    Ok((instance_id, tab))
}

async fn launch_dedicated(app: &Arc<App>, profile: Profile) -> Result<InstanceId, Error> {
    let (work_app, undo_app) = (Arc::clone(app), Arc::clone(app));

    detached(
        async move {
            launch_instance(&work_app, profile, None)
                .await
                .map(|(instance_id, _)| instance_id)
        },
        move |instance_id| async move { stop_instance(&undo_app, &instance_id).await },
    )
    .await
}

// Tracked with the request's own context already counted, so it cannot linger out before the
// request has used it.
async fn launch_pooled(app: &Arc<App>, profile: Profile) -> Result<InstanceId, Error> {
    let (work_app, undo_app) = (Arc::clone(app), Arc::clone(app));
    let profile_id = profile.id;

    detached(
        async move {
            let (instance_id, _) = launch_instance(&work_app, profile, None).await?;
            work_app.oneshot.lock().track(instance_id, profile_id, 1);

            Ok(instance_id)
        },
        move |instance_id| async move { undo_app.oneshot.release(&undo_app, &instance_id) },
    )
    .await
}

async fn open_context(app: &Arc<App>, instance_id: InstanceId) -> Result<(ContextId, Tab), Error> {
    let (work_app, undo_app) = (Arc::clone(app), Arc::clone(app));

    detached(
        async move {
            let context_id = work_app.instances.create_context(&instance_id).await?;
            let opened = work_app
                .instances
                .open_tab_in(&instance_id, &context_id, None)
                .await;

            match opened {
                Ok(tab) => Ok((context_id, tab)),
                Err(error) => {
                    dispose(&work_app, &instance_id, &context_id).await;

                    Err(error)
                }
            }
        },
        move |(context_id, _)| async move { dispose(&undo_app, &instance_id, &context_id).await },
    )
    .await
}

// A tab in the default context of the Persistent Profile's own Instance.
async fn open_persistent(
    app: &Arc<App>,
    client: &CallClient,
    profile: &Profile,
    session: &mut Session,
) -> Result<(InstanceId, Tab), Error> {
    let running = app.instances.views().into_iter().find(|view| {
        view.instance.profile_id == Some(profile.id)
            && matches!(
                view.instance.state,
                InstanceState::Running | InstanceState::Idle
            )
    });
    let (work_app, undo_app) = (Arc::clone(app), Arc::clone(app));
    let (instance_id, tab) = match running {
        Some(view) => {
            let instance_id = view.instance.id;
            app.leases
                .guard(app, client, &instance_id, None, false)
                .await?;
            let slot = app.admission.check_tab(app)?;
            let tab = detached(
                async move { work_app.instances.open_tab(&instance_id, None).await },
                move |tab| async move { close_tab(&undo_app, &instance_id, &tab.id).await },
            )
            .await?;
            drop(slot);

            (instance_id, tab)
        }
        None => {
            let profile = profile.clone();

            detached(
                async move { launch_instance(&work_app, profile, None).await },
                move |(instance_id, tab)| async move {
                    close_tab(&undo_app, &instance_id, &tab.id).await
                },
            )
            .await?
        }
    };
    session.held = Some(Held {
        instance_id,
        origin: Origin::Persistent,
        context_id: None,
        tab_id: Some(tab.id),
    });
    app.leases
        .acquire_tab(app, client, &instance_id, &tab.id)
        .await?;

    Ok((instance_id, tab))
}

pub(crate) async fn launch_instance(
    app: &Arc<App>,
    profile: Profile,
    display: Option<Display>,
) -> Result<(InstanceId, Tab), Error> {
    let defaults = app.settings().admission.clone();
    let (instance, tab) = app
        .instances
        .launch(
            app,
            LaunchRequest {
                profile,
                display,
                priority: defaults.priority,
                admission: defaults.default,
                force: false,
            },
        )
        .await?;

    Ok((instance.id, tab))
}

async fn dispose(app: &Arc<App>, instance_id: &InstanceId, context_id: &ContextId) {
    if let Err(error) = app.instances.dispose_context(instance_id, context_id).await {
        warn!(%instance_id, %error, "oneshot context not disposed");
    }
}

async fn close_tab(app: &Arc<App>, instance_id: &InstanceId, tab_id: &TabId) {
    let closed = app
        .instances
        .close_tab(instance_id, tab_id, TabCloseReason::Client)
        .await;
    if let Err(error) = closed {
        warn!(%instance_id, %error, "oneshot tab not closed");
    }
}

async fn stop_instance(app: &Arc<App>, instance_id: &InstanceId) {
    if let Err(error) = app.instances.stop(app, instance_id, EndReason::Stop).await {
        warn!(%instance_id, %error, "oneshot instance not stopped");
    }
}

async fn cleanup(app: &Arc<App>, held: Held) {
    if let Some(context_id) = &held.context_id {
        dispose(app, &held.instance_id, context_id).await;
    } else if let Some(tab_id) = &held.tab_id {
        close_tab(app, &held.instance_id, tab_id).await;
    }

    match held.origin {
        Origin::Dedicated => stop_instance(app, &held.instance_id).await,
        Origin::Pooled => app.oneshot.release(app, &held.instance_id),
        Origin::Persistent => {}
    }
}

impl Pool {
    fn count(&self, profile_id: &ProfileId) -> usize {
        self.members
            .values()
            .filter(|member| member.profile_id == *profile_id)
            .count()
    }

    fn track(&mut self, instance_id: InstanceId, profile_id: ProfileId, contexts: usize) {
        self.members.insert(
            instance_id,
            Member {
                profile_id,
                contexts,
                linger: None,
                started: Instant::now(),
            },
        );
    }

    // Whether the Instance was one of ours. A crash soon after the start delays the refill.
    fn forget(&mut self, instance_id: &InstanceId, crashed: bool, now: Instant) -> bool {
        let Some(member) = self.members.remove(instance_id) else {
            return false;
        };

        if crashed && now.saturating_duration_since(member.started) < POOL_STABLE {
            let doublings = self.crashes.min(MAX_BACKOFF_DOUBLINGS);
            let backoff = (POOL_BACKOFF_BASE * 2u32.pow(doublings)).min(POOL_BACKOFF_MAX);
            self.crashes += 1;
            self.refill_at = Some(now + backoff);
        } else {
            self.crashes = 0;
        }

        true
    }

    // The bus can drop events (a lagging receiver): the Instances themselves say who is left.
    fn retain_alive(&mut self, alive: impl Fn(&InstanceId) -> bool) {
        self.members.retain(|instance_id, _| alive(instance_id));
    }

    fn backing_off(&self, now: Instant) -> Option<Instant> {
        self.refill_at.filter(|at| *at > now)
    }

    // The member of the Profile with the fewest open contexts and room for another Tab; its
    // context is counted here, so a linger timer can never stop it under the request.
    // `open_tabs` is None for an Instance that cannot take a Tab. Contexts still opening are not
    // listed tabs yet, so the larger of the two counts is the one that must stay below the limit.
    fn reserve(
        &mut self,
        profile_id: &ProfileId,
        max_tabs: usize,
        open_tabs: impl Fn(&InstanceId) -> Option<usize>,
    ) -> Option<InstanceId> {
        let (instance_id, member) = self
            .members
            .iter_mut()
            .filter(|(_, member)| member.profile_id == *profile_id)
            .filter(|(instance_id, member)| {
                open_tabs(instance_id).is_some_and(|open| open.max(member.contexts + 1) < max_tabs)
            })
            .min_by_key(|(_, member)| member.contexts)?;

        member.contexts += 1;
        if let Some(linger) = member.linger.take() {
            linger.abort();
        }

        Some(*instance_id)
    }

    // Whether the Instance is left without contexts.
    fn release(&mut self, instance_id: &InstanceId) -> bool {
        let Some(member) = self.members.get_mut(instance_id) else {
            return false;
        };

        member.contexts = member.contexts.saturating_sub(1);

        member.contexts == 0
    }

    fn set_linger(&mut self, instance_id: &InstanceId, linger: AbortHandle) {
        if let Some(member) = self.members.get_mut(instance_id) {
            member.linger = Some(linger);
        }
    }

    // A member without contexts goes when it is not one of the pool's, or when the pool is
    // larger than it should be; it is forgotten here so no request can pick it afterwards.
    fn expire(
        &mut self,
        instance_id: &InstanceId,
        pool_profile: Option<&ProfileId>,
        pool_size: usize,
    ) -> bool {
        let Some(member) = self.members.get(instance_id) else {
            return false;
        };
        let pooled = pool_profile == Some(&member.profile_id);
        let surplus = !pooled || self.count(&member.profile_id) > pool_size;

        if member.contexts > 0 || !surplus {
            return false;
        }

        self.members.remove(instance_id).is_some()
    }

    // The pool's idle members beyond `pool_size`.
    fn trim(&mut self, pool_profile: &ProfileId, pool_size: usize) -> Vec<InstanceId> {
        let surplus = self.count(pool_profile).saturating_sub(pool_size);
        let idle: Vec<InstanceId> = self
            .members
            .iter()
            .filter(|(_, member)| member.profile_id == *pool_profile && member.contexts == 0)
            .map(|(instance_id, _)| *instance_id)
            .take(surplus)
            .collect();

        for instance_id in &idle {
            self.members.remove(instance_id);
        }

        idle
    }
}

impl Oneshot {
    pub fn close(&self) {
        self.lock().closed = true;
    }

    fn lock(&self) -> MutexGuard<'_, Pool> {
        self.pool.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn reserve(&self, app: &Arc<App>, profile_id: ProfileId) -> Option<InstanceId> {
        let max_tabs = app.settings().limits.max_tabs_per_instance as usize;

        self.lock().reserve(&profile_id, max_tabs, |instance_id| {
            let view = app.instances.view(instance_id)?;
            let running = matches!(
                view.instance.state,
                InstanceState::Running | InstanceState::Idle
            );

            running.then(|| {
                view.tabs
                    .iter()
                    .filter(|tab| tab.state != TabState::Closed)
                    .count()
            })
        })
    }

    fn release(&self, app: &Arc<App>, instance_id: &InstanceId) {
        let mut pool = self.lock();
        if !pool.release(instance_id) {
            return;
        }

        let linger_s = app.settings().oneshot.linger_s;
        let app = Arc::downgrade(app);
        let instance_id = *instance_id;
        let timer = tokio::spawn(async move {
            time::sleep(Duration::from_secs(linger_s)).await;
            if let Some(app) = app.upgrade() {
                expire(&app, instance_id).await;
            }
        });
        pool.set_linger(&instance_id, timer.abort_handle());
    }
}

async fn expire(app: &Arc<App>, instance_id: InstanceId) {
    let pool_profile = oneshot_profile(app).await.map(|found| found.id);
    let pool_size = app.settings().oneshot.pool_size as usize;
    let due = app
        .oneshot
        .lock()
        .expire(&instance_id, pool_profile.as_ref(), pool_size);

    if due {
        stop_instance(app, &instance_id).await;
    }
}
