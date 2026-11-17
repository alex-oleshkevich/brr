use std::collections::{HashMap, HashSet};

use brr_core::{
    AdmissionMode, Display, Engine, Host, HostId, HostStatus, InstanceId, SandboxMode, Settings,
    settings::{EngineLimits, LimitMode},
};
use test_case::test_case;

use super::{
    Admission, AdmitRequest, Local, commit,
    decide::{Class, HostLoad, Place, Snapshot, Verdict, class, decide, place},
    evaluate, ewma, host_load,
};

const MIN_FREE_MB: u64 = 1024;
const RESERVED_MB: u64 = 500;
const ESTIMATE_MB: u64 = 400;
const SCRAPE: &str = "scrape";
const HOST_MAX: u32 = 2;

fn static_settings() -> Settings {
    let mut settings = Settings::default();
    settings.limits.max_instances = 2;
    settings.limits.max_instances_per_engine = EngineLimits {
        chromium: 1,
        camoufox: 5,
    };
    settings
        .limits
        .max_instances_per_tag
        .insert(SCRAPE.to_owned(), 1);

    settings
}

fn adaptive_settings() -> Settings {
    let mut settings = Settings::default();
    settings.limits.mode = LimitMode::Adaptive;
    settings.limits.max_instances = 10;
    settings.limits.max_instances_per_engine = EngineLimits {
        chromium: 10,
        camoufox: 10,
    };
    settings.adaptive.min_free_ram_mb = MIN_FREE_MB;
    settings.adaptive.max_load_per_core = 0.85;

    settings
}

fn snapshot<'a>(
    settings: &'a Settings,
    admitted: &'a [(Engine, Vec<String>)],
    free_ram_mb: u64,
    load_per_core: f64,
) -> Snapshot<'a> {
    Snapshot {
        settings,
        admitted,
        free_ram_mb,
        reserved_mb: RESERVED_MB,
        load_per_core,
        estimate_mb: ESTIMATE_MB,
    }
}

#[test]
fn static_ceilings() {
    let settings = static_settings();
    let one_chromium = [(Engine::Chromium, Vec::new())];
    let one_scrape = [(Engine::Chromium, vec![SCRAPE.to_owned()])];
    let scrape = [SCRAPE.to_owned()];

    let chromium_full = snapshot(&settings, &one_chromium, 0, 0.0);
    assert!(matches!(
        decide(&chromium_full, Engine::Chromium, &[]),
        Verdict::Full { .. }
    ));

    let tag_full = snapshot(&settings, &one_scrape, 0, 0.0);
    assert!(matches!(
        decide(&tag_full, Engine::Camoufox, &scrape),
        Verdict::Full { .. }
    ));

    let empty = snapshot(&settings, &[], 0, 0.0);
    assert_eq!(decide(&empty, Engine::Chromium, &scrape), Verdict::Admit);
}

#[test]
fn total_ceiling() {
    let settings = static_settings();
    let two = [
        (Engine::Camoufox, Vec::new()),
        (Engine::Camoufox, Vec::new()),
    ];

    let full = snapshot(&settings, &two, 0, 0.0);

    assert!(matches!(
        decide(&full, Engine::Camoufox, &[]),
        Verdict::Full { .. }
    ));
}

#[test_case(2000, 0.5, true; "enough memory")]
#[test_case(1900, 0.5, false; "below the floor")]
#[test_case(2000, 0.9, false; "too much load")]
fn adaptive_formula(free_ram_mb: u64, load_per_core: f64, admitted: bool) {
    let settings = adaptive_settings();

    let verdict = decide(
        &snapshot(&settings, &[], free_ram_mb, load_per_core),
        Engine::Chromium,
        &[],
    );

    assert_eq!(verdict == Verdict::Admit, admitted);
}

#[test]
fn adaptive_keeps_static_ceilings() {
    let mut settings = adaptive_settings();
    settings.limits.max_instances = 1;
    let one = [(Engine::Chromium, Vec::new())];

    let verdict = decide(
        &snapshot(&settings, &one, 100_000, 0.0),
        Engine::Chromium,
        &[],
    );

    assert!(matches!(verdict, Verdict::Full { .. }));
}

#[test_case(Engine::Chromium, Display::Headless, Class::ChromiumHeadless)]
#[test_case(Engine::Chromium, Display::Visible, Class::ChromiumDisplay)]
#[test_case(Engine::Chromium, Display::Virtual, Class::ChromiumDisplay)]
#[test_case(Engine::Camoufox, Display::Headless, Class::Camoufox)]
fn classes(engine: Engine, display: Display, expected: Class) {
    assert_eq!(class(engine, display), expected);
}

#[test]
fn ewma_moves_toward_the_sample() {
    assert_eq!(ewma(400, 1000), 580);
    assert_eq!(ewma(400, 400), 400);
}

fn request() -> AdmitRequest {
    AdmitRequest {
        sandbox: SandboxMode::Docker,
        instance_id: InstanceId::new(),
        profile_name: "p".to_owned(),
        engine: Engine::Chromium,
        display: Display::Headless,
        tags: Vec::new(),
        priority: 0,
        mode: AdmissionMode::Evict,
        queued_at: 0,
    }
}

