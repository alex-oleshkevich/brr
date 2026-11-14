use brr_core::{
    ArtifactKind, Error, TabId, TextOut,
    capture::{Area, ImageFormat, PdfOptions, ReturnMode, validate_image, validate_pdf},
};
use brr_engine::page::{Clip, Link, Metadata, ScreenshotOptions, Target};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    output,
    tool::{TabCall, TabInput, tab_call, tab_op},
};
use crate::{
    app::Ctx,
    internal,
    services::artifacts::{self, Delivery},
};

const MHTML_MIME: &str = "multipart/related";
const PDF_MIME: &str = "application/pdf";
const MHTML_FILENAME: &str = "page.mhtml";
const PDF_FILENAME: &str = "page.pdf";

tab_op!(SCREENSHOT {
    tool: "screenshot",
    cli: "screenshot",
    summary: "Screenshot an element (ref or selector), an area, the full page or the viewport",
    positional: &[],
    read_only: true,
    capability: None,
    input: ScreenshotInput,
    output: Delivery,
    run: screenshot,
});
tab_op!(PDF {
    tool: "pdf",
    cli: "pdf",
    summary: "Print the page to PDF (Chromium only)",
    positional: &[],
    read_only: true,
    capability: Some(brr_core::Capability::Pdf),
    input: PdfInput,
    output: Delivery,
    run: pdf,
});
tab_op!(MHTML {
    tool: "mhtml",
    cli: "mhtml",
    summary: "Save the page as a single MHTML file (Chromium only)",
    positional: &[],
    read_only: true,
    capability: Some(brr_core::Capability::Mhtml),
    input: MhtmlInput,
    output: Delivery,
    run: mhtml,
});
tab_op!(GET_HTML {
    tool: "get_html",
    cli: "get-html",
    summary: "Get the page's HTML",
    positional: &[],
    read_only: true,
    capability: None,
    input: TabInput,
    output: ContentOut,
    run: get_html,
});
tab_op!(GET_TEXT {
    tool: "get_text",
    cli: "get-text",
    summary: "Get the page's visible text",
    positional: &[],
    read_only: true,
    capability: None,
    input: TabInput,
    output: ContentOut,
    run: get_text,
});
tab_op!(GET_MARKDOWN {
    tool: "get_markdown",
    cli: "get-markdown",
    summary: "Get the page's main content as Markdown",
    positional: &[],
    read_only: true,
    capability: None,
    input: TabInput,
    output: ContentOut,
    run: get_markdown,
});
tab_op!(SNAPSHOT {
    tool: "snapshot",
    cli: "snapshot",
    summary: "Get the page's accessibility snapshot; its refs address elements in click, fill and the other element tools",
    positional: &[],
    read_only: true,
    capability: None,
    input: TabInput,
    output: SnapshotOut,
    run: snapshot,
});
tab_op!(GET_LINKS {
    tool: "get_links",
    cli: "get-links",
    summary: "List the page's links",
    positional: &[],
    read_only: true,
    capability: None,
    input: TabInput,
    output: LinksOut,
    run: get_links,
});
tab_op!(GET_METADATA {
    tool: "get_metadata",
    cli: "get-metadata",
    summary: "Get the page's title, description, canonical URL, language and Open Graph tags",
    positional: &[],
    read_only: true,
    capability: None,
    input: TabInput,
    output: Metadata,
    run: get_metadata,
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScreenshotInput {
    tab_id: Option<TabId>,
    r#ref: Option<String>,
    selector: Option<String>,
    area: Option<Area>,
    full_page: Option<bool>,
    format: Option<ImageFormat>,
    quality: Option<u8>,
    transparent: Option<bool>,
    #[serde(rename = "return")]
    return_mode: Option<ReturnMode>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PdfInput {
    tab_id: Option<TabId>,
    paper: Option<String>,
    landscape: Option<bool>,
    margin: Option<String>,
    pdf_scale: Option<f64>,
    pages: Option<String>,
    print_background: Option<bool>,
    #[serde(rename = "return")]
    return_mode: Option<ReturnMode>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MhtmlInput {
    tab_id: Option<TabId>,
    #[serde(rename = "return")]
    return_mode: Option<ReturnMode>,
}

#[derive(Serialize, JsonSchema)]
struct ContentOut {
    url: String,
    content: TextOut,
}

#[derive(Serialize, JsonSchema)]
struct SnapshotOut {
    url: String,
    title: String,
    snapshot: TextOut,
}

#[derive(Serialize, JsonSchema)]
struct LinksOut {
    links: Vec<Link>,
}

async fn screenshot(ctx: Ctx, input: ScreenshotInput) -> Result<Value, Error> {
    validate_image(
        input.area.as_ref(),
        input.selector.is_some() || input.r#ref.is_some(),
        input.full_page,
        input.format,
        input.quality,
        input.transparent,
    )?;
    let clip = match (input.area, input.full_page) {
        (Some(area), _) => Clip::Area(area),
        (None, _) if input.r#ref.is_some() || input.selector.is_some() => {
            Clip::Element(Target::from_input(input.r#ref, input.selector)?)
        }
        (None, Some(true)) => Clip::FullPage,
        (None, _) => Clip::Viewport,
    };
    let format = input.format.unwrap_or(ImageFormat::Png);
    let call = tab_call(&ctx, input.tab_id, &SCREENSHOT).await?;

    let image = call
        .page
        .screenshot(&ScreenshotOptions {
            clip,
            format,
            quality: input.quality,
            transparent: input.transparent.unwrap_or(false),
        })
        .await?;

    deliver(
        &ctx,
        &call,
        ArtifactKind::Screenshot,
        &format!("screenshot.{}", format.extension()),
        format.mime(),
        image,
        input.return_mode,
    )
    .await
}

async fn pdf(ctx: Ctx, input: PdfInput) -> Result<Value, Error> {
    let options = PdfOptions {
        paper: input.paper,
        landscape: input.landscape,
        margin: input.margin,
        pdf_scale: input.pdf_scale,
        pages: input.pages,
        print_background: input.print_background,
    };
    validate_pdf(&options)?;
    let call = tab_call(&ctx, input.tab_id, &PDF).await?;

    let bytes = call.page.pdf(&options).await?;

    deliver(
        &ctx,
        &call,
        ArtifactKind::Pdf,
        PDF_FILENAME,
        PDF_MIME,
        bytes,
        input.return_mode,
    )
    .await
}

async fn mhtml(ctx: Ctx, input: MhtmlInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &MHTML).await?;

    let bytes = call.page.mhtml().await?;

    deliver(
        &ctx,
        &call,
        ArtifactKind::Mhtml,
        MHTML_FILENAME,
        MHTML_MIME,
        bytes,
        input.return_mode,
    )
    .await
}

pub(super) async fn deliver(
    ctx: &Ctx,
    call: &TabCall,
    kind: ArtifactKind,
    filename: &str,
    mime: &str,
    data: Vec<u8>,
    return_mode: Option<ReturnMode>,
) -> Result<Value, Error> {
    let delivery = artifacts::deliver(
        &ctx.app,
        kind,
        call.owner(),
        filename,
        mime,
        data,
        return_mode.unwrap_or_default(),
    )
    .await?;

    output(&delivery)
}

async fn get_html(ctx: Ctx, input: TabInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &GET_HTML).await?;

    output(&ContentOut {
        url: call.page.url().await?,
        content: TextOut(call.page.html().await?),
    })
}

async fn get_text(ctx: Ctx, input: TabInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &GET_TEXT).await?;

    output(&ContentOut {
        url: call.page.url().await?,
        content: TextOut(call.page.text().await?),
    })
}

async fn get_markdown(ctx: Ctx, input: TabInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &GET_MARKDOWN).await?;

    let url = call.page.url().await?;
    let html = call.page.html().await?;
    let page_url = url.clone();
    let markdown = tokio::task::spawn_blocking(move || brr_content::markdown(&html, &page_url))
        .await
        .map_err(internal)??;

    output(&ContentOut {
        url,
        content: TextOut(markdown),
    })
}

// Only the Lease holder's snapshot hands out refs: a snapshot by anyone else would replace the
// ones the holder's next click resolves.
async fn snapshot(ctx: Ctx, input: TabInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &SNAPSHOT).await?;

    let lease = ctx.app.leases.tab_lease(&call.tab_id).await;
    let with_refs = lease.is_none_or(|lease| lease.client_id == ctx.client.client_id);
    let tree = if with_refs {
        call.page.snapshot().await?
    } else {
        call.page.snapshot_without_refs().await?
    };
    let (url, title) = call.info().await?;

    output(&SnapshotOut {
        url,
        title,
        snapshot: TextOut(tree),
    })
}

async fn get_links(ctx: Ctx, input: TabInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &GET_LINKS).await?;

    output(&LinksOut {
        links: call.page.links().await?,
    })
}

async fn get_metadata(ctx: Ctx, input: TabInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &GET_METADATA).await?;

    output(&call.page.metadata().await?)
}
