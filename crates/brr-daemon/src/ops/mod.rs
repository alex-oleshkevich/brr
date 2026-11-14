use std::{future::Future, panic::AssertUnwindSafe, time::Instant};

use brr_core::{ClientId, Error, InstanceId, OperationSpec};
use brr_store::instance;
use futures::{FutureExt, future::BoxFuture};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    app::Ctx,
    services::{events::as_actor, leases::run_watched},
};

mod artifacts;
mod backups;
mod batch;
mod bookmarks;
mod builds;
mod captcha;
mod clients;
mod companion;
mod cookies;
mod credentials;
mod events;
mod extensions;
mod instances;
mod leases;
mod limits;
mod live;
mod login;
mod login_checks;
mod oneshot;
mod profile_archive;
mod profiles;
mod proxies;
mod retention;
mod settings;
mod storage;
mod system;
mod tab_capture;
mod tab_human;
mod tab_input;
mod tab_state;
mod tabs;
mod templates;
mod tool;

pub(crate) use tool::TabCall;

pub(super) const MAX_PRIORITY: u8 = 9;

const PAGE_SCHEMES: [&str; 6] = ["http", "https", "file", "about", "data", "chrome"];

pub type Handler = fn(Ctx, Value) -> BoxFuture<'static, Result<Value, Error>>;

pub struct OpRegistration {
    pub spec: &'static OperationSpec,
    pub handler: Handler,
}

inventory::collect!(OpRegistration);

pub fn all() -> Vec<&'static OpRegistration> {
    let mut ops: Vec<&'static OpRegistration> = inventory::iter::<OpRegistration>().collect();
    ops.sort_by_key(|op| op.spec.name);

    ops
}

pub fn find(name: &str) -> Option<&'static OpRegistration> {
    inventory::iter::<OpRegistration>().find(|op| op.spec.name == name)
}

pub fn find_mcp(tool: &str) -> Option<&'static OpRegistration> {
    inventory::iter::<OpRegistration>().find(|op| op.spec.mcp_tool == Some(tool))
}

pub async fn dispatch(ctx: Ctx, op: &'static OpRegistration, input: Value) -> Result<Value, Error> {
    let client_id = ctx.client.client_id.clone();
    // The Client's idle timer must not run out under its own long call.
    let _calling = ctx.app.clients.in_call(&ctx.app, &client_id).await;

    logged(
        op.spec.name,
        &client_id,
        run_watched((op.handler)(ctx, input)),
    )
    .await
}

// A panic in a dependency answers this one call with Internal instead of dropping the connection;
// every call, including the routes that bypass `dispatch`, leaves one `op` line.
pub async fn logged<T>(
    name: &str,
    client_id: &ClientId,
    call: impl Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    let started = Instant::now();

    let result = AssertUnwindSafe(as_actor(client_id.clone(), call))
        .catch_unwind()
        .await
        .unwrap_or_else(|_| {
            Err(crate::internal(
                "the operation panicked; see the daemon log",
            ))
        });

    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match &result {
        Err(Error::Internal { message }) => {
            tracing::error!(op = name, %client_id, elapsed_ms, error = message.as_str(), "op");
        }
        _ => {
            let code = result.as_ref().err().map(Error::code);
            tracing::info!(op = name, %client_id, ok = result.is_ok(), code, elapsed_ms, "op");
        }
    }

    result
}

pub fn parse<T: DeserializeOwned>(input: Value) -> Result<T, Error> {
    serde_path_to_error::deserialize(input).map_err(|e| {
        let path = e.path().to_string();
        let message = e.into_inner().to_string();
        // serde reports a missing field at its parent's path.
        let missing = message
            .strip_prefix("missing field `")
            .and_then(|rest| rest.split('`').next());
        let field = match (path.as_str(), missing) {
            (".", Some(name)) => name.to_string(),
            (".", None) => "input".to_string(),
            (parent, Some(name)) => format!("{parent}.{name}"),
            (_, None) => path,
        };

        Error::InvalidArgument { field, message }
    })
}

pub fn output<T: Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(crate::internal)
}

// For `Option<Option<T>>` fields: absent keeps, `null` clears, a value sets.
pub fn some<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

// A page address with a scheme a browser can open; "notaurl" or "example.com" has none.
pub fn page_url(url: &str) -> Result<(), Error> {
    match reqwest::Url::parse(url) {
        Ok(parsed) if PAGE_SCHEMES.contains(&parsed.scheme()) => Ok(()),
        _ => Err(invalid(
            "url",
            &format!(
                "{url:?} must be an absolute URL starting with {}",
                PAGE_SCHEMES.join(":, ") + ":"
            ),
        )),
    }
}

// An Instance that has ended takes no more Tabs or Leases; its state says why.
pub async fn ensure_live(ctx: &Ctx, instance_id: InstanceId) -> Result<(), Error> {
    let state = ctx
        .app
        .store
        .run(move |c| instance::get(c, &instance_id))
        .await?
        .state;

    if state.is_final() {
        return Err(Error::InvalidTransition {
            from: format!("{state:?}").to_lowercase(),
            to: "running".to_owned(),
        });
    }

    Ok(())
}

pub fn invalid(field: &str, message: &str) -> Error {
    Error::InvalidArgument {
        field: field.to_string(),
        message: message.to_string(),
    }
}
