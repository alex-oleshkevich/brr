use std::{collections::BTreeMap, fmt::Display, fmt::Write, sync::Arc};

use axum::{
    extract::State,
    http::{HeaderValue, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
};
use brr_core::{BuildId, Error, InstanceId, TabState};
use brr_store::{artifact, build, instance};
use serde::Serialize;

use super::ops::ApiError;
use crate::{app::App, services::host};

const CONTENT_TYPE_VALUE: &str = "text/plain; version=0.0.4; charset=utf-8";
const UNKNOWN: &str = "unknown";

pub async fn serve(State(app): State<Arc<App>>) -> Result<Response, ApiError> {
    let body = render(&app).await?;

    Ok((
        [(CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE_VALUE))],
        body,
    )
        .into_response())
}

// Later milestones add their families here with `family` and `sample`; a family with no
// samples is left out, except the queue depth.
pub async fn render(app: &Arc<App>) -> Result<String, Error> {
    let mut out = String::new();

    let views = app.instances.views();
    let queued = app.admission.queue().await;

    // Only the Build engine comes from the database: owners know their Instances, the queue
    // knows its entries, neither knows the engine.
    let queued_ids: Vec<InstanceId> = queued.iter().map(|entry| entry.instance_id).collect();
    let (builds, queued_rows) = app
        .store
        .run(move |c| {
            let rows = queued_ids
                .iter()
                .map(|id| instance::get(c, id))
                .collect::<Result<Vec<_>, Error>>()?;

            Ok((build::all(c)?, rows))
        })
        .await?;
    let engine_of = |build_id: Option<BuildId>| {
        builds
            .iter()
            .find(|build| Some(build.id) == build_id)
            .map_or_else(|| UNKNOWN.to_owned(), |build| name(&build.engine))
    };

    let mut instances: BTreeMap<(String, String), u64> = BTreeMap::new();
    let owned = views.iter().map(|view| &view.instance);
    for instance in owned.chain(&queued_rows) {
        let key = (name(&instance.state), engine_of(instance.build_id));
        *instances.entry(key).or_default() += 1;
    }
    if !instances.is_empty() {
        family(
            &mut out,
            "brr_instances",
            "gauge",
            "Instances by state and engine",
        );
        for ((state, engine), count) in &instances {
            sample(
                &mut out,
                "brr_instances",
                &[("state", state), ("engine", engine)],
                count,
            );
        }
    }

    let mut tabs: BTreeMap<String, u64> = BTreeMap::new();
    for tab in views.iter().flat_map(|view| &view.tabs) {
        if tab.state != TabState::Closed {
            *tabs.entry(name(&tab.state)).or_default() += 1;
        }
    }
    if !tabs.is_empty() {
        family(&mut out, "brr_tabs", "gauge", "Open Tabs by state");
        for (state, count) in &tabs {
            sample(&mut out, "brr_tabs", &[("state", state)], count);
        }
    }

    // One sample per Instance, shared with the other readers for a few seconds.
    let browsers: Vec<_> = views
        .iter()
        .filter_map(|view| Some((view.instance.id, view.instance.pid?)))
        .collect();
    let pss = host::pss_by_instance(browsers).await;
    if !pss.is_empty() {
        family(
            &mut out,
            "brr_instance_rss_bytes",
            "gauge",
            "Memory of an Instance (Pss) in bytes",
        );
        let mut sorted: Vec<_> = pss.into_iter().collect();
        sorted.sort_by_key(|(instance_id, _)| instance_id.to_string());
        for (instance_id, bytes) in sorted {
            sample(
                &mut out,
                "brr_instance_rss_bytes",
                &[("instance", &instance_id.to_string())],
                bytes,
            );
        }
    }

    family(
        &mut out,
        "brr_admission_queue_depth",
        "gauge",
        "Launches waiting for room",
    );
    sample(
        &mut out,
        "brr_admission_queue_depth",
        &[],
        app.admission.queue().await.len(),
    );

    let mut leases: BTreeMap<String, u64> = BTreeMap::new();
    for lease in app.leases.list(None, None).await {
        *leases.entry(name(&lease.client_kind)).or_default() += 1;
    }
    if !leases.is_empty() {
        family(
            &mut out,
            "brr_leases",
            "gauge",
            "Active Leases by client kind",
        );
        for (kind, count) in &leases {
            sample(&mut out, "brr_leases", &[("client_kind", kind)], count);
        }
    }

    let kinds = app.store.run(|c| artifact::usage_by_kind(c)).await?;
    if !kinds.is_empty() {
        family(
            &mut out,
            "brr_storage_bytes",
            "gauge",
            "Bytes of stored Artifacts by kind",
        );
        for (kind, _, bytes) in &kinds {
            sample(
                &mut out,
                "brr_storage_bytes",
                &[("kind", &name(kind))],
                bytes,
            );
        }
    }

    app.captcha.render_metrics(&mut out);

    Ok(out)
}

fn family(out: &mut String, name: &str, kind: &str, help: &str) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} {kind}");
}

fn sample(out: &mut String, name: &str, labels: &[(&str, &str)], value: impl Display) {
    out.push_str(name);
    if !labels.is_empty() {
        let rendered: Vec<String> = labels
            .iter()
            .map(|(label, value)| format!("{label}=\"{}\"", escape(value)))
            .collect();
        let _ = write!(out, "{{{}}}", rendered.join(","));
    }
    let _ = writeln!(out, " {value}");
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

// Label values are enum names, serialized the way the API names them.
fn name(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        _ => UNKNOWN.to_owned(),
    }
}
