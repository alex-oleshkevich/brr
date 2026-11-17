use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use brr_core::{Display, Error, InstanceId, ProfileSettings, Size};
use brr_proto::frames::{Frame, FrameKind, Link, MAX_PAYLOAD, Side, Stream, write_frame};
use tempfile::TempDir;
use test_case::test_case;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::files::{self, PUT_DONE, Request};
use super::{
    BUILD_KIND, DockerTarget, KIND_LABEL, MountRoot, MountSource, OWNER_LABEL, RUN_KIND,
    RUN_VOLUME_PREFIX, RUNNER_SUBCOMMAND, RunnerSpec, SandboxSpec, command, copy_extensions,
    forget_host, hand_over, helper_mount, mounts, once, route, runner_spec,
};
use crate::LaunchSpec;

const ROOT: &str = "/data";
const HOST_ROOT: &str = "/srv/brr";
const VOLUME: &str = "brr-data";
const IMAGE: &str = "brr-sandbox:1";
const RUNNER: &str = "/usr/local/bin/brr";
const OWNER: &str = "daemon-1";
const USER: &str = "1000:1000";
const HOST_URL: &str = "ssh://alex@host";
// Their own Hosts: the cache of checked Hosts is shared by every test.
const FORGOTTEN_URL: &str = "ssh://alex@forgotten";
const KEPT_URL: &str = "ssh://alex@forgotten-not";
const IMAGE_KEY: &str = "image";
const BROWSER: &str = "/opt/chrome/chrome";
const BROWSER_DIR: &str = "/opt/chrome";
const BROWSER_ARG: &str = "--headless";
const SET_VAR: &str = "BRR_TEST_SET";
const SET_VALUE: &str = "1";
const REMOVED_VAR: &str = "BRR_TEST_REMOVED";
const SCREEN: Size = Size {
    width: 1280,
    height: 720,
};
const MANIFEST: &str = "manifest.json";
const RULES: &str = "rules/filters.json";
const RULES_JSON: &str = "[]";
const COOKIES: &str = "Cookies";
const OLD_COOKIES: &str = "before the run";
const NEW_COOKIES: &str = "after the run";
const TRANSPORT_BYTES: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);
// The marker entry and the two end blocks: what is left ends cleanly after the last real entry.
const MARKER_AND_END: usize = 3 * 512;

fn root(path: &str, source: MountSource) -> MountRoot {
    MountRoot {
        path: PathBuf::from(path),
        source,
    }
}

fn bind() -> MountSource {
    MountSource::Bind(PathBuf::from(HOST_ROOT))
}

fn volume() -> MountSource {
    MountSource::Volume(VOLUME.to_owned())
}

#[test_case(bind(), "/data/profiles/a", false, "type=bind,src=/srv/brr/profiles/a,dst=/data/profiles/a"; "bind below the root")]
#[test_case(bind(), "/data", false, "type=bind,src=/srv/brr,dst=/data"; "bind at the root")]
#[test_case(bind(), "/data/builds/c", true, "type=bind,src=/srv/brr/builds/c,dst=/data/builds/c,readonly"; "bind read-only")]
#[test_case(volume(), "/data/runs/x", false, "type=volume,src=brr-data,dst=/data/runs/x,volume-subpath=runs/x"; "volume with subpath")]
#[test_case(volume(), "/data", false, "type=volume,src=brr-data,dst=/data"; "volume without subpath")]
#[test_case(volume(), "/data/fonts", true, "type=volume,src=brr-data,dst=/data/fonts,volume-subpath=fonts,readonly"; "volume read-only")]
fn a_path_is_mounted_from_its_root(
    source: MountSource,
    path: &str,
    readonly: bool,
    expected: &str,
) {
    let roots = [root(ROOT, source)];

    let mounts = mounts(&roots, &[(Path::new(path), readonly)]).unwrap();

    assert_eq!(mounts, [expected]);
}

#[test]
fn the_longest_root_wins() {
    let roots = [root(ROOT, bind()), root("/data/builds", volume())];

    let mounts = mounts(&roots, &[(Path::new("/data/builds/c"), true)]).unwrap();

    assert_eq!(
        mounts,
        ["type=volume,src=brr-data,dst=/data/builds/c,volume-subpath=c,readonly"]
    );
}

