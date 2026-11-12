//! Logs every request to Octopus, see logging.rs.

use std::sync::Arc;
use std::time::Instant;

use bruce_graphql::{GraphQLCaller, GraphQLQuery, GraphQLResponse, ProgressMonitor};

use crate::data_set::{brief, is_transient};
use crate::{log_error, log_info, log_warning};

/// A caller which logs each request it makes, by its name and how long it took: INFO when it succeeds, with how much data
/// was returned, WARNING when it fails in a way which is retried, e.g. too many requests or a network error, see
/// is_transient, and ERROR otherwise.
pub struct LoggingCaller<C>(pub C);

impl<C: GraphQLCaller> GraphQLCaller for LoggingCaller<C> {
    async fn call3<Q: GraphQLQuery<R>, R: GraphQLResponse, P: ProgressMonitor + ?Sized>(&self, query: &Q, token: Option<&Arc<String>>, progress: &P)
        -> Result<R, bruce_graphql::Error>
    {
        let started = Instant::now();
        let result = self.0.call3(query, token, progress).await;
        let seconds = started.elapsed().as_secs_f64();
        let name = Q::get_request_name();
        match &result {
            Ok(response) => {
                // The size of the data as JSON, which is close to the size of the response.
                let size = serde_json::to_vec(response).map(|json| data_size(json.len())).unwrap_or_default();
                log_info!("Octopus {name} request returned {size} in {seconds:.2}s");
            },
            Err(error) if is_transient(error) => log_warning!("Octopus {name} request failed after {seconds:.2}s: {}", brief(error)),
            Err(error) => log_error!("Octopus {name} request failed after {seconds:.2}s: {}", brief(error)),
        }
        result
    }
}

/// A number of bytes, e.g. 812 bytes, 4.2 KB or 1.3 MB.
fn data_size(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} bytes"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

#[cfg(test)]
mod tests {
    use super::data_size;

    #[test]
    fn sizes_are_in_bytes_kilobytes_or_megabytes() {
        assert_eq!(data_size(812), "812 bytes");
        assert_eq!(data_size(4300), "4.2 KB");
        assert_eq!(data_size(1_363_149), "1.3 MB");
    }
}
