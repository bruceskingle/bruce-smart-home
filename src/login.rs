//! What the app does when a module's source refuses its saved login, the same for every module. The module keeps the
//! login, so that its login page is filled in with it, and can show the secret which was refused, but removes any token
//! which it was given for it, and reports it with refused. A saved secret which cannot be decrypted is reported the same
//! way, see unreadable. Then:
//!
//! - the module's pages show its login page at once, with why, see views/module.rs, including after a restart, as why
//!   is kept in a file beside the module's tokens until it logs in again, see cleared;
//! - polling stops polling the module, so that it does not keep failing, or lock the login out, see polling.rs;
//! - Home's pages say which modules need logging in again, with a link to their login pages.
//!
//! A login the user gives on a login page which is refused is only shown there, as nothing saved was refused.

use std::path::PathBuf;

use dioxus::prelude::*;

use crate::AppContext;

/// The error a module returns when its source refused its saved login, see refused.
#[derive(Debug)]
pub struct LoginRefused {
    pub module_id: String,
    pub reason: String,
}

impl std::fmt::Display for LoginRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}, log in again", self.reason)
    }
}

impl std::error::Error for LoginRefused {}

/// Changes whenever a module's login is refused or given again, so that the pages which show it are drawn again.
static CHANGED: GlobalSignal<u64> = Signal::global(|| 0);

/// Where why the module's saved login was refused is kept until it logs in again.
fn reason_path(context: &AppContext, module_id: &str) -> anyhow::Result<PathBuf> {
    context.login_file_path(module_id)
}

/// Saves reason as why the module's saved login was refused, see saved_refusal.
fn save_reason(path: anyhow::Result<PathBuf>, module_id: &str, reason: &str) {
    let saved = path.and_then(|path| {
        Ok(std::fs::write(
            path,
            serde_json::to_string(&serde_json::json!({ "reason": reason }))?,
        )?)
    });
    if let Err(error) = saved {
        crate::log_error!("Unable to save why the {module_id} login was refused, {error:#}");
    }
}

/// Draws the pages which show refusals again, if there are pages, e.g. not for --fetch.
fn changed() {
    if dioxus::core::Runtime::try_current().is_some() {
        *CHANGED.write() += 1;
    }
}

/// Records that module_id's source refused its saved login, for reason, e.g. "GloBird refused the saved password", and
/// returns the error for the module to give.
pub fn refused(context: &AppContext, module_id: &str, reason: impl Into<String>) -> anyhow::Error {
    let reason = reason.into();
    crate::log_error!("{reason}, log in again");
    save_reason(reason_path(context, module_id), module_id, &reason);
    changed();
    LoginRefused {
        module_id: module_id.to_string(),
        reason,
    }
    .into()
}

/// Records that the saved login of module_id in profile_name could not be decrypted, for reason, as a refused login, so
/// that its login page says why it asks for it again, unless a refusal is already saved. It is called when the profile
/// file is read, before there is an AppContext, see profile.rs.
pub fn unreadable(profile_name: &str, module_id: &str, reason: String) {
    let path = AppContext::login_file_path_of(profile_name, module_id);
    if path.as_ref().is_ok_and(|path| path.exists()) {
        return;
    }
    // Nothing is drawn again, as the profile is read before its pages are drawn, which read the reason then.
    save_reason(path, module_id, &reason);
}

/// Records that module_id has logged in again, so that its pages are shown, and it is polled, again.
pub fn cleared(context: &AppContext, module_id: &str) {
    if let Ok(path) = reason_path(context, module_id)
        && path.exists()
        && let Err(error) = std::fs::remove_file(&path)
    {
        crate::log_error!("Unable to remove {}, {error:#}", path.display());
    }
    changed();
}

/// Why module_id's saved login was refused, if it was and it has not logged in since. In a component, it is drawn
/// again when that changes.
pub fn refusal(context: &AppContext, module_id: &str) -> Option<String> {
    if dioxus::core::Runtime::try_current().is_some() {
        let _ = CHANGED.read();
    }
    saved_refusal(context, module_id)
}

/// Why module_id's saved login was refused, as refusal, but without drawing anything again, e.g. for polling.
pub fn saved_refusal(context: &AppContext, module_id: &str) -> Option<String> {
    let text = std::fs::read_to_string(reason_path(context, module_id).ok()?).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json["reason"].as_str().map(String::from)
}

/// Whether module_id can be used, so that every page, and polling, agree: its saved login was not refused, see refused,
/// and it says it is logged in, see ModuleFactory::is_ready. A module may still have a token of a refused login, e.g.
/// Octopus, but it is not used until the user logs in again, as its login page says. An offline profile has no login.
pub async fn is_ready(
    context: &AppContext,
    module_id: &str,
    factory: &std::sync::Arc<dyn crate::ModuleFactory + Send>,
) -> anyhow::Result<bool> {
    if !context.profile.active_profile.offline && saved_refusal(context, module_id).is_some() {
        return Ok(false);
    }
    factory.is_ready().await
}

/// Whether error, or one of its causes, is a refused login, see refused.
pub fn is_refused(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.is::<LoginRefused>())
}

/// Says that module_id is not logged in, or that its saved login was refused, with a link to its login page, on a page
/// which would show what, e.g. "its electricity", from it.
#[component]
pub fn LogInLink(module_id: String, what: String) -> Element {
    let context_signal = use_context::<Signal<Option<std::sync::Arc<AppContext>>>>();
    let refusal = context_signal
        .read()
        .as_ref()
        .and_then(|context| refusal(context, &module_id));
    let link = rsx! {
        Link { to: crate::components::app::Route::Module { module_id: module_id.clone() }, "log in to {module_id}" }
    };
    match refusal {
        Some(refusal) => rsx! {
            p {
                crate::components::error_text::ErrorText { error: refusal.clone(), message: refusal.clone() }
                ", "
                {link}
                " again to see {what} here."
            }
        },
        None => rsx! {
            p { "To see {what} here, " {link} "." }
        },
    }
}