#[test]
fn a_path_under_no_root_is_not_mounted() {
    let roots = [root(ROOT, bind())];

    let mounts = mounts(&roots, &[(Path::new("/usr/lib/chromium"), true)]).unwrap();

    assert!(mounts.is_empty());
}

#[test_case("/usr/bin"; "usr")]
#[test_case("/etc/chromium"; "etc")]
fn a_system_dir_is_never_mounted_over_the_image(path: &str) {
    let roots = [root("/", MountSource::Bind(PathBuf::from("/")))];

    let mounts = mounts(&roots, &[(Path::new(path), true)]).unwrap();

    assert!(mounts.is_empty());
}

#[test]
fn a_host_root_mounts_a_browser_outside_the_system_dirs() {
    let roots = [root("/", MountSource::Bind(PathBuf::from("/")))];

    let mounts = mounts(&roots, &[(Path::new("/opt/google/chrome"), true)]).unwrap();

    assert_eq!(
        mounts,
        ["type=bind,src=/opt/google/chrome,dst=/opt/google/chrome,readonly"]
    );
}

#[test]
fn a_path_wanted_twice_is_mounted_once_read_write() {
    let roots = [root(ROOT, bind())];
    let path = Path::new("/data/profiles/a");

    let mounts = mounts(&roots, &[(path, true), (path, false), (path, true)]).unwrap();

    assert_eq!(
        mounts,
        ["type=bind,src=/srv/brr/profiles/a,dst=/data/profiles/a"]
    );
}

#[test_case("/data/a,b"; "comma")]
#[test_case("/data/a\"b"; "quote")]
fn a_path_that_breaks_the_mount_syntax_is_refused(path: &str) {
    let roots = [root(ROOT, bind())];

    let error = mounts(&roots, &[(Path::new(path), false)]).unwrap_err();

    assert!(matches!(error, Error::InvalidArgument { .. }));
}

struct Fixture {
    _dir: TempDir,
    spec: LaunchSpec,
    sandbox: SandboxSpec,
    // An extension in the shared library; the launch loads `spec.extensions`, its copies.
    library: PathBuf,
    runner_spec: RunnerSpec,
    args: Vec<String>,
}

fn library_extension(dir: &Path) -> PathBuf {
    let library = dir.join("extensions").join("ublock");
    std::fs::create_dir_all(library.join("rules")).unwrap();
    std::fs::write(library.join(MANIFEST), "{}").unwrap();
    std::fs::write(library.join(RULES), RULES_JSON).unwrap();

    library
}

fn fixture_on(docker_url: Option<&str>, copy_back: bool) -> Fixture {
    let dir = TempDir::new().unwrap();
    let mut spec = crate::tests::spec(Display::Virtual, ProfileSettings::default());
    spec.executable = PathBuf::from(BROWSER);
    spec.user_data_dir = dir.path().join("profile");
    spec.download_dir = dir.path().join("downloads");
    spec.log_file = dir.path().join("logs").join("browser.log");
    let run_dir = dir.path().join("run");
    let library = library_extension(dir.path());
    spec.extensions = copy_extensions(&run_dir, std::slice::from_ref(&library)).unwrap();
    let sandbox = SandboxSpec {
        run_dir,
        docker: DockerTarget {
            image: IMAGE.to_owned(),
            runner: PathBuf::from(RUNNER),
            roots: vec![MountRoot {
                path: dir.path().to_owned(),
                source: MountSource::Bind(dir.path().to_owned()),
            }],
            user: USER.to_owned(),
            memory_mb: 2048,
            cpus: 2.0,
            pids: 1024,
            shm_mb: 1024,
            owner: OWNER.to_owned(),
            docker_url: docker_url.map(str::to_owned),
        },
        copy_back,
    };
    let mut browser = Command::new(BROWSER);
    browser
        .arg(BROWSER_ARG)
        .env(SET_VAR, SET_VALUE)
        .env_remove(REMOVED_VAR);

    let runner_spec = runner_spec(&spec, &sandbox, &browser, Some(SCREEN)).unwrap();
    let docker = command(&spec, &sandbox).unwrap();
    let args = docker
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();

    Fixture {
        _dir: dir,
        spec,
        sandbox,
        library,
        runner_spec,
        args,
    }
}

fn fixture() -> Fixture {
    fixture_on(None, false)
}

fn remote() -> Fixture {
    fixture_on(Some(HOST_URL), true)
}

