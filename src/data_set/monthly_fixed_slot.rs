use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    pin::pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures::future::{Either, select};

use bruce_graphql::{
    GraphQLCaller, GraphQLQuery, GraphQLResponse, ProgressMonitor,
    types::{Date, DateTime},
};
use serde::{Serialize, de::DeserializeOwned};
use time_tz::Tz;

use crate::CacheConfig;

use super::{CancelToken, DataSetAttributes};

/// The window from start to end split at each midnight in tz, as unix timestamps.
fn day_windows(start: &DateTime, end: &DateTime, tz: &'static Tz) -> Vec<(i64, i64)> {
    let mut days = Vec::new();
    let mut day_start = *start;
    while day_start < *end {
        let day_end = day_start.to_date().next_day().at_midnight(tz).min(*end);
        days.push((day_start.unix_timestamp(), day_end.unix_timestamp()));
        day_start = day_end;
    }
    days
}

/// Whether the final samples in data cover the whole of the window from start to end with no gaps.
fn window_complete<V>(data: &BTreeMap<DateTime, Item<V>>, start: &DateTime, end: &DateTime) -> bool {
    let mut covered_to = *start;
    for (sample_start, item) in data.range(start..end) {
        if *sample_start != covered_to || !item.is_final {
            return false;
        }
        covered_to = item.end;
    }
    covered_to >= *end
}

/// The number of times a request which failed with a transient error is retried.
const RETRIES: u32 = 5;
/// The delay before the first retry, which doubles for each retry after it.
const FIRST_RETRY_DELAY: Duration = Duration::from_secs(2);

/// The error code of Octopus's rate limit, which is a GraphQL error rather than an HTTP 429.
const TOO_MANY_REQUESTS: &str = "KT-CT-1199";

/// Whether a request failed as too many requests have been made.
fn is_rate_limited(error: &bruce_graphql::Error) -> bool {
    match error {
        bruce_graphql::Error::HttpError(status, _) => status.as_u16() == 429,
        bruce_graphql::Error::GraphQLError(errors) => errors
            .iter()
            .any(|error| error.extensions.get("errorCode").and_then(|code| code.as_str()) == Some(TOO_MANY_REQUESTS)),
        _ => false,
    }
}

/// Whether a request which failed with this error may succeed if it is made again, e.g. after a network error, a
/// gateway timeout or too many requests.
pub fn is_transient(error: &bruce_graphql::Error) -> bool {
    match error {
        bruce_graphql::Error::IOError(_) => true,
        bruce_graphql::Error::HttpError(status, _) => status.is_server_error() || is_rate_limited(error),
        _ => is_rate_limited(error),
    }
}

/// When a rate limit which blocks a request ends, as a unix timestamp, and the rate it allows, e.g. "125/h".
pub struct RateLimitBlock {
    pub until: i64,
    pub rate: Option<String>,
}

/// A way of finding out when a rate limit ends, so that a request which was refused for too many requests is made
/// again then, rather than after the retry delays, which are far too short for e.g. an hourly limit.
pub trait RateLimitReset {
    /// None if it is not known, e.g. the request for it failed too.
    fn blocked_until(&self) -> impl Future<Output = Option<RateLimitBlock>>;
}

/// No way of finding out when a rate limit ends, so the retry delays are used.
pub struct NoRateLimitReset;

impl RateLimitReset for NoRateLimitReset {
    async fn blocked_until(&self) -> Option<RateLimitBlock> {
        None
    }
}

/// The number of times a request is made again after waiting for a rate limit to end, in case it is still blocked.
const RATE_LIMIT_WAITS: u32 = 3;
/// How long after a rate limit ends a request is made again, in case of a difference between the clocks.
const RATE_LIMIT_MARGIN: i64 = 5;

/// A rate as Octopus gives it, e.g. "125/h", in words, e.g. "125 requests an hour".
fn rate_in_words(rate: &str) -> String {
    let Some((count, unit)) = rate.split_once('/') else {
        return String::from(rate);
    };
    let per = match unit {
        "s" => "a second",
        "m" => "a minute",
        "h" => "an hour",
        "d" => "a day",
        _ => return String::from(rate),
    };
    format!("{count} requests {per}")
}

