//! A DataSet of one record, as the API gave it, kept in the store, see records.rs, by the sort key RECORD.

use std::time::Duration;

use bruce_graphql::{GraphQLCaller, GraphQLQuery, GraphQLResponse};

use crate::CacheConfig;

/// The sort key of the record.
const RECORD: &str = "-";

pub struct SingleRecordDataSet<R: GraphQLResponse>
{
    pub data: R,
}

impl<R: GraphQLResponse> SingleRecordDataSet<R>
{
    pub async fn new<Q, QP>(
            hash_key: &str,
            refresh_after: Duration,
            query_provider: QP,
            config: &CacheConfig,
            request_manager: &impl GraphQLCaller
        ) -> anyhow::Result<Self>
        where
            Q: GraphQLQuery<R>,
            QP: FnOnce() -> Result<Q, bruce_graphql::Error>,
    {
        let records = super::records::Records::open(config, hash_key)?;
        records.import_legacy(config, hash_key, &mut |records, value| records.put(RECORD, value))?;
        // An offline profile cannot refresh, so it always reads what is cached, however old.
        let mut refresh = !config.offline && records.fetched_age()?.is_none_or(|age| age > refresh_after);

        let mut data: Option<R> = None;

        if !refresh {
            match records.get(RECORD)?.map(serde_json::from_value::<R>) {
                Some(Ok(value)) => data = Some(value),
                Some(Err(error)) => {
                    crate::log_warning!("Failed to read the cached {hash_key}, fetching it again: {error}");
                    refresh = true;
                },
                None => refresh = true,
            }
        }

        // An offline profile only has what is cached.
        if refresh && !config.offline {
            let value = request_manager.call(&query_provider()?).await?;
            records.put(RECORD, serde_json::to_value(&value)?)?;
            records.mark_fetched()?;
            data = Some(value);
        }

        Ok(Self {
            data: data.ok_or_else(|| anyhow::anyhow!("{hash_key} is not in this offline profile's data"))?
        })
        
    }
}
