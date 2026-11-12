//! An error shown briefly, e.g. Not reachable, in the invalid style, with the whole of it as a tooltip, as the whole
//! of an error, e.g. a URL and the chain of causes, is long, and makes a mess of the page.

use dioxus::prelude::*;

/// error, shown as message if given, otherwise as short_error says, with the whole of error as a tooltip, which, as
/// every tooltip does, shows at once when clicked, see tooltips.rs.
#[component]
pub fn ErrorText(error: String, message: Option<String>) -> Element {
    let shown = message.unwrap_or_else(|| short_error(&error));
    rsx! { span { class: "invalid", title: "{error}", "{shown}" } }
}

/// The longest a short error is, see short_error.
const SHORT_LENGTH: usize = 40;

/// What an error means in a few words, e.g. Timed out, for the usual errors of fetching, otherwise its start.
pub fn short_error(error: &str) -> String {
    let lower = error.to_lowercase();
    let known = [
        (
            ["tcp connect", "connection refused", "client error (connect)"].as_slice(),
            "Not reachable",
        ),
        (
            &["deadline has elapsed", "timed out", "operation timed out"],
            "Timed out",
        ),
        (&["dns error", "failed to lookup address"], "Not found on the network"),
        (
            &["connection reset", "connection closed", "broken pipe"],
            "Connection lost",
        ),
        (&["error decoding response body", "invalid chunk"], "Bad response"),
        (&["(401)", "(403)", "unauthorized", "unauthorised"], "Refused"),
        (&["certificate"], "Certificate problem"),
    ];
    if let Some((_, short)) = known
        .iter()
        .find(|(patterns, _)| patterns.iter().any(|pattern| lower.contains(pattern)))
    {
        return short.to_string();
    }
    // Up to the first cause.
    let start = error.split([',', ':']).next().unwrap_or(error).trim();
    match start.char_indices().nth(SHORT_LENGTH) {
        Some((at, _)) => format!("{}...", &start[..at]),
        None => start.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usual_errors_are_a_few_words() {
        assert_eq!(
            short_error(
                "error sending request for url (http://192.168.4.97/api/capture/tesla/records?since=2026-10-30T06%3A55%3A50Z&limit=500): client error (Connect): tcp connect error: deadline has elapsed: deadline has elapsed"
            ),
            "Not reachable"
        );
        assert_eq!(
            short_error("error decoding response body: request or response body error: operation timed out"),
            "Timed out"
        );
        assert_eq!(
            short_error("the gateway said no, try again later"),
            "the gateway said no"
        );
    }
}