/// A number of seconds as minutes and seconds, e.g. 4:05.
fn minutes_and_seconds(seconds: i64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// A short description of a transient error, for progress messages, e.g. "Too many requests" rather than the whole
/// GraphQL error.
pub fn brief(error: &bruce_graphql::Error) -> String {
    match error {
        bruce_graphql::Error::GraphQLError(errors) => {
            let messages: Vec<&str> = errors
                .iter()
                .filter_map(|error| error.message.as_deref())
                .map(|message| message.trim_end_matches('.'))
                .collect();
            if messages.is_empty() {
                String::from("Octopus returned an error")
            } else {
                messages.join(", ")
            }
        }
        bruce_graphql::Error::IOError(_) => String::from("Could not connect to Octopus"),
        bruce_graphql::Error::HttpError(status, _) => format!("Octopus returned {status}"),
        _ => error.to_string(),
    }
}

/// Make a request, returning None without waiting for the response if the fetch is cancelled first.
/// A request which fails with a transient error is made again, with the same query and so the same cursor, after a
/// delay, or if it was refused for too many requests and rate_limit knows when that ends, then. Progress messages from
/// the request are passed on to progress with the given prefix.
pub async fn call_unless_cancelled<Q: GraphQLQuery<R>, R: GraphQLResponse>(
    request_manager: &impl GraphQLCaller,
    query: &Q,
    cancel: &CancelToken,
    progress: &(impl ProgressMonitor + ?Sized),
    prefix: &str,
    rate_limit: &impl RateLimitReset,
) -> anyhow::Result<Option<R>> {
    let request_progress = |message: String| progress.progress(format!("{prefix}: {message}"));
    // Sleeps for the duration, returning false if the fetch is cancelled first.
    let sleep = |duration: Duration| async move {
        matches!(
            select(pin!(tokio::time::sleep(duration)), pin!(cancel.cancelled())).await,
            Either::Left(_)
        )
    };
    let mut delay = FIRST_RETRY_DELAY;
    let mut retries = 0;
    let mut rate_limit_waits = 0;
    loop {
        let error = match select(
            pin!(request_manager.call3(query, None, &request_progress)),
            pin!(cancel.cancelled()),
        )
        .await
        {
            Either::Right(_) => return Ok(None),
            Either::Left((Ok(response), _)) => return Ok(Some(response)),
            Either::Left((Err(error), _)) => error,
        };
        let block = if rate_limit_waits < RATE_LIMIT_WAITS && is_rate_limited(&error) {
            rate_limit.blocked_until().await
        } else {
            None
        };
        if let Some(block) = block {
            rate_limit_waits += 1;
            let allowed = block
                .rate
                .as_deref()
                .map(|rate| format!(", Octopus allows {}", rate_in_words(rate)))
                .unwrap_or_default();
            crate::log_warning!(
                "{prefix}: {}{allowed}, waiting {} for it to continue",
                brief(&error),
                minutes_and_seconds(
                    (block.until + RATE_LIMIT_MARGIN
                        - SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|now| now.as_secs() as i64)
                            .unwrap_or_default())
                    .max(0)
                )
            );
            // Counts down to the end of the block, a second at a time.
            loop {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|now| now.as_secs() as i64)
                    .unwrap_or_default();
                let left = block.until + RATE_LIMIT_MARGIN - now;
                if left <= 0 {
                    break;
                }
                request_progress(format!(
                    "{}{allowed}, continuing in {}",
                    brief(&error),
                    minutes_and_seconds(left)
                ));
                if !sleep(Duration::from_secs(1)).await {
                    return Ok(None);
                }
            }
        } else if retries < RETRIES && is_transient(&error) {
            retries += 1;
            request_progress(format!(
                "{}, trying again in {}s (retry {retries} of {RETRIES})",
                brief(&error),
                delay.as_secs()
            ));
            if !sleep(delay).await {
                return Ok(None);
            }
            delay *= 2;
        } else {
            return Err(error.into());
        }
    }
}

pub trait SlotSource {
    type Value: DeserializeOwned + Serialize + Clone;
    /// What is kept of a sample, as the server gave it, from which its Value is made, see value, so that the store
    /// holds only what the server sent, see AGENTS.md.
    type Stored: DeserializeOwned + Serialize + Clone;
    type Response: GraphQLResponse;
    type Query: GraphQLQuery<Self::Response>;