// A victim past its reservation hold: its memory sits in MemAvailable as used, so only the
// measured process tree (or the class estimate) lets an adaptive request in.
#[test_case(Some(ESTIMATE_MB + MIN_FREE_MB), Verdict::Admit; "measured tree frees enough")]
#[test_case(Some(0), Verdict::Full { reason: String::new() }; "tree too small")]
#[test_case(None, Verdict::Admit; "estimate stands in for a missing measurement")]
fn evicting_a_victim_credits_its_memory(tree_mb: Option<u64>, expected: Verdict) {
    let settings = adaptive_settings();
    let admission = Admission::new(&settings);
    let mut state = admission.state.into_inner();
    let victim = request();
    commit(&mut state, &victim, Place::Local);
    state.reservations.clear();
    let estimate_mb = state.estimates.get(Class::ChromiumHeadless);
    let waiting = request();
    let mut local = Local {
        free_ram_mb: MIN_FREE_MB + estimate_mb - 1,
        load_per_core: 0.1,
        tree_mb: HashMap::new(),
    };
    local
        .tree_mb
        .extend(tree_mb.map(|mb| (victim.instance_id, mb)));

    let (kept, _) = evaluate(&state, &settings, &local, &waiting, &HashSet::new(), &[]);
    let removed = HashSet::from([victim.instance_id]);
    let (verdict, inputs) = evaluate(&state, &settings, &local, &waiting, &removed, &[]);

    assert!(matches!(kept, Verdict::Full { .. }));
    assert_eq!(
        std::mem::discriminant(&verdict),
        std::mem::discriminant(&expected)
    );
    assert_eq!(inputs.free_ram_mb, local.free_ram_mb);
}

// One Host as the placement table describes it.
#[derive(Clone, Copy)]
struct Spec {
    reachable: bool,
    enabled: bool,
    mem_mb: Option<u64>,
    placed: u32,
    reserved_mb: u64,
}

const fn up(mem_mb: u64) -> Spec {
    Spec {
        reachable: true,
        enabled: true,
        mem_mb: Some(mem_mb),
        placed: 0,
        reserved_mb: 0,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Want {
    Nowhere,
    Local,
    Host(usize),
}

#[test_case(SandboxMode::Docker, Some(4000), &[up(8000)], Want::Host(0); "host with more memory")]
#[test_case(SandboxMode::Docker, Some(8000), &[up(4000)], Want::Local; "local with more memory")]
#[test_case(SandboxMode::Docker, Some(4000), &[up(4000)], Want::Local; "tie goes to local")]
#[test_case(SandboxMode::Docker, Some(1000), &[up(2000), up(6000), up(3000)], Want::Host(1); "most memory of several hosts")]
#[test_case(SandboxMode::Docker, None, &[Spec { placed: HOST_MAX, ..up(8000) }, up(2000)], Want::Host(1); "full host skipped")]
#[test_case(SandboxMode::Docker, None, &[Spec { placed: HOST_MAX, ..up(8000) }], Want::Nowhere; "only host full")]
#[test_case(SandboxMode::Docker, Some(1000), &[Spec { reachable: false, ..up(16000) }], Want::Local; "unreachable host skipped")]
#[test_case(SandboxMode::Docker, Some(1000), &[Spec { enabled: false, ..up(16000) }], Want::Local; "disabled host skipped")]
#[test_case(SandboxMode::Docker, Some(2000), &[Spec { reserved_mb: 7000, ..up(8000) }], Want::Local; "host reservations count")]
#[test_case(SandboxMode::Docker, None, &[Spec { mem_mb: None, ..up(0) }], Want::Host(0); "host memory unknown still has room")]
#[test_case(SandboxMode::Docker, None, &[], Want::Nowhere; "local full without hosts")]
#[test_case(SandboxMode::None, Some(1000), &[up(8000)], Want::Local; "sandbox none keeps local")]
#[test_case(SandboxMode::None, None, &[up(8000)], Want::Nowhere; "sandbox none never uses hosts")]
fn placement_table(mode: SandboxMode, local_mb: Option<u64>, specs: &[Spec], want: Want) {
    let hosts: Vec<Host> = specs
        .iter()
        .enumerate()
        .map(|(n, spec)| Host {
            id: HostId::new(),
            name: format!("box-{n}"),
            docker_url: format!("ssh://brr@box-{n}"),
            max_instances: HOST_MAX,
            enabled: spec.enabled,
            created_at: 0,
            status: HostStatus {
                reachable: spec.reachable,
                mem_available_mb: spec.mem_mb,
                ..HostStatus::default()
            },
        })
        .collect();
    let load = |host_id: &HostId| {
        let n = hosts.iter().position(|host| host.id == *host_id).unwrap();

        HostLoad {
            placed: specs[n].placed,
            reserved_mb: specs[n].reserved_mb,
        }
    };

    let got = match place(mode, local_mb, &hosts, load) {
        None => Want::Nowhere,
        Some(Place::Local) => Want::Local,
        Some(Place::Host(id)) => Want::Host(hosts.iter().position(|host| host.id == id).unwrap()),
    };

    assert_eq!(got, want);
}

// A Host's runs take its own room, never the local limits'.
#[test]
fn host_runs_leave_local_limits_alone() {
    let settings = static_settings();
    let admission = Admission::new(&settings);
    let mut state = admission.state.into_inner();
    let host_id = HostId::new();
    for _ in 0..settings.limits.max_instances {
        commit(&mut state, &request(), Place::Host(host_id));
    }
    let local = Local {
        free_ram_mb: 0,
        load_per_core: 0.0,
        tree_mb: HashMap::new(),
    };
    let estimate_mb = state.estimates.get(Class::ChromiumHeadless);

    let (verdict, inputs) = evaluate(&state, &settings, &local, &request(), &HashSet::new(), &[]);
    let load = host_load(&state, &host_id, estimate_mb);

    assert_eq!(verdict, Verdict::Admit);
    assert_eq!(inputs.counts["total"], 0);
    assert_eq!(load.placed, settings.limits.max_instances);
    assert_eq!(
        load.reserved_mb,
        estimate_mb * u64::from(settings.limits.max_instances + 1)
    );
}
