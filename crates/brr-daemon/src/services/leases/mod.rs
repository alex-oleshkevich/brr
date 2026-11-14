use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use brr_core::{
    ClientId, ClientKind, ConflictReason, Error, EventData, InstanceId, Lease, LeaseId,
    ReleaseReason, TabId, TabLease, now_ms,
};
use brr_store::lease;
use tokio::{sync::Mutex, sync::Notify, time};

use crate::{
    app::{App, CallClient},
    internal,
    services::clients,
};

tokio::task_local! {
    // Set by `ops::dispatch` for the length of one call.
    static CALL: Arc<CallWatch>;
}

#[derive(Default)]
struct CallWatch {
    lost: Notify,
    take_over_expected: AtomicBool,
}

// Wakes the call when a Lease it ran under was released by someone else, so the call can end with LeaseLost.
pub async fn run_watched<T>(call: impl Future<Output = Result<T, Error>>) -> Result<T, Error> {
    let watch = Arc::new(CallWatch::default());

    CALL.scope(Arc::clone(&watch), async {
        tokio::select! {
            result = call => result,
            () = watch.lost.notified() => Err(Error::LeaseLost),
        }
    })
    .await
}

// A call that waits for a person (LC6) expects them to take the Tab over; that is not a loss.
pub fn expect_take_over() {
    let _ = CALL.try_with(|call| call.take_over_expected.store(true, Ordering::Relaxed));
}

#[derive(Default)]
struct Active {
    // In acquisition order.
    leases: Vec<Lease>,
    // Who a taken-over Lease came from, by Lease id.
    transferred: HashMap<LeaseId, ClientId>,
    // Calls running under a Tab Lease, by Lease id.
    watchers: HashMap<LeaseId, Vec<Weak<CallWatch>>>,
}

impl Active {
    fn origin_of(&self, lease_id: &LeaseId) -> Option<ClientId> {
        self.transferred.get(lease_id).cloned()
    }

    fn watch(&mut self, lease_id: LeaseId) {
        let Ok(call) = CALL.try_with(Arc::downgrade) else {
            return;
        };
        let watchers = self.watchers.entry(lease_id).or_default();

        watchers.retain(|watcher| watcher.strong_count() > 0);
        watchers.push(call);
    }
}

#[derive(Default)]
pub struct Leases {
    // Every change is written to the table before the lock opens.
    active: Mutex<Active>,
    released: Notify,
}

impl Leases {
    pub async fn acquire_tab(
        &self,
        app: &Arc<App>,
        client: &CallClient,
        instance_id: &InstanceId,
        tab_id: &TabId,
    ) -> Result<Lease, Error> {
        let mut active = self.active.lock().await;

        acquire_locked(app, &mut active, client, instance_id, tab_id).await
    }

    pub async fn acquire_exclusive(
        &self,
        app: &Arc<App>,
        client: &CallClient,
        instance_id: &InstanceId,
    ) -> Result<Lease, Error> {
        let wait = Duration::from_secs(app.settings().lease.exclusive_wait_s);
        let deadline = time::Instant::now() + wait;

        loop {
            // Registered before the check, so a release between the check and the wait is not lost.
            let released = self.released.notified();
            tokio::pin!(released);
            released.as_mut().enable();

            let holder = {
                let mut active = self.active.lock().await;
                let mine = active.leases.iter().find(|l| {
                    l.exclusive && l.instance_id == *instance_id && l.client_id == client.client_id
                });
                if let Some(lease) = mine {
                    return Ok(lease.clone());
                }

                let other = active
                    .leases
                    .iter()
                    .find(|l| l.instance_id == *instance_id && l.client_id != client.client_id);
                match other {
                    Some(other) => other.client_id.clone(),
                    None => {
                        ensure_active(app, client).await?;
                        let lease = new_lease(client, instance_id, None, true);

                        return insert_locked(app, &mut active, lease).await;
                    }
                }
            };

            if time::timeout_at(deadline, released).await.is_err() {
                return Err(Error::LeaseHeld { client_id: holder });
            }
        }
    }

    // A person overrides an agent (LC6): another Client's Tab Lease, and an exclusive Instance
    // Lease, do not stand in the way.
    pub async fn take_over(
        &self,
        app: &Arc<App>,
        client: &CallClient,
        instance_id: &InstanceId,
        tab_id: &TabId,
    ) -> Result<Lease, Error> {
        let mut active = self.active.lock().await;
        ensure_active(app, client).await?;

        let held = active
            .leases
            .iter()
            .find(|l| l.tab_id.as_ref() == Some(tab_id))
            .cloned();
        let Some(held) = held else {
            let lease = new_lease(client, instance_id, Some(tab_id), false);

            return insert_locked(app, &mut active, lease).await;
        };
        if held.client_id == client.client_id {
            return Ok(held);
        }

        let from = held.client_id;

        self.release_and_insert(app, &mut active, held.id, client, from.clone(), Some(from))
            .await
    }

