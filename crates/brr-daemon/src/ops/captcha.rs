use std::collections::BTreeMap;

use brr_core::{
    CaptchaPolicy, CaptchaSolve, CaptchaType, Empty, Error, Http, Method, OperationSpec, Page,
    Secret, SecretStatus, TabId, Toolset, UsdMicros, schema,
};
use brr_store::captcha_solve;
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    OpRegistration, invalid, output, parse, some,
    tool::{tab_call, tab_op},
};
use crate::{
    app::{App, Ctx},
    internal,
    services::captcha::{
        API_KEY_SECRET, PROVIDER_NAME, SolvedBy, provider, solve_now, spent_today,
    },
};

const SETTINGS_PATH: &str = "/v1/captcha/settings";

static SETTINGS_GET: OperationSpec = OperationSpec {
    name: "captcha.settings_get",
    group: "captcha",
    toolset: Toolset::Captcha,
    summary: "Show the captcha policy, budgets, today's spend and whether a provider key is set",
    http: Http {
        method: Method::Get,
        path: SETTINGS_PATH,
    },
    cli: &["captcha", "settings", "get"],
    cli_positional: &[],
    mcp_tool: Some("captcha_settings_get"),
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<SettingsView>,
};

static SETTINGS_SET: OperationSpec = OperationSpec {
    name: "captcha.settings_set",
    group: "captcha",
    toolset: Toolset::Captcha,
    summary: "Change captcha settings; only the given fields change, `api_key: null` removes the key",
    http: Http {
        method: Method::Put,
        path: SETTINGS_PATH,
    },
    cli: &["captcha", "settings", "set"],
    cli_positional: &[],
    mcp_tool: Some("captcha_settings_set"),
    read_only: false,
    capability: None,
    input_schema: schema::<SetInput>,
    output_schema: schema::<SettingsView>,
};

static SOLVES: OperationSpec = OperationSpec {
    name: "captcha.solves",
    group: "captcha",
    toolset: Toolset::Captcha,
    summary: "List the captcha solve log, newest first",
    http: Http {
        method: Method::Get,
        path: "/v1/captcha/solves",
    },
    cli: &["captcha", "solves"],
    cli_positional: &[],
    mcp_tool: Some("captcha_solves"),
    read_only: true,
    capability: None,
    input_schema: schema::<SolvesInput>,
    output_schema: schema::<SolvesOutput>,
};

static BALANCE: OperationSpec = OperationSpec {
    name: "captcha.balance",
    group: "captcha",
    toolset: Toolset::Captcha,
    summary: "Ask the provider for the account balance",
    http: Http {
        method: Method::Get,
        path: "/v1/captcha/balance",
    },
    cli: &["captcha", "balance"],
    cli_positional: &[],
    mcp_tool: Some("captcha_balance"),
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<BalanceOutput>,
};

tab_op!(SOLVE_CAPTCHA {
    tool: "solve_captcha",
    cli: "solve-captcha",
    summary: "Detect a captcha on the Tab and solve it with the provider; when that fails or the budget is spent, wait for a person",
    positional: &[],
    read_only: false,
    capability: None,
    input: SolveInput,
    output: SolveOutcome,
    run: solve_captcha,
});

inventory::submit!(OpRegistration {
    spec: &SETTINGS_GET,
    handler: settings_get
});
inventory::submit!(OpRegistration {
    spec: &SETTINGS_SET,
    handler: settings_set
});
inventory::submit!(OpRegistration {
    spec: &SOLVES,
    handler: solves
});
inventory::submit!(OpRegistration {
    spec: &BALANCE,
    handler: balance
});

