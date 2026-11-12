use std::{collections::HashSet, sync::Arc, time::Duration};

use bruce_graphql::{GraphQLCaller, GraphQLQuery, GraphQLResponse};
use serde::{Serialize, de::DeserializeOwned};

use super::DataSetAttributes;
use crate::{CacheConfig, OrderedMap};

/// A DataSet which is a list of records where the records returned from the API are in ascending order and new records can be
/// appended to the end of the list.
///
/// Each record is final or not, see the indexer, e.g. a closed bill or an open one, which may still change. A refresh
/// fetches from the earliest record which is not final, or if they all are, from the latest, see fetch_from. With
/// drop_missing_provisional, a record which is not final and is no longer returned is removed, e.g. an open bill which
/// the server replaced with another, see missing_provisional.
pub struct OrderedListDataSet<K, O, V> {
    pub data: OrderedMap<K, O, (DataSetAttributes, V)>,
}

impl<K, O, V> OrderedListDataSet<K, O, V>
where
    K: Eq + std::hash::Hash + Ord + Clone + Send + Sync + Serialize,
    O: Ord + Clone + Serialize,
    V: DeserializeOwned + Serialize,
{
    pub async fn new<R, Q, IQP, CQP, RI, IN, I>(
        hash_key: &str,
        refresh_after: Duration,
        initial_query_provider: IQP,
        continuation_query_provider: CQP,
        response_iterator: RI,
        indexer: IN,
        drop_missing_provisional: bool,
        config: &Arc<CacheConfig>,
        request_manager: &impl GraphQLCaller,
    ) -> anyhow::Result<Self>
    where
        R: GraphQLResponse,
        Q: GraphQLQuery<R>,
        IQP: FnOnce(Option<&V>) -> Result<Q, bruce_graphql::Error>,
        CQP: Fn(&R) -> Option<Result<Q, bruce_graphql::Error>>,
        RI: Fn(R) -> I,
        IN: Fn(&V) -> (K, O, bool),
        I: IntoIterator<Item = V>,
    {
        let mut data: OrderedMap<K, O, (DataSetAttributes, V)> = OrderedMap::new();
        let records = super::records::Records::open(config, hash_key)?;
        records.import_legacy(config, hash_key, &mut |records, json| {
            let value: V = serde_json::from_value(json.clone())?;
            let (index, order, _) = indexer(&value);
            records.put(&sort_key(&order, &index)?, json)
        })?;
        let mut refresh = records.fetched_age()?.is_none_or(|age| age > refresh_after);

        for (sk, value) in records.all()? {
            match serde_json::from_value(value) {
                Ok(value) => {
                    let (index, order, _is_final) = indexer(&value);
                    data.insert(index, order, (DataSetAttributes { cached: true }, value));
                }
                Err(error) => {
                    crate::log_warning!(
                        "Failed to read record {sk} of the cached {hash_key}, fetching it again: {error}"
                    );
                    refresh = true;
                    data.clear();
                    break;
                }
            }
        }

        if data.len() == 0 {
            refresh = true;
        }

        // An offline profile only has what is cached.
        if refresh && !config.offline {
            let from = fetch_from(data.iter().map(|(_, (_, value))| indexer(value)));
            let last_record = from
                .as_ref()
                .and_then(|(index, _)| data.get(index))
                .map(|(_, value)| value);
            let query = initial_query_provider(last_record)?;
            let mut response = request_manager.call(&query).await?;
            // The records returned, for drop_missing_provisional.
            let mut returned: HashSet<K> = HashSet::new();

            loop {
                let cq = continuation_query_provider(&response);

                for value in response_iterator(response) {
                    let (index, order, _is_final) = indexer(&value);
                    returned.insert(index.clone());

                    // Only a record which is new or has changed is put, as a new version.
                    let update = match data.get(&index) {
                        Some((_attrs, current_value)) => {
                            serde_json::to_value(&value)? != serde_json::to_value(current_value)?
                        }
                        None => true,
                    };

                    if update {
                        records.put(&sort_key(&order, &index)?, serde_json::to_value(&value)?)?;
                        data.insert(index, order, (DataSetAttributes { cached: false }, value));
                    }
                }

                if let Some(continuation_query) = cq {
                    let query = continuation_query?;

                    response = request_manager.call(&query).await?;
                } else {
                    break;
                }
            }

            if drop_missing_provisional {
                let from_order = from.as_ref().map(|(_, order)| order);
                let missing =
                    missing_provisional(data.iter().map(|(_, (_, value))| indexer(value)), from_order, &returned);
                if !missing.is_empty() {
                    crate::log_info!(
                        "Removed {} records which are no longer returned from {hash_key}",
                        missing.len()
                    );
                    for index in &missing {
                        if let Some((_, value)) = data.get(index) {
                            let (_, order, _) = indexer(value);
                            records.delete(&sort_key(&order, index)?)?;
                        }
                        data.remove(index);
                    }
                }
            }

            records.mark_fetched()?;
        };

        Ok(Self { data })
    }
}