    // The holder must have taken the Tab over; the Lease goes back to the Client it came from.
    pub async fn hand_back(
        &self,
        app: &Arc<App>,
        client: &CallClient,
        tab_id: &TabId,
    ) -> Result<Lease, Error> {
        let mut active = self.active.lock().await;
        let held = active
            .leases
            .iter()
            .find(|l| l.tab_id.as_ref() == Some(tab_id) && l.client_id == client.client_id)
            .and_then(|l| Some((l.id, active.origin_of(&l.id)?)));
        let Some((lease_id, origin)) = held else {
            return Err(Error::InvalidArgument {
                field: "tab_id".to_string(),
                message: "this Tab was not taken over by you".to_string(),
            });
        };

        self.return_locked(app, &mut active, lease_id, client.client_id.clone(), origin)
            .await
    }

    // The person is done in the UI: the Lease goes back when its original Client is still
    // around, and stays with the person otherwise.
    pub async fn hand_back_after_done(&self, app: &Arc<App>, tab_id: &TabId) -> Result<(), Error> {
        let mut active = self.active.lock().await;
        let held = active
            .leases
            .iter()
            .find(|l| l.tab_id.as_ref() == Some(tab_id))
            .and_then(|l| Some((l.id, l.client_id.clone(), active.origin_of(&l.id)?)));
        let Some((lease_id, holder_id, origin)) = held else {
            return Ok(());
        };

        match self
            .return_locked(app, &mut active, lease_id, holder_id, origin)
            .await
        {
            Err(Error::Conflict {
                reason: ConflictReason::ClientGone,
                ..
            }) => Ok(()),
            result => result.map(|_| ()),
        }
    }

    pub async fn guard(
        &self,
        app: &Arc<App>,
        client: &CallClient,
        instance_id: &InstanceId,
        tab_id: Option<&TabId>,
        read_only: bool,
    ) -> Result<(), Error> {
        if read_only {
            return Ok(());
        }

        // LC6: a person may navigate a Tab that waits for them without taking it over.
        if client.kind == ClientKind::Ui
            && tab_id.is_some_and(|tab_id| app.instances.tab_awaits_human(instance_id, tab_id))
        {
            app.instances.activity(instance_id, tab_id);

            return Ok(());
        }

        let mut active = self.active.lock().await;

        match tab_id {
            Some(tab_id) => {
                let lease = acquire_locked(app, &mut active, client, instance_id, tab_id).await?;

                active.watch(lease.id);
            }
            None => check_exclusive(&active.leases, client, instance_id)?,
        }

        // L3: an operation under a Lease is activity; read-only tools returned above.
        app.instances.activity(instance_id, tab_id);

        Ok(())
    }

    // Stopping ends every Client's Tabs, so another Client's Lease on the Instance stands in the
    // way. A person in the UI may stop any Instance (LC6).
    pub async fn guard_stop(
        &self,
        client: &CallClient,
        instance_id: &InstanceId,
    ) -> Result<(), Error> {
        if client.kind == ClientKind::Ui {
            return Ok(());
        }

        let active = self.active.lock().await;
        let other = active
            .leases
            .iter()
            .find(|l| l.instance_id == *instance_id && l.client_id != client.client_id);

        match other {
            Some(other) => Err(Error::LeaseHeld {
                client_id: other.client_id.clone(),
            }),
            None => Ok(()),
        }
    }

    pub async fn release(
        &self,
        app: &Arc<App>,
        client: &CallClient,
        lease_id: &LeaseId,
    ) -> Result<Lease, Error> {
        let mut active = self.active.lock().await;
        let owner = active
            .leases
            .iter()
            .find(|l| l.id == *lease_id)
            .map(|l| l.client_id.clone())
            .ok_or_else(|| Error::NotFound {
                what: format!("lease {lease_id}"),
            })?;

        if owner != client.client_id {
            return Err(Error::LeaseHeld { client_id: owner });
        }

        let released = self
            .release_locked(app, &mut active, ReleaseReason::Released, |l| {
                l.id == *lease_id
            })
            .await?;

        released.into_iter().next().ok_or_else(|| Error::NotFound {
            what: format!("lease {lease_id}"),
        })
    }

