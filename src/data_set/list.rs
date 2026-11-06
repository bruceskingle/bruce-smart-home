//! A DataSet of a list of records, as the API gave them, a page at a time, kept in the store, see records.rs, each by
//! its place in the list, so that the order they were first given is kept.

use std::{sync::Arc, time::Duration};

use indexmap::IndexMap;
use serde::{Serialize, de::DeserializeOwned};
use sparko_graphql::{GraphQLCaller, GraphQLQuery, GraphQLResponse};

use crate::{CacheConfig};
use super::DataSetAttributes;

/// A DataSet which is a list of records where the records returned paginated from the API
pub struct ListDataSet<K, V>
{
    pub data: IndexMap<K, V> ,
    pub _attributes: DataSetAttributes,
}

impl<K, V> ListDataSet<K, V>
where
    K: Eq + std::hash::Hash + Ord + Clone + Send + Sync,
    V: DeserializeOwned + Serialize
{
   pub async fn new<R, Q, IQP, CQP, RI, IN, I>(
            hash_key: &str,
            refresh_after: Option<Duration>,
            initial_query_provider: IQP,
            continuation_query_provider: CQP,
            response_iterator: RI,
            indexer: IN,
            config: &Arc<CacheConfig>,
            request_manager: &impl GraphQLCaller
        ) -> anyhow::Result<Self>
        where
            R: GraphQLResponse,
            Q: GraphQLQuery<R>,
            IQP: FnOnce() -> Result<Q, sparko_graphql::Error>,
            CQP: Fn(&R) -> Option<Result<Q, sparko_graphql::Error>>,
            RI: Fn(R) -> I,
            IN: Fn(&V) -> K,
            I: IntoIterator<Item = V>,
    {

        let mut attributes = DataSetAttributes { cached: true, };
        let mut data: IndexMap<K, V> = IndexMap::new();
        let records = super::records::Records::open(config, hash_key)?;
        let mut place = 0;
        records.import_legacy(config, hash_key, &mut |records, value| {
            place += 1;
            records.put(&format!("{:08}", place - 1), value)
        })?;
        // An offline profile cannot refresh, so it always reads what is cached, however old.
        let mut refresh = match refresh_after {
            Some(refresh_after) if !config.offline => records.fetched_age()?.is_none_or(|age| age > refresh_after),
            _ => false,
        };

        if !refresh {
            for (sk, value) in records.all()? {
                match serde_json::from_value::<V>(value) {
                    Ok(value) => {
                        let index = indexer(&value);
                        data.insert(index, value);
                    },
                    Err(error) => {
                        crate::log_warning!("Failed to read record {sk} of the cached {hash_key}, fetching it again: {error}");
                        refresh = true;
                        data.clear();
                        break;
                    },
                }
            }
        }

        if data.len() == 0 {
            refresh = true;
        }

        // An offline profile only has what is cached.
        if refresh && !config.offline {
            let query = initial_query_provider()?;
            let mut response = request_manager.call(&query).await?;

            attributes.cached = false;
            // The records are kept in the order they were first given, by their place in it.
            let mut next = records.all()?.len();

            loop {
                let cq = continuation_query_provider(&response);

                for value in response_iterator(response) {
                    let index = indexer(&value);
                    if ! data.contains_key(&index) {
                        records.put(&format!("{next:08}"), serde_json::to_value(&value)?)?;
                        next += 1;
                        data.insert(index, value);
                    }
                }

                if let Some(continuation_query) = cq {
                    let query = continuation_query?;

                    response = request_manager.call(&query).await?;
                }
                else {
                    break;
                }
            }
            records.mark_fetched()?;
        };

        Ok(Self {
            data,
            _attributes: attributes,
        })
        
    }
}
