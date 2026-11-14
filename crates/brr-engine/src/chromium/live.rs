use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use brr_cdp::{Cdp, CdpEvent, SessionId};
use brr_core::TargetId;
use brr_core::settings::TabSettings;
use tokio::sync::{Notify, mpsc};

use super::session::{attach_page, register_tab};
use super::targets::{self, Mode};
use super::{EmulateOptions, Inner};
use crate::live::{LiveTarget, Shared};
use crate::{Chromium, DriverEvent};

impl Chromium {
    // The engine of a browser brr did not start: pages route through the same state as a launched
    // one, but no download directory or Profile view is applied to it.
    pub(crate) fn connect(
        cdp: Cdp,
        cdp_events: mpsc::Receiver<CdpEvent>,
        events: mpsc::Sender<DriverEvent>,
        stop: Arc<Notify>,
        initial: Vec<LiveTarget>,
        shared: Arc<Shared>,
    ) -> Chromium {
        // A Live attach carries no Tab settings of its own, so the defaults apply.
        let tab = TabSettings::default();
        let inner = Arc::new(Inner {
            cdp,
            download_dir: PathBuf::new(),
            console_buffer: tab.console_buffer,
            network_buffer: tab.network_buffer,
            profile_view: EmulateOptions::default(),
            content_defaults: Mutex::default(),
            tabs: Mutex::default(),
            routes: Mutex::default(),
        });
        tokio::spawn(targets::run(
            inner.clone(),
            cdp_events,
            events,
            Mode::Live {
                stop,
                initial,
                shared,
            },
        ));

        Chromium { inner }
    }

    pub(crate) fn cdp(&self) -> &Cdp {
        &self.inner.cdp
    }

    pub(crate) fn session_ids(&self) -> Vec<SessionId> {
        self.inner
            .tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .map(|tab| tab.session_id.clone())
            .collect()
    }

    // Registers the page before it initializes, so events during init already route to its
    // state. The Tab events were sent when the page was discovered, so attach_page's go nowhere.
    pub(crate) async fn init_attached(&self, target_id: TargetId, session_id: SessionId) {
        let state = register_tab(&self.inner, &session_id, &target_id);
        let (nowhere, _) = mpsc::channel(1);

        attach_page(
            self.inner.clone(),
            nowhere,
            Arc::new(Mutex::new(None)),
            false,
            session_id,
            target_id,
            state,
        )
        .await;
    }
}
