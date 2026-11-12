use anyhow::anyhow;
use std::sync::Arc;
use std::time::Duration;

use crate::CacheConfig;
use crate::data_set::SingleRecordDataSet;
use bruce_graphql::{GraphQLCaller, GraphQLQueryBuilder};

use super::graphql::meter;

pub struct MeterPointDataSet {
    pub meter_points: Vec<meter::active_meter_points_with_agreements::ElectricityMeterPointType>,
}

impl MeterPointDataSet {
    pub async fn new(account_number: &String, config: &Arc<CacheConfig>, request_manager: &impl GraphQLCaller)  -> anyhow::Result<Self> {
        // Named for the time of use schemes, which earlier versions did not fetch, so that they are fetched at once.
        let hash_key = format!("{}#MeterPointsWithTimesOfUse", account_number);
        let query_provider = || {
            meter::active_meter_points_with_agreements::Query::builder()
                .with_account_number(account_number.clone())
                .build()
        };
        let data_set = SingleRecordDataSet::new(
            &hash_key,
            Duration::from_hours(24),
            query_provider,
            config,
            request_manager
        ).await?;

        let mut meter_points = Vec::new();
        let account = if let Some(account) = data_set.data.account_ {
            account
        }
        else {
            return Err(anyhow!("No account found"));
        };

        for property in account.properties_ {
            for point in property.electricity_meter_points_ {
                meter_points.push(point);
            }
        }

        Ok(Self {
            meter_points,
        })
    }
}