#[derive(Serialize, JsonSchema)]
struct SettingsView {
    default_policy: CaptchaPolicy,
    provider: &'static str,
    api_key: Option<SecretStatus>,
    daily_usd_micros: u64,
    daily_solves: u32,
    human_wait_s: u64,
    spent_today_usd_micros: u64,
    solves_today: u32,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetInput {
    default_policy: Option<CaptchaPolicy>,
    #[serde(default, deserialize_with = "some")]
    api_key: Option<Option<Secret>>,
    daily_usd_micros: Option<u64>,
    daily_solves: Option<u32>,
    human_wait_s: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SolvesInput {
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct SolvesOutput {
    solves: Vec<CaptchaSolve>,
    next_cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct BalanceOutput {
    balance_usd_micros: u64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SolveInput {
    tab_id: Option<TabId>,
    human_fallback: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
struct SolveOutcome {
    detected: Option<DetectedView>,
    solved_by: Option<SolvedBy>,
}

#[derive(Serialize, JsonSchema)]
struct DetectedView {
    #[serde(rename = "type")]
    captcha_type: CaptchaType,
    sitekey: Option<String>,
}

async fn view(app: &App) -> Result<Value, Error> {
    let settings = app.settings();
    let limits = &settings.captcha;
    let key = app.secrets.status(&app.store, API_KEY_SECRET).await?;
    let (spent, solves) = spent_today(app).await?;

    output(&SettingsView {
        default_policy: limits.default_policy,
        provider: PROVIDER_NAME,
        api_key: key.set.then_some(key),
        daily_usd_micros: limits.daily_usd.0,
        daily_solves: limits.daily_solves,
        human_wait_s: limits.human_wait_s,
        spent_today_usd_micros: spent,
        solves_today: solves,
    })
}

fn settings_get(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        view(&ctx.app).await
    }
    .boxed()
}

fn settings_set(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: SetInput = parse(input)?;
        if let Some(Some(key)) = &input.api_key
            && key.expose().is_empty()
        {
            return Err(invalid("api_key", "must not be empty"));
        }

        let mut changes = BTreeMap::new();
        if let Some(policy) = input.default_policy {
            changes.insert(
                "captcha.default_policy".to_string(),
                serde_json::to_value(policy).map_err(internal)?,
            );
        }
        if let Some(micros) = input.daily_usd_micros {
            changes.insert(
                "captcha.daily_usd".to_string(),
                serde_json::to_value(UsdMicros(micros)).map_err(internal)?,
            );
        }
        if let Some(solves) = input.daily_solves {
            changes.insert("captcha.daily_solves".to_string(), json!(solves));
        }
        if let Some(wait) = input.human_wait_s {
            changes.insert("captcha.human_wait_s".to_string(), json!(wait));
        }
        if !changes.is_empty() {
            ctx.app.set_settings(changes).await?;
        }

        let app = &ctx.app;
        match &input.api_key {
            Some(Some(key)) => app.secrets.set(&app.store, API_KEY_SECRET, key).await?,
            Some(None) => app.secrets.delete(&app.store, API_KEY_SECRET).await?,
            None => {}
        }

        view(app).await
    }
    .boxed()
}

fn solves(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: SolvesInput = parse(input)?;
        let page = Page::new(input.limit, input.cursor);
        let (solves, next_cursor) = ctx
            .app
            .store
            .run(move |c| captcha_solve::list(c, &page))
            .await?;

        output(&SolvesOutput {
            solves,
            next_cursor,
        })
    }
    .boxed()
}

fn balance(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        let app = &ctx.app;
        let key = app
            .secrets
            .get(&app.store, API_KEY_SECRET)
            .await?
            .ok_or_else(|| invalid("api_key", "set a 2captcha API key first"))?;
        // The provider's own error says "could not be solved", which is wrong for a balance check.
        let balance_usd_micros =
            provider(app, &key)
                .balance()
                .await
                .map_err(|error| match error {
                    Error::CaptchaFailed => Error::DependencyMissing {
                        name: "the 2captcha balance".to_string(),
                        help: "check the API key and that the provider is reachable".to_string(),
                    },
                    other => other,
                })?;

        output(&BalanceOutput { balance_usd_micros })
    }
    .boxed()
}

async fn solve_captcha(ctx: Ctx, input: SolveInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &SOLVE_CAPTCHA).await?;
    let human_fallback = input.human_fallback.unwrap_or(true);

    let solved = solve_now(&ctx.app, &call, human_fallback).await?;

    output(&match solved {
        Some((detected, solved_by)) => SolveOutcome {
            detected: Some(DetectedView {
                captcha_type: detected.captcha_type,
                sitekey: detected.sitekey,
            }),
            solved_by: Some(solved_by),
        },
        None => SolveOutcome {
            detected: None,
            solved_by: None,
        },
    })
}
