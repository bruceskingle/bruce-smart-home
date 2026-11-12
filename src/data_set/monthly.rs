//! A DataSet of records a month at a time, each as the API gave it, kept in the store, see records.rs, by its time in
//! UTC, see time_key, with a `#fetched/<month>` record of when each month was last fetched, which says whether it is
//! stale or complete.

use std::{collections::BTreeMap, time::Duration};

use serde::{Serialize, de::DeserializeOwned};
use bruce_graphql::{
    GraphQLCaller, GraphQLQuery, GraphQLResponse, ProgressMonitor,
    types::{Date, DateTime},
};

use crate::CacheConfig;
use super::DataSetAttributes;

/// A DataSet which is a monthly bucket containing a list of records where each record has a DateTime ordering and the records 
/// returned from the API are in ascending order
pub struct MonthlyDataSet<V>
{
    pub start_date: Date,
    pub next_date: Date,
    pub data: BTreeMap<DateTime, (DataSetAttributes, V)> ,
}

impl<V> MonthlyDataSet<V>
where
    V: DeserializeOwned + Serialize
{
    
    /// The month containing date, from the cache, fetched again if it is more than refresh_after old and not yet
    /// complete, or if force is set. A month is complete once it has been fetched more than refresh_after after it
    /// ended, so it has had time to get the data for its end, and then it is not fetched again unless forced. An empty
    /// month counts as fetched, but a new file is dated the epoch, so if the fetch fails it is fetched again next time.
    /// With force, the whole month is fetched rather than from the last final record.
    pub async fn new<R, Q, IQP, CQP, RI, IN, I>(
            hash_key: &str,
            date: &Date,
            refresh_after: Duration,
            force: bool,
            initial_query_provider: IQP,
            continuation_query_provider: CQP,
            response_iterator: RI,
            indexer: IN,
            config: &CacheConfig,
            request_manager: &impl GraphQLCaller,
            progress: &(impl ProgressMonitor + ?Sized),
        ) -> anyhow::Result<Self>
        where
            R: GraphQLResponse,
            Q: GraphQLQuery<R>,
            IQP: FnOnce(&Date, Option<&V>) -> Result<Q, bruce_graphql::Error>,
            CQP: Fn(&Date, &R) -> Option<Result<Q, bruce_graphql::Error>>,
            RI: Fn(R) -> I,
            IN: Fn(&V) -> (DateTime, bool),
            I: IntoIterator<Item = V>,
    {
        let (start, next) = super::month_bounds(date)?;
        let records = super::records::Records::open(config, hash_key)?;
        records.import_legacy(config, &super::legacy_month_file(hash_key, &start), &mut |records, json| {
            let value: V = serde_json::from_value(json.clone())?;
            records.put(&super::time_key(&indexer(&value).0), json)
        })?;
        let fetched_key = format!("#fetched/{}", &start.to_string()[..7]);
        // When the month was last fetched, from its #fetched record's version, see Records.
        let fetched_at = records.fetched_at(&fetched_key)?;
        let now = time::OffsetDateTime::now_utc();
        let stale = fetched_at.is_none_or(|at| now - at > refresh_after);
        let month_end = next.midnight().assume_utc();
        let complete = fetched_at.is_some_and(|at| at > month_end + refresh_after);
        let mut refresh = force || (stale && !complete);

        let mut data: BTreeMap<DateTime, (DataSetAttributes, V)> = BTreeMap::new();
        let mut last_final_index = None;
        let mut first_non_final_index = None;
        // A day either side, as the times are in UTC and the month's days in the API's own offset.
        let (from, to) = ((*start - time::Duration::days(1)).to_string(), (*next + time::Duration::days(1)).to_string());
        for (sk, value) in records.range(from.as_str()..to.as_str())? {
            match serde_json::from_value::<V>(value) {
                Ok(value) => {
                    let (index, is_final) = indexer(&value);
                    let date: time::Date = index.date();
                    if date < *start || date >= *next {
                        continue;
                    }
                    if is_final {
                        if last_final_index.as_ref().is_none_or(|current| index > *current) {
                            last_final_index = Some(index.clone());
                        }
                    }
                    else if first_non_final_index.as_ref().is_none_or(|current| index < *current) {
                        first_non_final_index = Some(index.clone());
                    }
                    data.insert(index, (DataSetAttributes { cached: true, }, value));
                },
                Err(error) => {
                    crate::log_warning!("Failed to read record {sk} of the cached {hash_key}, fetching the month again: {error}");
                    refresh = true;
                    data.clear();
                    last_final_index = None;
                    first_non_final_index = None;
                    break;
                },
            }
        }

        if refresh && !config.offline {
            // Forced, the month is fetched whole, and a record no longer returned is deleted.
            let previous: Vec<DateTime> = if force { data.keys().cloned().collect() } else { Vec::new() };
            if force {
                data.clear();
            }
            let last_index = match (&last_final_index, &first_non_final_index) {
                _ if force => None,
                (Some(last_final), Some(first_non_final)) if first_non_final < last_final => first_non_final_index.clone(),
                (Some(_), _) => last_final_index.clone(),
                (None, _) => None,
            };
            let last_record = last_index.and_then(|index| data.get(&index)).map(|(_, record)| record);
            let query = initial_query_provider(&start, last_record)?;
            let mut response = request_manager.call3(&query, None, progress).await?;

            loop {
                let cq = continuation_query_provider(&start, &response);

                for value in response_iterator(response) {
                    let (index, _is_final) = indexer(&value);
                    let date: time::Date = index.date();
                    if date >= *start && date < *next {
                        // Only a record which is new or has changed is put, as a new version.
                        let update = match data.get(&index) {
                            Some((_attrs, current_value)) => serde_json::to_value(&value)? != serde_json::to_value(current_value)?,
                            None => true,
                        };
                        if update {
                            records.put(&super::time_key(&index), serde_json::to_value(&value)?)?;
                            data.insert(index, (DataSetAttributes { cached: false, }, value));
                        }
                    }
                }

                if let Some(continuation_query) = cq {
                    let query = continuation_query?;
                    response = request_manager.call3(&query, None, progress).await?;
                }
                else {
                    break;
                }
            }
            for gone in previous.iter().filter(|index| !data.contains_key(*index)) {
                records.delete(&super::time_key(gone))?;
            }
            records.put(&fetched_key, serde_json::json!({}))?;
        };

        Ok(Self {
            start_date: start,
            next_date: next,
            data
        })
        
    }
}
