use std::{fs, path::PathBuf, sync::Arc};

use brr_core::{
    ConflictReason, Error, EventData, Instance, InstanceState, Paths, Profile, ProfileId,
    ProfileKind, now_ms,
};
use brr_store::profile;

use tracing::error;

use crate::{
    app::App,
    internal,
    services::profiles::{DEFAULT_BACKUP_KEEP, MAX_NAME_LEN},
};

const PROMOTED: &str = "promoted";
const ID_SUFFIX_LEN: usize = 6;
const UNPROMOTED_DIR: &str = "unpromoted";

// P1a: promotion is refused from stopping onwards.
pub(crate) fn check_promotable(instance: &Instance) -> Result<(), Error> {
    if !instance.ephemeral {
        return Err(Error::InvalidArgument {
            field: "instance_id".to_owned(),
            message: "only ephemeral instances can be promoted".to_owned(),
        });
    }

    match instance.state {
        InstanceState::Starting
        | InstanceState::Running
        | InstanceState::Idle
        | InstanceState::Reaping => Ok(()),
        state => Err(Error::InvalidTransition {
            from: format!("{state:?}").to_lowercase(),
            to: PROMOTED.to_owned(),
        }),
    }
}

// The data folder is moved, never copied: copying a live Chromium profile is what P1a forbids.
// Any error leaves the data where it is (or, once moved, in its Profile folder).
pub(crate) async fn finish_promote(
    app: &Arc<App>,
    instance: &Instance,
) -> Result<Option<Profile>, Error> {
    let Some(name) = instance.promote_to_name.clone() else {
        return Ok(None);
    };
    let (Some(source_id), Some(data_dir)) = (instance.profile_id, instance.data_dir.clone()) else {
        return Err(internal(format!(
            "instance {} has no profile or data dir to promote",
            instance.id
        )));
    };
    if !data_dir.starts_with(app.paths.ephemeral_dir()) {
        return Err(internal(format!(
            "data dir {} is outside the ephemeral dir; not promoting it",
            data_dir.display()
        )));
    }

    let profiles_dir = app.paths.profiles_dir();
    let instance_id = instance.id.to_string();
    let promoted = app
        .store
        .run(move |c| {
            let source = profile::get(c, &source_id)?;
            let id = ProfileId::new();
            let dest = profiles_dir.join(id.to_string());

            // Move first: a crash between the move and the insert leaves the data in a folder
            // nothing sweeps, never a Profile without data.
            fs::create_dir_all(&profiles_dir)
                .and_then(|()| fs::rename(&data_dir, &dest))
                .map_err(internal)?;

            let now = now_ms();
            let mut created = Profile {
                id,
                name,
                kind: ProfileKind::Persistent,
                template_id: source.template_id,
                template_name: source.template_name,
                settings: source.settings,
                pinned: false,
                notes: format!("promoted from instance {instance_id}"),
                tags: source.tags,
                data_dir: Some(dest.clone()),
                disk_quota_bytes: None,
                backup_schedule: None,
                backup_keep: DEFAULT_BACKUP_KEEP,
                created_at: now,
                updated_at: now,
                capabilities: Vec::new(),
                login_status: None,
                builtin: false,
            };

            // The name may have been taken since the request; one retry with the instance's
            // own id keeps the data findable.
            let mut inserted = profile::insert(c, &created);
            if let Err(Error::Conflict {
                reason: ConflictReason::NameTaken,
                ..
            }) = inserted
            {
                let tail = &instance_id[instance_id.len() - ID_SUFFIX_LEN..];
                let base: String = created
                    .name
                    .chars()
                    .take(MAX_NAME_LEN - ID_SUFFIX_LEN - 1)
                    .collect();
                created.name = format!("{base}-{tail}");
                inserted = profile::insert(c, &created);
            }
            if let Err(error) = inserted {
                // Back to where it was; if even that fails the data stays in `dest`.
                if let Err(undo) = fs::rename(&dest, &data_dir) {
                    error!(%undo, dest = %dest.display(), "promoted data not moved back");
                }

                return Err(error);
            }

            Ok(created)
        })
        .await?;

    app.events
        .publish(EventData::ProfileCreated {
            profile_id: promoted.id,
            name: promoted.name.clone(),
        })
        .await?;

    Ok(Some(promoted))
}

// A promotion that cannot finish must not leave the data where a sweep deletes it. Returns the
// dir when it could not even be set aside and so stays in place.
pub(crate) fn set_aside(paths: &Paths, instance: &Instance) -> Option<PathBuf> {
    let dir = instance.data_dir.clone().filter(|dir| dir.exists())?;
    let dest = paths
        .data_dir
        .join(UNPROMOTED_DIR)
        .join(instance.id.to_string());

    let moved = fs::create_dir_all(paths.data_dir.join(UNPROMOTED_DIR))
        .and_then(|()| fs::rename(&dir, &dest));
    match moved {
        Ok(()) => {
            error!(instance_id = %instance.id, path = %dest.display(), "promotion failed; the data was set aside");

            None
        }
        Err(error) => {
            error!(instance_id = %instance.id, %error, path = %dir.display(), "promotion failed; the data stays in place");

            Some(dir)
        }
    }
}
