//! What the modules whose APIs are not GraphQL share for making requests, e.g. myenergi and Tesla. The GraphQL
//! equivalent is bruce_graphql's RequestManager. This may become a crate of its own which that is built on, once what
//! the APIs have in common is clearer.

use std::time::Duration;

use bruce_graphql::redact::redact_secrets;

include!(concat!(env!("OUT_DIR"), "/crate_info.rs"));

const DASHES: &str = "=======================================================================================================================";

/// A client builder with the app's user agent and a timeout, so that a device which does not answer cannot hang a
/// page.
pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(create_info::USER_AGENT)
        .timeout(Duration::from_secs(30))
}

/// Prints a response in verbose mode, with its secrets redacted, see redact_secrets.
pub fn print_response(verbose: bool, label: &str, json: &serde_json::Value) {
    if verbose {
        let mut redacted = json.clone();
        redact_secrets(&mut redacted);
        println!("\n{DASHES}\r{label} ");
        println!("{}", serde_json::to_string_pretty(&redacted).unwrap_or_default());
        println!("{DASHES}");
    }
}

/// The start of a response's text, for an error message, as the whole of it may be a long HTML page.
pub fn summary(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(200) {
        Some((index, _)) => format!("{}...", &text[..index]),
        None => text.to_string(),
    }
}
