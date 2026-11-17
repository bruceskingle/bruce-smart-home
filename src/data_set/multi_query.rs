//! A DataSet of the responses of several queries, as the API gave them, kept in the store, see records.rs, each by its
//! place.

use std::time::Duration;

use anyhow::anyhow;
use bruce_graphql::{GraphQLCaller, GraphQLQuery, GraphQLResponse};

use super::DataSetAttributes;
use crate::CacheConfig;

/// A DataSet which is a list of records where each record is returned from the API in a single call
pub struct MultiQueryDataSet<R> {
    pub data: Vec<R>,
    pub _attributes: DataSetAttributes,
}

impl<R> MultiQueryDataSet<R>
where
    R: GraphQLResponse,
{
    pub async fn new<Q, QI>(
        hash_key: &str,
        refresh_after: Duration,
        query_iterator: QI,
        config: &CacheConfig,
        request_manager: &impl GraphQLCaller,
    ) -> anyhow::Result<Self>
    where
        Q: GraphQLQuery<R>,
        QI: IntoIterator<Item = Result<Q, bruce_graphql::Error>>,
    {
        let mut data: Vec<R> = Vec::new();
        let records = super::records::Records::open(config, hash_key)?;
        let mut place = 0;
        records.import_legacy(config, hash_key, &mut |records, value| {
            place += 1;
            records.put(&format!("{:08}", place - 1), value)
        })?;
        let mut _attributes = DataSetAttributes { cached: true };
        // An offline profile cannot refresh, so it always reads what is cached, however old.
        let mut refresh = !config.offline && records.fetched_age()?.is_none_or(|age| age > refresh_after);

        if !refresh {
            for (sk, value) in records.all()? {
                match serde_json::from_value(value) {
                    Ok(value) => data.push(value),
                    Err(error) => {
                        crate::log_warning!(
                            "Failed to read record {sk} of the cached {hash_key}, fetching it again: {error}"
                        );
                        refresh = true;
                        break;
                    }
                }
            }
        }

        if data.len() == 0 {
            refresh = true;
        }

        // An offline profile only has what is cached.
        if refresh && !config.offline {
            let previous = records.all()?.len();
            data.clear();
            _attributes.cached = false;

            // Each response by its place, those of a longer set fetched before deleted.
            for query_result in query_iterator {
                match query_result {
                    Ok(query) => {
                        let response = request_manager.call(&query).await?;
                        records.put(&format!("{:08}", data.len()), serde_json::to_value(&response)?)?;
                        data.push(response);
                    }
                    Err(err) => {
                        return Err(anyhow!(format!("Failed to get query {}", err)));
                    }
                }
            }
            for gone in data.len()..previous {
                records.delete(&format!("{gone:08}"))?;
            }
            records.mark_fetched()?;
        }

        Ok(Self { data, _attributes })
    }

    // fn handle_records<R, RI, IN, I>(
    //     response: R,
    //     response_iterator: RI,
    //     indexer: IN,
    //     data: &mut OrderedMap<K, O, (DataSetAttributes, V)>,
    //     file: &mut std::fs::File
    // )   -> anyhow::Result<()>

    //     where
    //         R: GraphQLResponse,
    //         RI: FnOnce(R) -> I,
    //         IN: Fn(&V) -> (K, O),
    //         I: IntoIterator<Item = V>,
    // {
    //     for value in response_iterator(response) {
    //         let (index, order) = indexer(&value);
    //         if ! data.contains_key(&index) {
    //             writeln!(file, "{}", serde_json::to_string(&value)?)?;
    //             data.insert(index, order, (DataSetAttributes { cached: false, }, value));
    //         }
    //     }
    //     Ok(())
    // }
}
