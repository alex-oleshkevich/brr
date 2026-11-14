use std::collections::HashMap;
use std::pin::Pin;

use brr_core::Error;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{MAX_TEXT_BYTES, Page, REF_FUNCTION, cap};

const SNAPSHOT_FUNCTION: &str = r"function() {
    const root = document.body ?? document.documentElement;
    return this.ariaSnapshotWithRefs(root, { mode: 'ai' });
}";

// Not the one that assigns the refs the other tools resolve: this one leaves them as they are.
const PLAIN_SNAPSHOT_FUNCTION: &str = r"function() {
    const root = document.body ?? document.documentElement;
    return this.ariaSnapshotForExpectFailure(root, { mode: 'default' });
}";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiSnapshot {
    text: String,
    iframe_depths: HashMap<String, Value>,
}

// `- iframe [active] [ref=e4]` -> the line's indent and the ref.
fn iframe_ref(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    let rest = trimmed.strip_prefix("- iframe ")?;
    let rest = rest.strip_prefix("[active] ").unwrap_or(rest);
    let rest = rest.strip_prefix("[ref=")?;

    Some((indent, &rest[..rest.find(']')?]))
}

impl Page {
    // The latest snapshot's refs resolve: each frame's injected script keeps its last one.
    pub async fn snapshot(&self) -> Result<String, Error> {
        let main = self.state.main_frame();
        let text = self.snapshot_frame(&main).await?;

        Ok(cap(self.state.redact_owned(text), MAX_TEXT_BYTES))
    }

    // The tree without refs and without the iframes' content; earlier refs keep resolving.
    pub async fn snapshot_without_refs(&self) -> Result<String, Error> {
        let main = self.state.main_frame();
        let (_, mut remote) = self
            .call_injected(&main, PLAIN_SNAPSHOT_FUNCTION, &[], true)
            .await?;
        let text: String =
            serde_json::from_value(remote["value"].take()).map_err(|error| Error::Internal {
                message: format!("unexpected snapshot: {error}"),
            })?;

        Ok(cap(self.state.redact_owned(text), MAX_TEXT_BYTES))
    }

    fn snapshot_frame<'a>(
        &'a self,
        frame_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, Error>> + Send + 'a>> {
        Box::pin(async move {
            let (_, mut remote) = self
                .call_injected(frame_id, SNAPSHOT_FUNCTION, &[], true)
                .await?;
            let snapshot: AiSnapshot =
                serde_json::from_value(remote["value"].take()).map_err(|error| {
                    Error::Internal {
                        message: format!("unexpected snapshot: {error}"),
                    }
                })?;

            let mut lines: Vec<String> = Vec::new();
            for line in snapshot.text.lines() {
                let Some((indent, iframe)) = iframe_ref(line) else {
                    lines.push(line.to_owned());
                    continue;
                };

                // A frame that is gone or not yet attached contributes nothing.
                let child = if snapshot.iframe_depths.contains_key(iframe) {
                    self.child_snapshot(frame_id, iframe)
                        .await
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                if child.is_empty() {
                    lines.push(line.to_owned());
                    continue;
                }
                lines.push(format!("{line}:"));
                lines.extend(child.lines().map(|child| format!("{indent}  {child}")));
            }

            Ok(lines.join("\n"))
        })
    }

    async fn child_snapshot(&self, frame_id: &str, iframe: &str) -> Result<String, Error> {
        let (session_id, remote) = self
            .call_injected(frame_id, REF_FUNCTION, &[json!(iframe)], false)
            .await?;
        let Some(object_id) = remote["objectId"].as_str() else {
            return Ok(String::new());
        };

        let context_id = self
            .state
            .frame(frame_id)
            .and_then(|frame| frame.utility_context);
        let child = self
            .session
            .content_frame(&session_id, frame_id, object_id)
            .await;
        // Only the frame id was wanted; the element must not stay pinned in the page.
        self.session.drop_object(&session_id, context_id, object_id);

        match child? {
            Some(child) if self.state.owns_frame(&child) => self.snapshot_frame(&child).await,
            _ => Ok(String::new()),
        }
    }
}
