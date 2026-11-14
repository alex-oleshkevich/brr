use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

use brr_core::{
    EndReason, Error, EventData, InstanceId, InstanceKind, InstanceTrigger, Paths, TabCloseReason,
    now_ms,
};
use brr_engine::procs::{kill_marked, parent_and_group};
use brr_store::{instance, tab};
use nix::{
    sys::{
        prctl,
        signal::{Signal, killpg},
        wait::{WaitPidFlag, waitpid},
    },
    unistd::{Pid, getpgrp},
};
use tokio::signal::unix::{SignalKind, signal};
use tracing::{error, info, warn};

use crate::{app::App, internal, services::instances};

const SELF_TASK_DIR: &str = "/proc/self/task";
const DAEMON_RESTART: &str = "daemon_restart";

// Process-wide: every App's reaper must see every App's browsers, or it reaps what another
// App's tokio Child still waits for.
static TRACKED: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());

// The Xvfb of each Camoufox Instance on a virtual display; a relaunch replaces it.
static XVFB: Mutex<BTreeMap<InstanceId, u32>> = Mutex::new(BTreeMap::new());

pub struct Supervisor;

impl Supervisor {
    pub fn new() -> Result<Supervisor, Error> {
        prctl::set_child_subreaper(true).map_err(internal)?;

        let mut sigchld = signal(SignalKind::child()).map_err(internal)?;
        tokio::spawn(async move {
            while sigchld.recv().await.is_some() {
                reap_orphans();
            }
        });

        Ok(Supervisor)
    }

    pub fn track(&self, pid: u32) {
        TRACKED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(pid);
    }

    pub fn untrack(&self, pid: u32) {
        TRACKED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&pid);
    }

    pub fn track_xvfb(&self, instance_id: InstanceId, pid: u32) {
        self.track(pid);
        let replaced = XVFB
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(instance_id, pid);
        if let Some(old) = replaced.filter(|old| *old != pid) {
            self.untrack(old);
        }
    }

    pub fn untrack_xvfb(&self, instance_id: &InstanceId) {
        let removed = XVFB
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(instance_id);
        if let Some(pid) = removed {
            self.untrack(pid);
        }
    }

    // Only our own markers: the instance's process group and BRR_INSTANCE_ID values of this DB.
    // The group goes first and takes the whole tree in the common case; the host-wide marker scan
    // catches processes that left it and runs on the blocking pool.
    pub async fn kill_instance(&self, instance_id: &InstanceId, pgid: Option<u32>) {
        if let Some(pgid) = pgid {
            let _ = killpg(nix_pid(pgid), Signal::SIGKILL);
        }

        let ids = HashSet::from([*instance_id]);
        if let Err(error) = tokio::task::spawn_blocking(move || kill_marked(&ids)).await {
            warn!(%instance_id, %error, "marker scan did not finish");
        }
    }

    pub async fn reconcile(app: &Arc<App>) -> Result<(), Error> {
        let ids: HashSet<InstanceId> = app
            .store
            .run(|c| instance::all_ids(c))
            .await?
            .into_iter()
            .collect();
        kill_marked(&ids);

        let stale = app.store.run(|c| instance::non_final(c)).await?;
        let mut kept: Vec<PathBuf> = Vec::new();
        for mut row in stale {
            let trigger = match row.kind {
                InstanceKind::Managed => InstanceTrigger::Reconcile,
                InstanceKind::Live => InstanceTrigger::Detach,
            };
            row.ended_at = Some(now_ms());
            row.end_reason = Some(EndReason::DaemonRestart);
            instances::transition(app, &mut row, trigger, Some(DAEMON_RESTART.to_owned())).await?;

            let instance_id = row.id;
            let now = now_ms();
            let closed = app
                .store
                .run(move |c| {
                    let open = tab::list(c, &instance_id, false)?;
                    tab::close_open(c, &instance_id, now)?;

                    Ok(open)
                })
                .await?;
            for closed_tab in closed {
                app.events
                    .publish(EventData::TabClosed {
                        tab_id: closed_tab.id,
                        instance_id,
                        reason: TabCloseReason::DaemonRestart,
                    })
                    .await?;
            }

            // Promotion moves the data out first; after a failed one the data is kept.
            match instances::promote::finish_promote(app, &row).await {
                Ok(None) => {
                    if row.ephemeral
                        && let Some(dir) = &row.data_dir
                    {
                        remove_ephemeral(&app.paths, dir);
                    }
                }
                Ok(Some(_)) => {}
                Err(error) => {
                    error!(%instance_id, %error, "promotion failed");
                    kept.extend(instances::promote::set_aside(&app.paths, &row));
                }
            }
            info!(%instance_id, "stale instance reconciled");
        }

        kept.extend(retry_unfinished_promotions(app).await);

        remove_ephemeral_leftovers(&app.paths, &kept);

        Ok(())
    }
}

