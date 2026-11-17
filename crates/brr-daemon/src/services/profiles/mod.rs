use std::{collections::HashMap, path::PathBuf};

use brr_core::{
    Build, BuildChoice, BuildSource, BuildState, Channel, Display, Error, LoginStatus, Page, Paths,
    Profile, ProfileId, ProfileSettings, Settings, now_ms,
};
use brr_store::{Connection, Store, build, profile, template};

use crate::{internal, services::builds::current_for_channel};

pub(crate) const MAX_NAME_LEN: usize = 64;
const BUILTIN_TEMPLATES: [(&str, &str, BuildSource, &str, Display); 4] = [
    (
        "chromium-headless",
        "Chrome with no window, for agents and scripts.",
        BuildSource::Cft,
        "stable",
        Display::Headless,
    ),
    (
        "chromium-visible",
        "Chrome with a window you can watch and step into.",
        BuildSource::Cft,
        "stable",
        Display::Visible,
    ),
    (
        "camoufox-stealth",
        "Firefox-based browser that hides automation, with a virtual screen.",
        BuildSource::Camoufox,
        "latest-compatible",
        Display::Virtual,
    ),
    (
        "camoufox-headless",
        "Firefox-based stealth browser with no window.",
        BuildSource::Camoufox,
        "latest-compatible",
        Display::Headless,
    ),
];

pub async fn seed_templates(store: &Store) -> Result<(), Error> {
    store
        .run(|c| {
            let now = now_ms();

            for (name, description, source, channel, display) in BUILTIN_TEMPLATES {
                let settings = ProfileSettings {
                    build: BuildChoice::Channel(Channel {
                        source,
                        name: channel.to_string(),
                    }),
                    display,
                    ..ProfileSettings::default()
                };
                template::upsert_builtin(c, name, description, &settings, now)?;
            }

            Ok(())
        })
        .await
}

pub const DEFAULT_BACKUP_KEEP: u32 = 7;

pub fn resolve_build(c: &Connection, choice: &BuildChoice) -> Result<Build, Error> {
    pick_build(&build::all(c)?, choice)
}

fn pick_build(builds: &[Build], choice: &BuildChoice) -> Result<Build, Error> {
    let usable = |b: &Build| matches!(b.state, BuildState::Installed | BuildState::Registered);

    match choice {
        BuildChoice::BuildId(build_id) => builds
            .iter()
            .find(|b| b.id == *build_id && usable(b))
            .cloned()
            .ok_or_else(|| Error::BuildMissing {
                reason: format!("build {build_id} is not installed or registered"),
            }),
        BuildChoice::Channel(channel) => {
            let system = builds
                .iter()
                .find(|b| b.source == BuildSource::System && b.state == BuildState::Registered);
            let fallback = if channel.source == BuildSource::Cft {
                system
            } else {
                None
            };

            current_for_channel(builds, channel)
                .or_else(|| fallback.cloned())
                .ok_or_else(|| Error::BuildMissing {
                    reason: format!(
                        "no {} {} build installed; run brr build install {} --channel {}",
                        channel.source, channel.name, channel.source, channel.name
                    ),
                })
        }
    }
}

pub fn mark_builtin(settings: &Settings, mut profile: Profile) -> Profile {
    profile.builtin = profile.name == settings.oneshot.profile;

    profile
}

pub fn with_capabilities(builds: &[Build], mut profile: Profile) -> Profile {
    profile.capabilities = pick_build(builds, &profile.settings.build)
        .map(|b| b.capabilities)
        .unwrap_or_default();

    profile
}

// None when the Profile has no login check.
pub fn with_login_status(
    statuses: &HashMap<ProfileId, LoginStatus>,
    mut profile: Profile,
) -> Profile {
    profile.login_status = statuses.get(&profile.id).copied();

    profile
}

// An id-shaped name would be taken for an id by the id-or-name lookups; "." and ".." read as paths
// and a leading "-" as a CLI flag.
pub fn validate_name(name: &str, field: &str) -> Result<(), Error> {
    let valid = (1..=MAX_NAME_LEN).contains(&name.len())
        && name.parse::<ProfileId>().is_err()
        && !matches!(name, "." | "..")
        && !name.starts_with('-')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
    if valid {
        return Ok(());
    }

    Err(Error::InvalidArgument {
        field: field.to_string(),
        message: format!(
            "use 1 to {MAX_NAME_LEN} characters from A-Z, a-z, 0-9, `.`, `_`, `-`, not starting with `-` and not `.` or `..`"
        ),
    })
}

// remove_dir_all on a path read from the database is a data-loss path: only a directory that
// really sits under the profiles dir may go.
pub fn removable_data_dir(paths: &Paths, profile: &Profile) -> Result<Option<PathBuf>, Error> {
    let Some(dir) = &profile.data_dir else {
        return Ok(None);
    };
    let Ok(canonical) = dir.canonicalize() else {
        return Ok(None);
    };
    let root = paths.profiles_dir().canonicalize().map_err(internal)?;

    if canonical != root && canonical.starts_with(&root) {
        return Ok(Some(canonical));
    }

    Err(internal(format!(
        "profile {} data dir {} is outside {}; not deleting it",
        profile.id,
        dir.display(),
        root.display()
    )))
}

#[cfg(test)]
mod tests;

pub async fn all(store: &Store) -> Result<Vec<Profile>, Error> {
    store
        .run(|c| {
            let mut found = Vec::new();
            let mut cursor = None;
            loop {
                let (rows, next) = profile::list(c, None, &Page::new(Some(u32::MAX), cursor))?;
                found.extend(rows);
                if next.is_none() {
                    return Ok(found);
                }
                cursor = next;
            }
        })
        .await
}
