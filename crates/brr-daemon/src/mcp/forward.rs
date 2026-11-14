use brr_core::{ClientId, ClientKind, Error};
use futures::future::BoxFuture;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::remote::{DAEMON_NAME, Remote, RemoteHold};

pub type Ensure = Box<dyn Fn() -> BoxFuture<'static, Result<(), Error>> + Send + Sync>;

struct Session {
    client_id: ClientId,
    hold: RemoteHold,
}

// `brr mcp` owns one Client on the daemon and its Hold. A daemon restart ends both, so the next
// call starts the daemon if needed and registers a fresh Client instead of failing for good.
pub struct Forwarder {
    remote: Remote,
    label: String,
    ensure: Ensure,
    session: Mutex<Option<Session>>,
}

impl Forwarder {
    pub fn new(remote: Remote, label: &str, ensure: Ensure) -> Forwarder {
        Forwarder {
            remote,
            label: label.to_string(),
            ensure,
            session: Mutex::new(None),
        }
    }

    pub async fn client_id(&self) -> Result<ClientId, Error> {
        let mut session = self.session.lock().await;
        let open = match session.take() {
            Some(open) if !open.hold.is_closed() => open,
            _ => self.register().await?,
        };
        let client_id = open.client_id.clone();
        *session = Some(open);

        Ok(client_id)
    }

    pub async fn call(&self, op: &str, input: &Value) -> Result<Value, Error> {
        let client_id = self.client_id().await?;

        match self.remote.call(op, input, Some(&client_id)).await {
            // The call failed before it ran, so it is safe to repeat on the new Client.
            Err(error) if lost(&error, &client_id) => {
                self.replace(&client_id).await?;
                let client_id = self.client_id().await?;

                self.remote.call(op, input, Some(&client_id)).await
            }
            result => result,
        }
    }

    async fn replace(&self, stale: &ClientId) -> Result<(), Error> {
        let mut session = self.session.lock().await;
        if session.as_ref().is_none_or(|open| open.client_id == *stale) {
            *session = Some(self.register().await?);
        }

        Ok(())
    }

    async fn register(&self) -> Result<Session, Error> {
        (self.ensure)().await?;
        let client = self
            .remote
            .create_client(ClientKind::Mcp, &self.label)
            .await?;
        let hold = self.remote.hold(&client.id).await?;

        Ok(Session {
            client_id: client.id,
            hold,
        })
    }
}

fn lost(error: &Error, client_id: &ClientId) -> bool {
    match error {
        Error::NotFound { what } => what.contains(client_id.as_str()),
        Error::DependencyMissing { name, .. } => name == DAEMON_NAME,
        _ => false,
    }
}
