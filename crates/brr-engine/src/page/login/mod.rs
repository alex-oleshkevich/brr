use std::sync::Arc;
use std::time::Duration;

use brr_core::Error;
use serde::Deserialize;

use super::{Page, PageState, Target};

#[cfg(test)]
mod tests;

const FIELDS_SCRIPT: &str = include_str!("fields.js");
const READY_POLL: Duration = Duration::from_millis(50);
const COMPLETE_SCRIPT: &str = "document.readyState === 'complete'";
const MASK_FUNCTION: &str = "function(node) {
    node.style.setProperty('-webkit-text-security', 'disc', 'important');
    return 'done';
}";

// CSS selectors that `Target::Selector` accepts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LoginFields {
    pub username: Option<String>,
    pub password: Option<String>,
    pub totp: Option<String>,
    pub submit: Option<String>,
}

// Keeps `evaluate` refused on the Page while it lives.
pub struct EvaluateLock(Arc<PageState>);

impl Drop for EvaluateLock {
    fn drop(&mut self) {
        self.0.unlock_evaluate();
    }
}

impl Page {
    // For an operation that types secrets: a script cannot be allowed to read them in any form
    // while it runs, however often the page navigates.
    pub fn lock_evaluate(&self) -> EvaluateLock {
        self.state.lock_evaluate();

        EvaluateLock(self.state.clone())
    }

    pub fn navigation_count(&self) -> u64 {
        *self.state.navigations().borrow()
    }

    // Until the main frame has navigated since `since` and its new document has loaded.
    pub async fn wait_navigated(&self, since: u64) {
        let mut count = self.state.navigations().subscribe();
        if count.wait_for(|count| *count > since).await.is_err() {
            return;
        }

        // Errors mean the document is still being replaced.
        while self.evaluate_utility(COMPLETE_SCRIPT).await.ok() != Some(true.into()) {
            tokio::time::sleep(READY_POLL).await;
        }
    }

    pub async fn login_fields(&self) -> Result<LoginFields, Error> {
        let value = self.evaluate_utility(FIELDS_SCRIPT).await?;

        serde_json::from_value(value).map_err(|error| Error::Internal {
            message: format!("unexpected login fields: {error}"),
        })
    }

    pub async fn password_visible(&self) -> Result<bool, Error> {
        let value = self
            .evaluate_utility(&format!("{FIELDS_SCRIPT}.passwordVisible"))
            .await?;

        Ok(value == true)
    }

    // Before the value is typed, so no frame or screenshot ever shows it.
    pub async fn mask(&self, target: &Target, timeout: Duration) -> Result<(), Error> {
        let element = self.resolve(target, timeout).await?;
        let result = self.call_element(&element, MASK_FUNCTION, []).await;
        self.release(&element);
        result?;

        Ok(())
    }
}
