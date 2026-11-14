use brr_core::{Display, Engine, Settings, settings::LimitMode};

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
