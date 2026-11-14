use std::time::Duration;

use brr_core::{CredentialId, Error, TabId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    output,
    tool::{tab_call, tab_op, timeout},
};
use crate::{app::Ctx, services::login::login};

const DEFAULT_LOGIN_TIMEOUT_MS: u64 = 60_000;

tab_op!(LOGIN {
    tool: "login",
    cli: "login",
    summary: "Log in to the Tab's site with a vault credential linked to its Profile; the daemon types the secrets, the agent never sees them",
    positional: &["credential_id"],
    read_only: false,
    capability: None,
    input: LoginInput,
    output: LoginOut,
    run: run,
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LoginInput {
    tab_id: Option<TabId>,
    credential_id: CredentialId,
    // Milliseconds for the whole login, default 60000.
    timeout: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
struct LoginOut {
    ok: bool,
    steps: u8,
    url: String,
}

async fn run(ctx: Ctx, input: LoginInput) -> Result<Value, Error> {
    let limit: Duration = timeout(Some(input.timeout.unwrap_or(DEFAULT_LOGIN_TIMEOUT_MS)))?;
    let call = tab_call(&ctx, input.tab_id, &LOGIN).await?;

    let done = login(&ctx.app, &call, &input.credential_id, limit).await?;

    output(&LoginOut {
        ok: true,
        steps: done.steps,
        url: done.url,
    })
}