    pub async fn release_client(
        &self,
        app: &Arc<App>,
        client_id: &ClientId,
        reason: ReleaseReason,
    ) -> Result<(), Error> {
        let mut active = self.active.lock().await;

        self.release_locked(app, &mut active, reason, |l| l.client_id == *client_id)
            .await?;

        Ok(())
    }

    pub async fn release_instance(
        &self,
        app: &Arc<App>,
        instance_id: &InstanceId,
        reason: ReleaseReason,
    ) -> Result<(), Error> {
        let mut active = self.active.lock().await;

        self.release_locked(app, &mut active, reason, |l| l.instance_id == *instance_id)
            .await?;

        Ok(())
    }

    pub async fn release_tab(
        &self,
        app: &Arc<App>,
        tab_id: &TabId,
        reason: ReleaseReason,
    ) -> Result<(), Error> {
        let mut active = self.active.lock().await;

        self.release_locked(app, &mut active, reason, |l| {
            l.tab_id.as_ref() == Some(tab_id)
        })
        .await?;

        Ok(())
    }

    pub async fn list(
        &self,
        instance_id: Option<&InstanceId>,
        client_id: Option<&ClientId>,
    ) -> Vec<Lease> {
        let active = self.active.lock().await;

        active
            .leases
            .iter()
            .filter(|l| instance_id.is_none_or(|id| l.instance_id == *id))
            .filter(|l| client_id.is_none_or(|id| l.client_id == *id))
            .cloned()
            .collect()
    }

    pub async fn tab_lease(&self, tab_id: &TabId) -> Option<TabLease> {
        let active = self.active.lock().await;
        let lease = active
            .leases
            .iter()
            .find(|l| l.tab_id.as_ref() == Some(tab_id))?;

        Some(TabLease {
            lease_id: lease.id,
            client_id: lease.client_id.clone(),
            client_kind: lease.client_kind,
            client_label: lease.client_label.clone(),
            transferred_from_client_id: active.origin_of(&lease.id),
        })
    }

    pub async fn count(&self, instance_id: &InstanceId) -> u32 {
        let active = self.active.lock().await;
        let count = active
            .leases
            .iter()
            .filter(|l| l.instance_id == *instance_id)
            .count();

        u32::try_from(count).unwrap_or(u32::MAX)
    }

    async fn return_locked(
        &self,
        app: &Arc<App>,
        active: &mut Active,
        lease_id: LeaseId,
        holder_id: ClientId,
        origin: ClientId,
    ) -> Result<Lease, Error> {
        // An implicit Client is never registered, so it counts as gone too.
        let Ok(origin) = app.clients.resolve(app, &origin).await else {
            return Err(Error::Conflict {
                reason: ConflictReason::ClientGone,
                detail: None,
            });
        };

        self.release_and_insert(app, active, lease_id, &origin, holder_id, None)
            .await
    }

    async fn release_and_insert(
        &self,
        app: &Arc<App>,
        active: &mut Active,
        lease_id: LeaseId,
        to: &CallClient,
        from_client_id: ClientId,
        transferred_from: Option<ClientId>,
    ) -> Result<Lease, Error> {
        let released = self
            .release_locked(app, active, ReleaseReason::Transferred, |l| {
                l.id == lease_id
            })
            .await?;
        let old = released
            .into_iter()
            .next()
            .ok_or_else(|| internal("lease vanished under the lock"))?;

        let lease = new_lease(to, &old.instance_id, old.tab_id.as_ref(), false);
        let lease = insert_locked(app, active, lease).await?;
        if let Some(from) = transferred_from {
            active.transferred.insert(lease.id, from);
        }

        app.events
            .publish(EventData::LeaseTransferred {
                lease_id: lease.id,
                instance_id: lease.instance_id,
                from_client_id,
                to_client_id: lease.client_id.clone(),
            })
            .await?;

        Ok(lease)
    }