#[test_case(&["--network", "none"]; "no network")]
#[test_case(&["--read-only"]; "read-only rootfs")]
#[test_case(&["--cap-drop", "ALL"]; "no capabilities")]
#[test_case(&["--security-opt", "no-new-privileges"]; "no new privileges")]
#[test_case(&["--user", USER]; "the daemon's user")]
#[test_case(&["--entrypoint", RUNNER]; "runner entrypoint")]
fn the_container_is_confined(wanted: &[&str]) {
    for fixture in [fixture(), remote()] {
        assert!(
            fixture
                .args
                .windows(wanted.len())
                .any(|window| window == wanted),
            "{wanted:?} not in {:?}",
            fixture.args
        );
    }
}

#[test]
fn the_image_runs_the_runner() {
    let fixture = fixture();

    assert_eq!(fixture.args[0], "run");
    assert_eq!(fixture.args[fixture.args.len() - 2], IMAGE);
    assert_eq!(fixture.args[fixture.args.len() - 1], RUNNER_SUBCOMMAND);
}

#[test]
fn the_container_carries_its_owner() {
    let fixture = fixture();
    let label = format!("{OWNER_LABEL}={OWNER}");

    assert!(
        fixture
            .args
            .windows(2)
            .any(|window| window[0] == "--label" && window[1] == label)
    );
}

#[test]
fn the_writable_dirs_are_mounted_read_write() {
    let fixture = fixture();
    let run_dir = fixture.sandbox.run_dir.to_string_lossy().into_owned();

    assert!(
        fixture
            .args
            .contains(&format!("type=bind,src={run_dir},dst={run_dir}"))
    );
}

#[test]
fn the_launch_loads_copies_of_its_extensions_in_the_run_dir() {
    let fixture = fixture();
    let extensions = &fixture.spec.extensions;

    assert_eq!(
        *extensions,
        [fixture.sandbox.run_dir.join("extensions").join("0")]
    );
    assert_eq!(
        std::fs::read_to_string(extensions[0].join(RULES)).unwrap(),
        RULES_JSON
    );
    assert!(extensions[0].join(MANIFEST).is_file());
}

#[test]
fn the_extension_library_is_not_mounted() {
    let fixture = fixture();
    let library = fixture.library.to_string_lossy().into_owned();
    let copy = fixture.spec.extensions[0].to_string_lossy().into_owned();

    assert!(
        !fixture
            .args
            .iter()
            .any(|arg| arg.contains(&library) || arg.contains(&copy)),
        "{:?}",
        fixture.args
    );
}

#[test]
fn an_extension_with_a_link_is_not_copied() {
    let dir = TempDir::new().unwrap();
    let library = library_extension(dir.path());
    std::os::unix::fs::symlink("/etc/passwd", library.join("passwd")).unwrap();

    let result = copy_extensions(&dir.path().join("run"), &[library]);

    assert!(matches!(result, Err(Error::Internal { .. })), "{result:?}");
}

#[test]
fn the_runner_spec_carries_only_what_the_browser_command_set() {
    let fixture = fixture();
    let spec = &fixture.runner_spec;

    assert_eq!(
        spec.env,
        BTreeMap::from([(SET_VAR.to_owned(), SET_VALUE.to_owned())])
    );
    assert_eq!(spec.argv, [BROWSER, BROWSER_ARG]);
    assert_eq!(spec.xvfb, Some(SCREEN));
    assert_eq!(spec.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        spec.writable,
        [
            fixture.spec.user_data_dir.clone(),
            fixture.spec.download_dir.clone(),
            fixture.sandbox.run_dir.clone()
        ]
    );
}

#[test_case(None, true, false, false; "local runs at once")]
#[test_case(Some(HOST_URL), false, true, false; "remote ephemeral waits")]
#[test_case(Some(HOST_URL), true, true, true; "remote persistent copies back")]
fn the_runner_spec_says_how_files_travel(
    docker_url: Option<&str>,
    copy_back: bool,
    wait_for_files: bool,
    copies_back: bool,
) {
    let fixture = fixture_on(docker_url, copy_back);
    let spec = &fixture.runner_spec;

    assert_eq!(spec.wait_for_files, wait_for_files);
    assert_eq!(
        spec.copy_back,
        copies_back.then(|| fixture.spec.user_data_dir.clone())
    );
}

#[test]
fn a_remote_run_talks_to_the_host() {
    let fixture = remote();

    assert_eq!(fixture.args[..3], ["-H", HOST_URL, "run"]);
}