/// The sort key of a record, its order then its key, e.g. 2026-10-01#474269026, each as text, or JSON if it is not.
fn sort_key<K: Serialize, O: Serialize>(order: &O, key: &K) -> anyhow::Result<String> {
    let text = |value: serde_json::Value| match value {
        serde_json::Value::String(text) => text,
        other => other.to_string(),
    };
    Ok(format!(
        "{}#{}",
        text(serde_json::to_value(order)?),
        text(serde_json::to_value(key)?)
    ))
}

/// The record to fetch from, as its key and order, from each record's key, order and whether it is final: the earliest
/// record which is not final, as it may have changed, if it is before the latest final one, otherwise the latest final
/// one, as new records come after it. None, to fetch them all, if none are final.
fn fetch_from<K, O: Ord>(records: impl Iterator<Item = (K, O, bool)>) -> Option<(K, O)> {
    let mut latest_final: Option<(K, O)> = None;
    let mut earliest_provisional: Option<(K, O)> = None;
    for (key, order, is_final) in records {
        if is_final {
            if latest_final.as_ref().is_none_or(|(_, latest)| order > *latest) {
                latest_final = Some((key, order));
            }
        } else if earliest_provisional
            .as_ref()
            .is_none_or(|(_, earliest)| order < *earliest)
        {
            earliest_provisional = Some((key, order));
        }
    }
    match (latest_final, earliest_provisional) {
        (Some(latest), Some(earliest)) if earliest.1 < latest.1 => Some(earliest),
        (latest, _) => latest,
    }
}

/// The keys of the records, from each one's key, order and whether it is final, which are not final, from the order
/// fetched from, or all of them if from is None, and were not returned.
fn missing_provisional<K: Eq + std::hash::Hash, O: Ord>(
    records: impl Iterator<Item = (K, O, bool)>,
    from: Option<&O>,
    returned: &HashSet<K>,
) -> Vec<K> {
    records
        .filter(|(key, order, is_final)| !is_final && from.is_none_or(|from| order >= from) && !returned.contains(key))
        .map(|(key, _, _)| key)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{fetch_from, missing_provisional};

    /// Bills as (id, from date, closed), as they were cached when an open statement was replaced, see BillDataSet.
    fn bills() -> Vec<(&'static str, &'static str, bool)> {
        vec![
            ("64400356", "2021-05-04", true),
            ("464800541", "2026-09-25", true),
            ("465467620", "2026-09-28", false),
            ("473394964", "2026-09-28", true),
            ("474269026", "2026-10-25", false),
        ]
    }

    #[test]
    fn a_refresh_is_from_the_earliest_record_which_is_not_final_by_order_not_key() {
        // The open statement from 28 Sep, rather than the 2021 bill, whose id is the largest as text.
        let from = fetch_from(bills().into_iter());
        assert_eq!(from.map(|(_, order)| order), Some("2026-09-28"));
        // With them all final, from the latest.
        let closed = bills().into_iter().map(|(id, date, _)| (id, date, true));
        assert_eq!(fetch_from(closed), Some(("474269026", "2026-10-25")));
        // With none final, all of them.
        let open = bills().into_iter().map(|(id, date, _)| (id, date, false));
        assert_eq!(fetch_from(open), None);
    }

    #[test]
    fn records_which_are_not_final_and_are_no_longer_returned_are_missing() {
        let returned: HashSet<&str> = ["473394964", "474269026"].into_iter().collect();
        assert_eq!(
            missing_provisional(bills().into_iter(), Some(&"2026-09-28"), &returned),
            ["465467620"]
        );
        // Final records, and those before the fetch, are never missing.
        assert!(missing_provisional(bills().into_iter(), Some(&"2026-10-25"), &returned).is_empty());
    }
}
