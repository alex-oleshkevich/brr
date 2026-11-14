use brr_core::{Capability, capture::CaptureParams};

use super::output;
use crate::services::{artifacts::Delivery, oneshot};

macro_rules! oneshot_op {
    ($spec:ident, $kind:ident, $name:literal, $summary:literal, $capability:expr) => {
        static $spec: brr_core::OperationSpec = brr_core::OperationSpec {
            name: concat!("oneshot.", $name),
            group: "oneshot",
            toolset: brr_core::Toolset::Browser,
            summary: $summary,
            http: brr_core::Http {
                method: brr_core::Method::Post,
                path: concat!("/v1/oneshot/", $name),
            },
            cli: &["shot", $name],
            cli_positional: &["url"],
            mcp_tool: Some(concat!("oneshot_", $name)),
            read_only: false,
            capability: $capability,
            input_schema: brr_core::schema::<CaptureParams>,
            output_schema: brr_core::schema::<Delivery>,
        };

        const _: () = {
            fn handler(
                ctx: crate::app::Ctx,
                input: serde_json::Value,
            ) -> futures::future::BoxFuture<'static, Result<serde_json::Value, brr_core::Error>>
            {
                futures::FutureExt::boxed(async move {
                    let params: CaptureParams = super::parse(input)?;
                    let delivery = oneshot::run(
                        &ctx.app,
                        &ctx.client,
                        brr_core::capture::OneshotKind::$kind,
                        params,
                    )
                    .await?;

                    output(&delivery)
                })
            }

            inventory::submit!(super::OpRegistration {
                spec: &$spec,
                handler
            });
        };
    };
}

oneshot_op!(
    SCREENSHOT,
    Screenshot,
    "screenshot",
    "Open a URL in a throwaway browser context and screenshot it",
    None
);
oneshot_op!(
    PDF,
    Pdf,
    "pdf",
    "Open a URL in a throwaway browser context and print it to PDF (Chromium only)",
    Some(Capability::Pdf)
);
oneshot_op!(
    MHTML,
    Mhtml,
    "mhtml",
    "Open a URL in a throwaway browser context and save it as MHTML (Chromium only)",
    Some(Capability::Mhtml)
);
oneshot_op!(
    MARKDOWN,
    Markdown,
    "markdown",
    "Open a URL in a throwaway browser context and return its main content as Markdown",
    None
);
oneshot_op!(
    CONTENT,
    Content,
    "content",
    "Open a URL in a throwaway browser context and return its rendered HTML",
    None
);
oneshot_op!(
    LINKS,
    Links,
    "links",
    "Open a URL in a throwaway browser context and list its links",
    None
);