#[test_case(|fixture| fixture.spec.user_data_dir.clone(), "profile"; "profile")]
#[test_case(|fixture| fixture.spec.download_dir.clone(), "downloads"; "downloads")]
#[test_case(|fixture| fixture.sandbox.run_dir.clone(), "run"; "run dir")]
fn a_remote_run_mounts_its_volume_at_brr_paths(dir: fn(&Fixture) -> PathBuf, subpath: &str) {
    let fixture = remote();
    let mount = format!(
        "type=volume,src={RUN_VOLUME_PREFIX}{},dst={},volume-subpath={subpath}",
        fixture.spec.instance_id,
        dir(&fixture).display()
    );

    assert!(fixture.args.contains(&mount), "{:?}", fixture.args);
}

#[test]
fn a_remote_run_mounts_the_browser_from_a_build_volume() {
    let fixture = remote();
    let suffix = format!(",dst={BROWSER_DIR},readonly");

    assert!(
        fixture
            .args
            .iter()
            .any(|arg| arg.starts_with("type=volume,src=brr-build-") && arg.ends_with(&suffix)),
        "{:?}",
        fixture.args
    );
}

#[test]
fn a_remote_run_mounts_no_path_of_brr() {
    let fixture = remote();

    assert!(
        !fixture.args.iter().any(|arg| arg.contains("type=bind")),
        "{:?}",
        fixture.args
    );
}

fn link_pair() -> (Link, mpsc::Receiver<Stream>) {
    let (daemon, runner) = tokio::io::duplex(TRANSPORT_BYTES);
    let (reader, writer) = tokio::io::split(daemon);
    let (link, _, _) = Link::start(reader, writer, Side::Daemon);
    let (reader, writer) = tokio::io::split(runner);
    let (_, _, accepted) = Link::start(reader, writer, Side::Runner);

    (link, accepted)
}

// Records what brr opens and unpacks each put into `received/<its position>`.
async fn fake_runner(
    mut accepted: mpsc::Receiver<Stream>,
    received: PathBuf,
) -> (Vec<Request>, RunnerSpec) {
    let mut seen = Vec::new();
    let mut spec = None;
    while let Some(mut stream) = accepted.recv().await {
        let request: Request = serde_json::from_slice(stream.request()).unwrap();
        seen.push(request.clone());
        match request {
            Request::Spec => {
                let mut json = Vec::new();
                stream.read_to_end(&mut json).await.unwrap();
                let runner_spec: RunnerSpec = serde_json::from_slice(&json).unwrap();
                let waits = runner_spec.wait_for_files;
                spec = Some(runner_spec);
                if !waits {
                    break;
                }
            }
            Request::Put(_) => {
                let (reader, mut answer) = tokio::io::split(stream);
                files::unpack(reader, received.join(seen.len().to_string()))
                    .await
                    .unwrap();
                answer.write_all(PUT_DONE.as_bytes()).await.unwrap();
                answer.shutdown().await.unwrap();
            }
            _ => break,
        }
    }

    (seen, spec.unwrap())
}

#[tokio::test]
async fn a_remote_run_gets_its_files_before_the_start() {
    let fixture = remote();
    std::fs::write(fixture.spec.user_data_dir.join(COOKIES), OLD_COOKIES).unwrap();
    let received = TempDir::new().unwrap();
    let (link, accepted) = link_pair();
    let runner = tokio::spawn(fake_runner(accepted, received.path().to_owned()));

    timeout(
        TIMEOUT,
        hand_over(&link, &fixture.spec, &fixture.sandbox, &fixture.runner_spec),
    )
    .await
    .unwrap()
    .unwrap();
    let (seen, spec) = timeout(TIMEOUT, runner).await.unwrap().unwrap();

    assert_eq!(
        seen,
        [
            Request::Spec,
            Request::Put(fixture.spec.user_data_dir.clone()),
            Request::Put(fixture.sandbox.run_dir.clone()),
            Request::Start
        ]
    );
    assert_eq!(spec, fixture.runner_spec);
    assert_eq!(
        std::fs::read_to_string(received.path().join("2").join(COOKIES)).unwrap(),
        OLD_COOKIES
    );
    assert!(
        received
            .path()
            .join("3")
            .join("extensions")
            .join("0")
            .join(MANIFEST)
            .is_file()
    );
}