    fn hash_key(&self) -> &str;
    /// A query for the samples from start up to end. A source whose query has no end may return samples after it.
    fn initial_query(&mut self, start: &DateTime, end: &DateTime) -> anyhow::Result<Self::Query>;
    /// The query for the next page of the samples from start up to end, if there is one.
    fn continuation_query(
        &mut self,
        start: &DateTime,
        end: &DateTime,
        resp: &Self::Response,
    ) -> anyhow::Result<Option<Self::Query>>;
    /// The samples of a response, as they are kept, see Stored.
    fn items(&self, response: Self::Response)
    -> anyhow::Result<impl IntoIterator<Item = anyhow::Result<Self::Stored>>>;
    /// The value of a sample as it is kept.
    fn value(&self, stored: &Self::Stored) -> anyhow::Result<Self::Value>;
    /// What to keep of a value from a DataSet's file in the old layout, which kept values rather than what the server
    /// gave, see Records::import_legacy, None to leave it out.
    fn stored_from_value(&self, _value: serde_json::Value) -> Option<Self::Stored> {
        None
    }
    fn index(&self, value: &Self::Value) -> anyhow::Result<(DateTime, DateTime, bool)>;
    fn refresh_after(&self) -> Duration;
    /// Whether a sample is missing data which a request for a shorter period may provide, e.g. measurements whose
    /// statistics the server leaves out of the whole response if any day in it has a problem. A request for more than
    /// one day which returns such a sample is made again for each day.
    fn is_partial(&self, _value: &Self::Value) -> bool {
        false
    }
}

pub struct Item<V> {
    pub end: DateTime,
    pub is_final: bool,
    pub attributes: DataSetAttributes,
    pub value: V,
    /// The sample as it is kept, see SlotSource::Stored, as JSON.
    pub stored: serde_json::Value,
}

/// A monthly dataset of fixed-interval records that may arrive out of order.
/// Each sample has a start and end DateTime, for contiguous samples A and B A.end == B.start.
pub struct MonthlyFixedSlotDataSet<S: SlotSource>
// where
//     V: DeserializeOwned + Serialize + Clone,
{
    /// The start date of this DataSet
    pub start_date: Date,
    /// The start date of the next monthly bucket, also the exclusive end date of this one.
    pub next_date: Date,
    /// The values indexed by the start DateTime of the sample.
    pub data: BTreeMap<DateTime, Item<<S as SlotSource>::Value>>,
    /// Whether every sample of the month is present and final, so that it is never fetched again, except by a reload.
    pub is_complete: bool,
}

