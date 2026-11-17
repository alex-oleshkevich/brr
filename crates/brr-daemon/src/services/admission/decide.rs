use brr_core::{Display, Engine, Host, HostId, SandboxMode, Settings, settings::LimitMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    ChromiumHeadless,
    ChromiumDisplay,
    Camoufox,
}

pub struct Snapshot<'a> {
    pub settings: &'a Settings,
    pub admitted: &'a [(Engine, Vec<String>)],
    pub free_ram_mb: u64,
    pub reserved_mb: u64,
    pub load_per_core: f64,
    pub estimate_mb: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Admit,
    Full { reason: String },
}

pub fn class(engine: Engine, display: Display) -> Class {
    match (engine, display) {
        (Engine::Camoufox, _) => Class::Camoufox,
        (Engine::Chromium, Display::Headless) => Class::ChromiumHeadless,
        (Engine::Chromium, Display::Visible | Display::Virtual) => Class::ChromiumDisplay,
    }
}

// The static maxima are ceilings in both modes; adaptive adds the memory and load checks (A1).
pub fn decide(s: &Snapshot<'_>, engine: Engine, tags: &[String]) -> Verdict {
    let limits = &s.settings.limits;

    if s.admitted.len() >= limits.max_instances as usize {
        return full(format!(
            "{} instances are running (limit {})",
            s.admitted.len(),
            limits.max_instances
        ));
    }

    let (engine_limit, engine_name) = match engine {
        Engine::Chromium => (limits.max_instances_per_engine.chromium, "chromium"),
        Engine::Camoufox => (limits.max_instances_per_engine.camoufox, "camoufox"),
    };
    let on_engine = s.admitted.iter().filter(|(e, _)| *e == engine).count();
    if on_engine >= engine_limit as usize {
        return full(format!(
            "{on_engine} {engine_name} instances are running (limit {engine_limit})"
        ));
    }

    for tag in tags {
        let Some(limit) = limits.max_instances_per_tag.get(tag) else {
            continue;
        };
        let with_tag = s.admitted.iter().filter(|(_, t)| t.contains(tag)).count();
        if with_tag >= *limit as usize {
            return full(format!(
                "{with_tag} instances tagged {tag} are running (limit {limit})"
            ));
        }
    }

    if limits.mode == LimitMode::Adaptive {
        let adaptive = &s.settings.adaptive;
        let needed = s.reserved_mb + s.estimate_mb + adaptive.min_free_ram_mb;
        if s.free_ram_mb < needed {
            return full(format!(
                "{} MB free, {needed} MB needed for reservations, this instance and the floor",
                s.free_ram_mb
            ));
        }
        if s.load_per_core > adaptive.max_load_per_core {
            return full(format!(
                "load per core {:.2} is above {:.2}",
                s.load_per_core, adaptive.max_load_per_core
            ));
        }
    }

    Verdict::Admit
}

fn full(reason: String) -> Verdict {
    Verdict::Full { reason }
}

// Where an admitted Instance runs: brr's own machine or a Host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Local,
    Host(HostId),
}

// What this daemon's admissions already put on one Host.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostLoad {
    pub placed: u32,
    // The reservations of its recent runs plus the candidate's own estimate.
    pub reserved_mb: u64,
}

// Local (when its limits admit, `local_mb` is its free memory after reservations) and every
// enabled, reachable Host below its max compete on free memory; the first of equals wins, so
// local wins ties. Without the docker sandbox only local runs browsers.
pub fn place(
    mode: SandboxMode,
    local_mb: Option<u64>,
    hosts: &[Host],
    load: impl Fn(&HostId) -> HostLoad,
) -> Option<Place> {
    let local = local_mb.map(|mb| (Place::Local, mb));
    if mode == SandboxMode::None {
        return local.map(|(place, _)| place);
    }

    let remote = hosts
        .iter()
        .filter(|host| host.enabled && host.status.reachable)
        .filter_map(|host| {
            let load = load(&host.id);
            let available_mb = host
                .status
                .mem_available_mb
                .unwrap_or(0)
                .saturating_sub(load.reserved_mb);

            (load.placed < host.max_instances).then_some((Place::Host(host.id), available_mb))
        });

    local
        .into_iter()
        .chain(remote)
        .fold(None, |best: Option<(Place, u64)>, candidate| match best {
            Some(best) if best.1 >= candidate.1 => Some(best),
            _ => Some(candidate),
        })
        .map(|(place, _)| place)
}