#[tokio::test]
async fn a_local_run_gets_only_its_spec() {
    let fixture = fixture();
    let received = TempDir::new().unwrap();
    let (link, accepted) = link_pair();
    let runner = tokio::spawn(fake_runner(accepted, received.path().to_owned()));

    timeout(
        TIMEOUT,
        hand_over(&link, &fixture.spec, &fixture.sandbox, &fixture.runner_spec),
    )
    .await
    .unwrap()
    .unwrap();
    let (seen, _) = timeout(TIMEOUT, runner).await.unwrap().unwrap();

    assert_eq!(seen, [Request::Spec]);
}

fn frame(stream: u32, kind: FrameKind, payload: &[u8]) -> Frame {
    Frame {
        stream,
        kind,
        payload: payload.to_vec().into(),
    }
}

// The runner's side is written frame by frame so the test ends the connection when it wants.
async fn copy_back(profile: &Path, archive: &[u8]) {
    let (daemon, mut runner) = tokio::io::duplex(TRANSPORT_BYTES);
    let (reader, writer) = tokio::io::split(daemon);
    let (_link, _pipe, opened) = Link::start(reader, writer, Side::Daemon);
    let (tunnels, _tunnels) = mpsc::channel(1);
    let routed = tokio::spawn(route(
        opened,
        tunnels,
        Some(profile.to_owned()),
        InstanceId::new(),
    ));

    let request = serde_json::to_vec(&Request::Profile).unwrap();
    write_frame(&mut runner, &frame(1, FrameKind::Open, &request))
        .await
        .unwrap();
    for chunk in archive.chunks(MAX_PAYLOAD) {
        write_frame(&mut runner, &frame(1, FrameKind::Data, chunk))
            .await
            .unwrap();
    }
    write_frame(&mut runner, &frame(1, FrameKind::Close, &[]))
        .await
        .unwrap();
    drop(runner);

    timeout(TIMEOUT, routed).await.unwrap().unwrap();
}

async fn profiles() -> (TempDir, PathBuf, Vec<u8>) {
    let dir = TempDir::new().unwrap();
    let profile = dir.path().join("profile");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(profile.join(COOKIES), OLD_COOKIES).unwrap();
    let after = dir.path().join("after");
    std::fs::create_dir_all(&after).unwrap();
    std::fs::write(after.join(COOKIES), NEW_COOKIES).unwrap();
    let archive = files::pack(after, Vec::new()).await.unwrap();

    (dir, profile, archive)
}

fn leftovers(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();

    names
}

#[test_case(0, NEW_COOKIES; "a complete copy replaces the Profile")]
#[test_case(MARKER_AND_END, OLD_COOKIES; "a cut copy leaves the Profile")]
#[tokio::test]
async fn the_profile_comes_back_only_whole(cut: usize, expected: &str) {
    let (dir, profile, archive) = profiles().await;

    copy_back(&profile, &archive[..archive.len() - cut]).await;

    assert_eq!(
        std::fs::read_to_string(profile.join(COOKIES)).unwrap(),
        expected
    );
    assert_eq!(leftovers(dir.path()), ["after", "profile"]);
}

async fn check(url: &str, runs: &AtomicUsize) {
    once(format!("{url} {IMAGE_KEY}"), async {
        runs.fetch_add(1, Ordering::Relaxed);
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_forgotten_host_is_checked_again() {
    let forgotten = AtomicUsize::new(0);
    let kept = AtomicUsize::new(0);
    check(FORGOTTEN_URL, &forgotten).await;
    check(KEPT_URL, &kept).await;

    forget_host(FORGOTTEN_URL);
    check(FORGOTTEN_URL, &forgotten).await;
    check(KEPT_URL, &kept).await;

    assert_eq!(forgotten.load(Ordering::Relaxed), 2);
    assert_eq!(kept.load(Ordering::Relaxed), 1);
}

#[test_case(RUN_KIND; "run volume")]
#[test_case(BUILD_KIND; "build volume")]
fn a_host_volume_carries_its_owner_and_kind(kind: &str) {
    let mount = helper_mount(VOLUME, kind, OWNER).unwrap();

    assert!(
        mount.starts_with(&format!("type=volume,src={VOLUME},")),
        "{mount}"
    );
    assert!(
        mount.contains(&format!(",volume-label={OWNER_LABEL}={OWNER}")),
        "{mount}"
    );
    assert!(
        mount.ends_with(&format!(",volume-label={KIND_LABEL}={kind}")),
        "{mount}"
    );
}