impl<S: SlotSource> MonthlyFixedSlotDataSet<S>
// where
//     V: DeserializeOwned + Serialize + Clone,
{
    fn new(
        start_date: Date,
        next_date: Date,
        tz: &'static Tz,
        data: BTreeMap<DateTime, Item<<S as SlotSource>::Value>>,
    ) -> Self {
        let is_complete = window_complete(&data, &start_date.at_midnight(tz), &next_date.at_midnight(tz));

        Self {
            start_date,
            next_date,
            data,
            is_complete,
        }
    }

    /// The month's samples kept in records, see records.rs, by their starts, a day either side of the month being read,
    /// as the starts are in UTC, and those which start in the month kept.
    fn load(
        source: &S,
        records: &super::records::Records,
        start_date: Date,
        next_date: Date,
        tz: &'static Tz,
    ) -> anyhow::Result<Self> {
        let mut data: BTreeMap<DateTime, Item<<S as SlotSource>::Value>> = BTreeMap::new();
        let (from, to) = (
            (*start_date - time::Duration::days(1)).to_string(),
            (*next_date + time::Duration::days(1)).to_string(),
        );
        for (sk, stored) in records.range(from.as_str()..to.as_str())? {
            let value = serde_json::from_value::<S::Stored>(stored.clone())
                .map_err(anyhow::Error::from)
                .and_then(|kept| source.value(&kept));
            match value {
                Ok(value) => {
                    let (start, end, is_final) = source.index(&value)?;
                    let date = start.to_timezone(tz).to_date();
                    if date < start_date || date >= next_date {
                        continue;
                    }
                    data.insert(
                        start,
                        Item {
                            end,
                            is_final,
                            attributes: DataSetAttributes { cached: true },
                            value,
                            stored,
                        },
                    );
                }
                Err(error) => crate::log_warning!("Unable to read the sample {sk} of {}, {error:#}", source.hash_key()),
            }
        }

        Ok(Self::new(start_date, next_date, tz, data))
    }

    /// The key of each month the window from start to end covers, with the part of the window in that month.
    fn window_months(start: &DateTime, end: &DateTime, tz: &'static Tz) -> Vec<(i32, DateTime, DateTime)> {
        let mut months = Vec::new();
        let mut part_start = *start;
        while part_start < *end {
            let date = part_start.to_date();
            let next_month = Date::from_calendar_date(
                if date.month() == time::Month::December {
                    date.year() + 1
                } else {
                    date.year()
                },
                date.month().next(),
                1,
            )
            .map(|date| date.at_midnight(tz))
            .unwrap_or(*end);
            let part_end = std::cmp::min(next_month, *end);
            months.push((Self::key_for(&date), part_start, part_end));
            part_start = part_end;
        }
        months
    }

    fn key_for(start: &Date) -> i32 {
        let key = start.year() * 12 + start.month() as i32 - 1;
        key
    }

    /// The data from start to end, from the cache, and fetched from the server a whole month at a time for each month
    /// of the period which is due, see OpenMonthlyFixedSlotDataSet::is_due: it is not complete, and was never fetched,
    /// or not within the source's refresh_after, or its last fetch failed. A month of half hours is one request, so a
    /// whole month costs no more than part of one, and looking at part of a month, e.g. zooming in, fetches nothing
    /// once the month is up to date. With reload, the period it gives is fetched again however up to date the cache is,
    /// as a single request even if it crosses a month boundary, and the months it covers are returned too; a reload
    /// does not change when its months count as fetched, as it may cover only part of them.
    pub async fn fetch(
        mut source: S,
        tz: &'static Tz,
        config: &CacheConfig,
        request_manager: &impl GraphQLCaller,
        start: &DateTime,
        end: &DateTime,
        reload: Option<(DateTime, DateTime)>,
        cancel: &CancelToken,
        progress: &(impl ProgressMonitor + ?Sized),
    ) -> anyhow::Result<Vec<MonthlyFixedSlotDataSet<S>>> {
        let mut result_keys = Vec::new();
        let mut result = Vec::new();
        let mut data_set_map = HashMap::new();
        let end = &match reload {
            Some((_, reload_end)) => std::cmp::max(*end, reload_end),
            None => *end,
        };

        {
            // scope this start_date, which includes the months of the period to reload
            let mut start_date = match reload {
                Some((reload_start, _)) => std::cmp::min(*start, reload_start),
                None => *start,
            }
            .to_date();

            progress.progress(String::from("Loading cached data"));

            // end is exclusive so a bucket starting at end is not needed
            while start_date.at_midnight(tz) < *end {
                let open_data_set: OpenMonthlyFixedSlotDataSet<S> =
                    OpenMonthlyFixedSlotDataSet::new(&source, &config, source.hash_key(), &start_date, tz)?;

                let key = Self::key_for(&start_date);
                result_keys.push(key);

                start_date = open_data_set.data_set.next_date.clone();
                data_set_map.insert(key, open_data_set);
            }
        }

        let refresh_after = source.refresh_after();
        // The months fetched whole, whose #fetched records say whether the fetch completed, see flush.
        let mut attempted_keys = HashSet::new();

        // The fetching is in a block of its own so that if it fails, the rows fetched before the failure are still
        // written to the cache below rather than being lost.
        let fetch_result: anyhow::Result<()> = async {
            // An offline profile only has what is cached.
            if config.offline {
                return Ok(());
            }
            // The windows to fetch, as unix timestamps.
            let mut windows = VecDeque::new();
            if let Some((reload_start, reload_end)) = reload {
                // The cached samples are treated as not final so that they are fetched again, as one window even across a
                // month boundary, see reload. They are only replaced if they have changed.
                for ods in data_set_map.values_mut() {
                    for (_, item) in ods.data_set.data.range_mut(reload_start..reload_end) {
                        item.is_final = false;
                    }
                }
                windows.push_back((reload_start.unix_timestamp(), reload_end.unix_timestamp()));
            } else {
                let now = DateTime::now(tz);
                for key in &result_keys {
                    let ods = &data_set_map[key];
                    let month_start = ods.data_set.start_date.at_midnight(tz);
                    if month_start < now && ods.is_due(&refresh_after)? {
                        let month_end = ods.data_set.next_date.at_midnight(tz);
                        windows.push_back((month_start.unix_timestamp(), month_end.unix_timestamp()));
                        attempted_keys.insert(*key);
                    }
                }
            }

            // The number of requests made and of rows they returned, for progress messages.
            let mut requests = 0;
            let mut rows = 0;
            let date_format = time::format_description::parse("[year]-[month]-[day] [hour]:[minute]").unwrap();

            // The number of windows fetched, windows may be added as they are split into days.
            let mut window_number = 0;
            while let Some((window_start, window_end)) = windows.pop_front() {
                if cancel.is_cancelled() {
                    break;
                }
                window_number += 1;
                let window_count = window_number + windows.len();
                let window_start = DateTime::from_unix_timestamp(window_start)?.to_timezone(tz);
                let window_end = DateTime::from_unix_timestamp(window_end)?.to_timezone(tz);
                let prefix = |requests: usize, rows: usize| {
                    format!(
                        "Period {} of {} from {}, request {requests}, {rows} rows loaded",
                        window_number,
                        window_count,
                        window_start.format(&date_format).unwrap_or_default(),
                    )
                };
                // Whether a sample in the window is missing data which a request for each day may provide.
                let mut partial = false;

                requests += 1;
                let query = source.initial_query(&window_start, &window_end)?;
                let mut next_response = call_unless_cancelled(
                    request_manager,
                    &query,
                    cancel,
                    progress,
                    &prefix(requests, rows),
                    &NoRateLimitReset,
                )
                .await?;

                while let Some(response) = next_response {
                    let cq = source.continuation_query(&window_start, &window_end, &response);

                    // Self::handle_records(response, &response_iterator, &indexer, &mut data, &mut file)?;

                    for stored in source.items(response)? {
                        let stored = serde_json::to_value(stored?)?;
                        let value = source.value(&serde_json::from_value(stored.clone())?)?;
                        rows += 1;
                        let (start, end, is_final) = source.index(&value)?;
                        if start >= window_start && start < window_end && source.is_partial(&value) {
                            partial = true;
                        }
                        let start_date = start.to_date();
                        let key = Self::key_for(&start_date);

                        let data_set = if let Some(data_set) = data_set_map.get_mut(&key) {
                            data_set
                        } else {
                            data_set_map.insert(
                                key,
                                OpenMonthlyFixedSlotDataSet::new(&source, &config, source.hash_key(), &start_date, tz)?,
                            );
                            data_set_map.get_mut(&key).unwrap()
                        };

                        // Whether the server gave something different, as it may revise a sample.
                        let update = data_set
                            .data_set
                            .data
                            .get(&start)
                            .is_none_or(|item| item.stored != stored);

                        if !update {
                            // Unchanged, but it may have been treated as not final to reload it.
                            if let Some(item) = data_set.data_set.data.get_mut(&start) {
                                item.is_final = is_final;
                            }
                        } else {
                            data_set.changed.insert(start);
                            data_set.data_set.data.insert(
                                start,
                                Item {
                                    end,
                                    is_final,
                                    attributes: DataSetAttributes { cached: false },
                                    value,
                                    stored,
                                },
                            );
                        }
                    }

                    // A source whose query has no end keeps returning samples after the window, in no useful order,
                    // so stop once the window is complete rather than when a sample after the window arrives. Only
                    // a reload crosses a month boundary, so the window is complete when its part in each month is.
                    let complete = Self::window_months(&window_start, &window_end, tz).iter().all(
                        |(key, part_start, part_end)| {
                            data_set_map
                                .get(key)
                                .is_some_and(|data_set| window_complete(&data_set.data_set.data, part_start, part_end))
                        },
                    );
                    if complete {
                        break;
                    }

                    if let Some(continuation_query) = cq? {
                        requests += 1;
                        next_response = call_unless_cancelled(
                            request_manager,
                            &continuation_query,
                            cancel,
                            progress,
                            &prefix(requests, rows),
                            &NoRateLimitReset,
                        )
                        .await?;
                    } else {
                        break;
                    }
                }

                // The window was fetched, unless it was cancelled part way through.
                if !cancel.is_cancelled() {
                    for (key, _, _) in Self::window_months(&window_start, &window_end, tz) {
                        if let Some(data_set) = data_set_map.get_mut(&key) {
                            data_set.fetched = true;
                        }
                    }

                    // Fetch each day again, next, so that a problem with one day only loses the data for that day.
                    if partial {
                        let days = day_windows(&window_start, &window_end, tz);
                        if days.len() > 1 {
                            for day in days.into_iter().rev() {
                                windows.push_front(day);
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        .await;

        // A month fetched whole is up to date if its fetch completed, and its days fetched again after it if it had
        // samples missing statistics; if the fetch was cancelled or failed, we keep what we have, and the month is due
        // again. Other months, e.g. those only reloaded in part, or another month whose samples a response included,
        // keep their #fetched records as they were.
        let failed = cancel.is_cancelled() || fetch_result.is_err();
        for key in result_keys {
            let ods = data_set_map.remove(&key).unwrap();
            let fetched = attempted_keys.contains(&key).then(|| ods.fetched && !failed);
            result.push(ods.flush(fetched)?);
        }
        for (_key, ods) in data_set_map {
            ods.flush(None)?;
        }

        // Report a failure now that what was fetched has been saved.
        fetch_result?;
        Ok(result)
    }
}

/// A month of samples being read and fetched, see fetch, whose changes are put when it is flushed.
struct OpenMonthlyFixedSlotDataSet<S: SlotSource> {
    records: super::records::Records,
    /// The sort key of the month's #fetched record, which says when it was last fetched, and whether it is stale.
    fetched_key: String,
    /// The starts of the samples new or changed, to put.
    changed: std::collections::BTreeSet<DateTime>,
    /// Whether a window of the month was fetched in full, which for a month fetched whole makes it up to date.
    fetched: bool,
    data_set: MonthlyFixedSlotDataSet<S>,
}

impl<S: SlotSource> OpenMonthlyFixedSlotDataSet<S> {
    /// The month containing date, with the samples kept of it.
    fn new(source: &S, config: &CacheConfig, hash_key: &str, date: &Date, tz: &'static Tz) -> anyhow::Result<Self> {
        let (start, next) = super::month_bounds(date)?;
        let records = super::records::Records::open(config, hash_key)?;
        // An old file's lines are values, kept as the source says, see SlotSource::stored_from_value.
        records.import_legacy(
            config,
            &super::legacy_month_file(hash_key, &start),
            &mut |records, json| {
                let value: S::Value = serde_json::from_value(json.clone())?;
                let (sample_start, _, _) = source.index(&value)?;
                match source.stored_from_value(json) {
                    Some(stored) => records.put(&super::time_key(&sample_start), serde_json::to_value(stored)?),
                    None => Ok(()),
                }
            },
        )?;
        let data_set: MonthlyFixedSlotDataSet<S> = MonthlyFixedSlotDataSet::load(source, &records, start, next, tz)?;
        Ok(Self {
            records,
            fetched_key: format!("#fetched/{}", &start.to_string()[..7]),
            changed: Default::default(),
            fetched: false,
            data_set,
        })
    }

    /// Whether the month is due to be fetched: it is not complete, and it never has been fetched, its last fetch failed,
    /// see flush, or that was longer ago than refresh_after. An empty month, e.g. one before the server has any data,
    /// goes by its #fetched record like any other.
    fn is_due(&self, refresh_after: &Duration) -> anyhow::Result<bool> {
        if self.data_set.is_complete {
            return Ok(false);
        }
        let marked_stale = self
            .records
            .get(&self.fetched_key)?
            .map(|fetched| fetched.get("stale").and_then(|stale| stale.as_bool()) == Some(true));
        Ok(match marked_stale {
            None | Some(true) => true,
            Some(false) => self
                .records
                .fetched_at(&self.fetched_key)?
                .is_some_and(|at| time::OffsetDateTime::now_utc() - at > *refresh_after),
        })
    }

    /// Puts the samples which changed, and, for a month fetched whole, its #fetched record: up to date, Some(true), if
    /// the fetch completed, even if nothing changed, e.g. a month before the server has any data, so that it is not
    /// fetched again until refresh_after has passed, or stale, Some(false), so that it is fetched again next time.
    fn flush(self, fetched: Option<bool>) -> anyhow::Result<MonthlyFixedSlotDataSet<S>> {
        for start in &self.changed {
            if let Some(item) = self.data_set.data.get(start) {
                self.records.put(&super::time_key(start), item.stored.clone())?;
            }
        }
        match fetched {
            Some(true) => self.records.put(&self.fetched_key, serde_json::json!({}))?,
            Some(false) => self
                .records
                .put(&self.fetched_key, serde_json::json!({ "stale": true }))?,
            None => {}
        }
        Ok(self.data_set)
    }
}
#[cfg(test)]
mod window_complete_tests {
    use std::collections::BTreeMap;

    use bruce_graphql::types::DateTime;

    use super::{Item, window_complete};
    use crate::data_set::DataSetAttributes;

    const SLOT: i64 = 1800;

    fn time(slot: i64) -> DateTime {
        DateTime::from_unix_timestamp(slot * SLOT).unwrap()
    }

    /// Samples for the given slots, final unless listed in not_final.
    fn samples(slots: &[i64], not_final: &[i64]) -> BTreeMap<DateTime, Item<()>> {
        slots
            .iter()
            .map(|slot| {
                (
                    time(*slot),
                    Item {
                        end: time(slot + 1),
                        is_final: !not_final.contains(slot),
                        attributes: DataSetAttributes { cached: false },
                        value: (),
                        stored: serde_json::Value::Null,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_window_is_complete_only_when_every_slot_is_present_and_final() {
        let all: Vec<i64> = (0..10).collect();
        assert!(window_complete(&samples(&all, &[]), &time(2), &time(8)));

        // Samples outside the window do not matter, even if they arrived first.
        assert!(window_complete(
            &samples(&[2, 3, 4, 5, 6, 7, 40, 41], &[]),
            &time(2),
            &time(8)
        ));

        assert!(!window_complete(&samples(&[2, 3, 5, 6, 7], &[]), &time(2), &time(8)));
        assert!(!window_complete(&samples(&[3, 4, 5, 6, 7], &[]), &time(2), &time(8)));
        assert!(!window_complete(&samples(&[2, 3, 4, 5, 6], &[]), &time(2), &time(8)));
        assert!(!window_complete(&samples(&all, &[4]), &time(2), &time(8)));
    }
}

#[cfg(test)]
mod retry_tests {
    use super::{brief, is_transient, rate_in_words};

    #[test]
    fn network_errors_gateway_timeouts_and_rate_limits_are_retried_but_not_other_errors() {
        use bruce_graphql::Error;

        // bruce_graphql uses a different version of reqwest, so its status codes are made from numbers.
        let http = |code: u16| Error::HttpError(code.try_into().unwrap(), String::new());
        assert!(is_transient(&http(504)));
        assert!(is_transient(&http(502)));
        assert!(is_transient(&http(429)));
        assert!(!is_transient(&http(400)));
        assert!(!is_transient(&http(401)));
        assert!(!is_transient(&Error::InternalError(String::from("bad"))));
        let graphql = |code: &str| {
            Error::GraphQLError(std::sync::Arc::new(vec![
                serde_json::from_value(
                    serde_json::json!({"message": "Too many requests.", "extensions": {"errorCode": code}}),
                )
                .unwrap(),
            ]))
        };
        assert!(is_transient(&graphql("KT-CT-1199")));
        assert!(!is_transient(&graphql("KT-GB-4051")));
        assert_eq!(brief(&graphql("KT-CT-1199")), "Too many requests");
        assert_eq!(rate_in_words("125/h"), "125 requests an hour");
        assert_eq!(rate_in_words("odd"), "odd");
    }
}

#[cfg(test)]
mod day_windows_tests {
    use bruce_graphql::types::DateTime;
    use time_tz::timezones;

    use super::day_windows;

    #[test]
    fn a_window_is_split_at_each_local_midnight_including_across_a_clock_change() {
        let tz = timezones::db::europe::LONDON;
        let time = |timestamp: i64| DateTime::from_unix_timestamp(timestamp).unwrap().to_timezone(tz);

        // 12:00 on 28 March 2026 to 06:00 on 30 March, the clocks go forward on 29 March so it has 23 hours.
        let start = 1774699200;
        let midnight_29 = 1774742400;
        let midnight_30 = 1774825200;
        let end = 1774846800;
        assert_eq!(midnight_30 - midnight_29, 23 * 3600);
        assert_eq!(
            day_windows(&time(start), &time(end), tz),
            [(start, midnight_29), (midnight_29, midnight_30), (midnight_30, end)]
        );

        // A window within one day is not split.
        assert_eq!(
            day_windows(&time(midnight_29), &time(midnight_30), tz),
            [(midnight_29, midnight_30)]
        );
    }
}
