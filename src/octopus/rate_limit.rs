//! When a field which Octopus has blocked for too many requests can be requested again, from its rateLimitInfo query.

use bruce_graphql::{GraphQLCaller, NoProgress};

use super::graphql::meter::rate_limit;
use crate::data_set::{RateLimitBlock, RateLimitReset};

/// The rate limit of one field, named as rateLimitInfo names it, e.g. "Query.smartMeterTelemetry".
pub struct FieldRateLimit<'a, C: GraphQLCaller> {
    pub request_manager: &'a C,
    pub field: &'static str,
}

impl<C: GraphQLCaller> RateLimitReset for FieldRateLimit<'_, C> {
    async fn blocked_until(&self) -> Option<RateLimitBlock> {
        let response = match self
            .request_manager
            .call3(&rate_limit::Query::new(), None, &NoProgress)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                crate::log_warning!("Failed to find when the rate limit of {} ends: {error}", self.field);
                return None;
            }
        };
        let limit = response
            .rate_limit_info_?
            .field_specific_rate_limits_
            .edges_
            .into_iter()
            .filter_map(|edge| edge.node_)
            .find(|limit| limit.field_.as_deref() == Some(self.field) && limit.is_blocked_ == Some(true))?;
        Some(RateLimitBlock {
            until: limit.ttl_? as i64,
            rate: limit.rate_,
        })
    }
}