    async fn release_locked(
        &self,
        app: &Arc<App>,
        active: &mut Active,
        reason: ReleaseReason,
        matches: impl Fn(&Lease) -> bool,
    ) -> Result<Vec<Lease>, Error> {
        let mut released = Vec::new();

        while let Some(index) = active.leases.iter().position(&matches) {
            // Memory first: a caller dropped during the write still leaves the Lease gone, its
            // waiters woken and its calls told, and the write below finishes in its own task.
            let mut lease = active.leases.remove(index);
            let now = now_ms();
            let lease_id = lease.id;

            self.released.notify_waiters();
            active.transferred.remove(&lease.id);
            // A closed Tab or a stopped Instance ends its calls its own way; here someone took the Lease away.
            for call in active
                .watchers
                .remove(&lease.id)
                .into_iter()
                .flatten()
                .filter_map(|call| call.upgrade())
            {
                let taken = match reason {
                    ReleaseReason::Released | ReleaseReason::ClientEnded => true,
                    ReleaseReason::Transferred => !call.take_over_expected.load(Ordering::Relaxed),
                    _ => false,
                };
                if taken {
                    call.lost.notify_one();
                }
            }

            let event = EventData::LeaseReleased {
                lease_id,
                client_id: lease.client_id.clone(),
                instance_id: lease.instance_id,
                tab_id: lease.tab_id,
                exclusive: lease.exclusive,
                reason,
            };
            let app = Arc::clone(app);
            let finished = tokio::spawn(async move {
                match app
                    .store
                    .run(move |c| lease::release(c, &lease_id, reason, now))
                    .await
                {
                    Ok(()) => Ok(app.events.publish(event).await.map(drop)),
                    Err(error) => Err(error),
                }
            })
            .await
            .map_err(internal)?;
            match finished {
                Ok(announced) => announced?,
                Err(error) => {
                    active.leases.insert(index, lease);

                    return Err(error);
                }
            }

            lease.released_at = Some(now);
            lease.release_reason = Some(reason);
            released.push(lease);
        }

        Ok(released)
    }
}

fn new_lease(
    client: &CallClient,
    instance_id: &InstanceId,
    tab_id: Option<&TabId>,
    exclusive: bool,
) -> Lease {
    Lease {
        id: LeaseId::new(),
        client_id: client.client_id.clone(),
        client_kind: client.kind,
        client_label: client.label.clone(),
        instance_id: *instance_id,
        tab_id: tab_id.copied(),
        exclusive,
        acquired_at: now_ms(),
        released_at: None,
        release_reason: None,
    }
}

fn check_exclusive(
    leases: &[Lease],
    client: &CallClient,
    instance_id: &InstanceId,
) -> Result<(), Error> {
    let other = leases
        .iter()
        .find(|l| l.exclusive && l.instance_id == *instance_id && l.client_id != client.client_id);

    match other {
        Some(other) => Err(Error::LeaseHeld {
            client_id: other.client_id.clone(),
        }),
        None => Ok(()),
    }
}

async fn acquire_locked(
    app: &Arc<App>,
    active: &mut Active,
    client: &CallClient,
    instance_id: &InstanceId,
    tab_id: &TabId,
) -> Result<Lease, Error> {
    ensure_active(app, client).await?;

    let held = active
        .leases
        .iter()
        .find(|l| l.tab_id.as_ref() == Some(tab_id))
        .cloned();
    // Before the exclusive check: a person who took the Tab over keeps using it while another
    // Client holds the Instance exclusively.
    if let Some(held) = &held
        && held.client_id == client.client_id
    {
        return Ok(held.clone());
    }
    check_exclusive(&active.leases, client, instance_id)?;
    if let Some(held) = held {
        return Err(Error::LeaseHeld {
            client_id: held.client_id,
        });
    }

    let lease = new_lease(client, instance_id, Some(tab_id), false);

    insert_locked(app, active, lease).await
}

// Checked under the Leases lock (Leases, then Clients): a Client that ended cannot gain Leases
// after its end released them.
async fn ensure_active(app: &Arc<App>, client: &CallClient) -> Result<(), Error> {
    if client.implicit || app.clients.is_active(&client.client_id).await {
        return Ok(());
    }

    Err(clients::not_active(&client.client_id))
}

// The unique index on active Tab Leases is a backstop: the checks above decide under the lock,
// so a violation here is a bug and surfaces as Internal.
async fn insert_locked(app: &Arc<App>, active: &mut Active, lease: Lease) -> Result<Lease, Error> {
    // Memory first: a cancelled call leaves the write that already reached the store in step.
    active.leases.push(lease.clone());
    let row = lease.clone();
    if let Err(error) = app.store.run(move |c| lease::insert(c, &row)).await {
        active.leases.pop();

        return Err(error);
    }

    app.events
        .publish(EventData::LeaseAcquired {
            lease_id: lease.id,
            client_id: lease.client_id.clone(),
            instance_id: lease.instance_id,
            tab_id: lease.tab_id,
            exclusive: lease.exclusive,
        })
        .await?;

    Ok(lease)
}