// A daemon killed between a stop and the move leaves a final row whose data still sits in the
// ephemeral dir; the leftover sweep must not take it. Returns the dirs that stay in place.
async fn retry_unfinished_promotions(app: &Arc<App>) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(app.paths.ephemeral_dir()) else {
        return Vec::new();
    };

    let mut kept = Vec::new();
    for entry in entries.flatten() {
        let Some(instance_id) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<InstanceId>().ok())
        else {
            continue;
        };
        let Ok(row) = app.store.run(move |c| instance::get(c, &instance_id)).await else {
            continue;
        };
        if row.promote_to_name.is_none() {
            continue;
        }

        if let Err(error) = instances::promote::finish_promote(app, &row).await {
            error!(%instance_id, %error, "promotion failed");
            kept.extend(instances::promote::set_aside(&app.paths, &row));
        }
    }

    kept
}

// remove_dir_all on a path read from the database is a data-loss path: only a directory that
// really sits under the ephemeral dir may go.
pub(crate) fn remove_ephemeral(paths: &Paths, dir: &Path) {
    let (Ok(canonical), Ok(root)) = (dir.canonicalize(), paths.ephemeral_dir().canonicalize())
    else {
        return;
    };

    if canonical == root || !canonical.starts_with(&root) {
        warn!(dir = %dir.display(), root = %root.display(), "ephemeral data dir is outside the ephemeral dir; not deleting it");

        return;
    }

    if let Err(error) = fs::remove_dir_all(&canonical) {
        warn!(%error, dir = %canonical.display(), "ephemeral data not deleted");
    }
}

fn remove_ephemeral_leftovers(paths: &Paths, kept: &[PathBuf]) {
    let Ok(entries) = fs::read_dir(paths.ephemeral_dir()) else {
        return;
    };

    for entry in entries.flatten() {
        if kept.contains(&entry.path()) {
            continue;
        }
        remove_ephemeral(paths, &entry.path());
    }
}

fn nix_pid(pid: u32) -> Pid {
    Pid::from_raw(pid as i32)
}

// Orphaned browser descendants are reparented to us (subreaper). Never waitpid(-1): that would
// steal the children tokio waits for. Tracked pids and children in our own group (version
// probes) belong to tokio.
fn reap_orphans() {
    let own_group = getpgrp().as_raw() as u32;

    for pid in own_children() {
        let is_tracked = TRACKED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&pid);
        let foreign_group = parent_and_group(pid).is_some_and(|(_, group)| group != own_group);

        if !is_tracked && foreign_group {
            let _ = waitpid(nix_pid(pid), Some(WaitPidFlag::WNOHANG));
        }
    }
}

fn own_children() -> Vec<u32> {
    let Ok(tasks) = fs::read_dir(SELF_TASK_DIR) else {
        return Vec::new();
    };

    tasks
        .flatten()
        .filter_map(|task| fs::read_to_string(task.path().join("children")).ok())
        .flat_map(|children| {
            children
                .split_whitespace()
                .filter_map(|pid| pid.parse().ok())
                .collect::<Vec<u32>>()
        })
        .collect()
}
