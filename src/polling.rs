//! Polling the modules while the app is open, e.g. snapshots of the Tesla gateway and the zappi's status, whose
//! history nothing else keeps, see Module::poll.
//!
//! The Poller has its own instance of each module of the profile, built when the app starts or the profile changes,
//! and polls each one when it is due, one at a time. What it is doing is in the PollStatuses signal, which the Polling
//! page shows, and every poll is logged.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use dioxus::prelude::*;
use time::OffsetDateTime;

use crate::{AppContext, Module, ModuleRegistrations};

/// After this many failures in a row a module is paused, so that e.g. a device which rejects the login is not asked
/// again and again.
const MAX_FAILURES: u32 = 3;

/// How often the poller checks whether anything is due.
const TICK: Duration = Duration::from_secs(1);

/// How often the poller records that it is still running in its log, see PollLog.
const HEARTBEAT: Duration = Duration::from_secs(60);

/// The running poller, by the id it was given, and its log, to record in the log when the app quits, see record_quit.
static RUNNING: std::sync::Mutex<Option<(u64, PollLog)>> = std::sync::Mutex::new(None);

/// The id the next poller is given, so that one which is replaced does not stop the log of the one replacing it.
static NEXT_POLLER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The poller's own log of a profile: when it started and stopped, a heartbeat every minute, and every poll, so that
/// whether it was running all the time can be seen, e.g. gaps while the computer was asleep. Kept in the profile's
/// store, see store.rs, as the kind `app.poll`, a DataSet for this installation, each record by its time.
#[derive(Clone)]
struct PollLog {
    store: Arc<bruce_db::Store>,
    hk: String,
}

impl PollLog {
    fn new(profile_name: &str) -> anyhow::Result<PollLog> {
        let store = crate::store::for_profile(profile_name)?;
        let hk = store.hash_key("app.poll", &[bruce_db::Part::Name(crate::store::origin())])?;
        Ok(PollLog { store, hk })
    }

    /// Records the record, with the time now, e.g. {"event": "heartbeat"}. Failing to is logged, not an error, as the
    /// polling matters more than its log.
    fn record(&self, mut record: serde_json::Value) {
        let version = crate::store::version_now();
        let time = version.time.to_text();
        if let Some(fields) = record.as_object_mut() {
            fields.insert("time".to_string(), serde_json::Value::String(time.clone()));
        }
        if let Err(error) = self.store.put(&bruce_db::Item::new(&self.hk, &time, version, record)) {
            crate::log_warning!("Unable to write the polling log, {error:#}");
        }
    }
}

/// Records in the running poller's log that the app quit, called when its window closes.
pub fn record_quit() {
    if let Some((_, log)) = RUNNING.lock().unwrap().take() {
        log.record(serde_json::json!({ "event": "stop", "reason": "quit" }));
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PollState {
    /// The module is being built.
    Starting,
    /// The module can not be polled, and why, e.g. it is not logged in or has nothing to poll.
    NotPolled(String),
    /// Waiting until the next poll is due.
    Waiting,
    Polling,
    /// Paused, by the user or after MAX_FAILURES failures in a row.
    Paused,
}

/// What the poller is doing with a module.
#[derive(Clone, Debug, PartialEq)]
pub struct PollStatus {
    pub state: PollState,
    pub interval: Option<Duration>,
    pub last_time: Option<OffsetDateTime>,
    /// The summary of the last poll, or why it failed.
    pub last_result: Option<Result<String, String>>,
    pub next_time: Option<OffsetDateTime>,
    pub polls: u32,
    pub failures: u32,
    /// Failures since the last success.
    pub failures_in_a_row: u32,
    /// Set to poll now, whether it is due or not.
    pub poll_now: bool,
}

impl PollStatus {
    fn new() -> PollStatus {
        PollStatus {
            state: PollState::Starting,
            interval: None,
            last_time: None,
            last_result: None,
            next_time: None,
            polls: 0,
            failures: 0,
            failures_in_a_row: 0,
            poll_now: false,
        }
    }
}

/// The status of each module of the profile, by module id, provided as a context by App.
pub type PollStatuses = Signal<BTreeMap<String, PollStatus>>;

/// The first time after now which is a whole number of intervals since midnight UTC, e.g. 10:05:00 after 10:03:12
/// every 5 minutes, so that every poller's samples are at the same times, which with intervals which divide an hour
/// are on the hour and half hour too, as Octopus's are. A poll at the start, or asked for, does not move them.
pub fn next_slot(now: OffsetDateTime, interval: Duration) -> OffsetDateTime {
    let seconds = (interval.as_secs() as i64).max(1);
    let next = (now.unix_timestamp().div_euclid(seconds) + 1) * seconds;
    OffsetDateTime::from_unix_timestamp(next).unwrap_or(now + interval)
}

/// Builds the module, if it is logged in.
async fn build(
    registrations: &ModuleRegistrations,
    context: &Arc<AppContext>,
    module_id: &str,
) -> anyhow::Result<Option<Box<dyn Module + Send>>> {
    let factory = registrations.module_factory(module_id, context).await?;
    if !factory.is_ready().await? {
        return Ok(None);
    }
    Ok(Some(factory.build().await?))
}

/// Polls the modules of the context's profile while it is shown. Renders nothing. It must be given a key which changes
/// with the context, so that a new profile's modules are polled.
#[component]
pub fn Poller(context: crate::util::SharedProp<AppContext>) -> Element {
    let registrations = use_context::<ModuleRegistrations>();
    let mut statuses = use_context::<PollStatuses>();
    let id = use_hook(|| NEXT_POLLER_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst));

    use_future(move || {
        let context = context.0.clone();
        let registrations = registrations.clone();
        async move {
            statuses.set(BTreeMap::new());
            crate::capture_device::clear_statuses();
            // An offline profile is only its cached data, so there is nothing to poll.
            if context.profile.active_profile.offline {
                return;
            }
            let log = match PollLog::new(&context.profile.active_profile.name) {
                Ok(log) => log,
                Err(error) => {
                    crate::log_error!("Unable to start the polling log, {error:#}");
                    return;
                }
            };
            log.record(serde_json::json!({ "event": "start", "version": env!("CARGO_PKG_VERSION"), "pid": std::process::id() }));
            *RUNNING.lock().unwrap() = Some((id, log.clone()));
            let mut last_heartbeat = std::time::Instant::now();
            let module_ids: Vec<String> = context.profile.active_profile.modules.keys().cloned().collect();
            let mut modules = Vec::new();
            for module_id in module_ids {
                statuses.write().insert(module_id.clone(), PollStatus::new());
                let (state, interval) = match build(&registrations, &context, &module_id).await {
                    Ok(Some(module)) => match module.poll_interval() {
                        Some(interval) => {
                            modules.push((module_id.clone(), module));
                            (PollState::Waiting, Some(interval))
                        }
                        None => (
                            PollState::NotPolled("It has nothing which needs polling".to_string()),
                            None,
                        ),
                    },
                    Ok(None) => (
                        PollState::NotPolled("It is not logged in, open it to log in".to_string()),
                        None,
                    ),
                    Err(error) => (PollState::NotPolled(format!("Unable to start it, {error:#}")), None),
                };
                if let Some(status) = statuses.write().get_mut(&module_id) {
                    status.state = state;
                    status.interval = interval;
                    // None, so that it is polled straight away, and not for a slot.
                    status.next_time = None;
                }
            }

            // The temperature and humidity the profile's capture devices measure, which are not a module's, see
            // climate.rs.
            if !crate::capture_device::adopted_devices(&context.profile.active_profile.name).is_empty() {
                let reader = crate::climate::ClimateReader::new(&context.profile.active_profile.name);
                let mut status = PollStatus::new();
                status.state = PollState::Waiting;
                status.interval = reader.poll_interval();
                statuses.write().insert(crate::climate::MODULE_ID.to_string(), status);
                modules.push((
                    crate::climate::MODULE_ID.to_string(),
                    Box::new(reader) as Box<dyn Module + Send>,
                ));
            }

            // The polls under way, each finishing on its own, so that a Poll Now of another module, or its next slot,
            // does not wait for them, e.g. for a capture device which does not answer, see CaptureReader::read_device.
            let mut in_flight = futures::stream::FuturesUnordered::new();
            loop {
                if last_heartbeat.elapsed() >= HEARTBEAT {
                    log.record(serde_json::json!({ "event": "heartbeat" }));
                    last_heartbeat = std::time::Instant::now();
                }
                // The modules which are due, with the slot each is polled for, None if it is not a scheduled poll,
                // started together, so that the readings of different devices are of as nearly the same moment as they
                // can be. Not those already polling, which are not Waiting, and which poll_now refuses.
                let now = crate::clock::now_utc();
                for (module_id, module) in &modules {
                    let mut statuses = statuses.write();
                    let Some(status) = statuses.get_mut(module_id) else {
                        continue;
                    };
                    let scheduled =
                        status.state == PollState::Waiting && status.next_time.is_none_or(|next| next <= now);
                    if !(scheduled || status.poll_now) {
                        continue;
                    }
                    let slot = if status.poll_now { None } else { status.next_time };
                    // A poll asked for while paused leaves it paused.
                    let was_paused = status.state == PollState::Paused;
                    status.state = PollState::Polling;
                    status.poll_now = false;
                    in_flight.push(async move {
                        let requested = crate::clock::now_utc();
                        let result = module.poll(slot).await;
                        (
                            module_id,
                            module,
                            slot,
                            was_paused,
                            result,
                            requested,
                            crate::clock::now_utc(),
                        )
                    });
                }

                let finished = tokio::select! {
                    Some(finished) = futures::StreamExt::next(&mut in_flight), if !in_flight.is_empty() => finished,
                    _ = tokio::time::sleep(TICK) => continue,
                };
                let (module_id, module, slot, was_paused, result, requested, received) = finished;
                let now = crate::clock::now_utc();
                let result = result.map_err(|error| format!("{error:#}"));
                let rfc3339 = |time: OffsetDateTime| {
                    time.format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_default()
                };
                log.record(serde_json::json!({
                    "event": "poll",
                    "module": module_id,
                    "slot": slot.map(rfc3339),
                    "requested": rfc3339(requested),
                    "received": rfc3339(received),
                    "ok": result.is_ok(),
                    "result": match &result { Ok(summary) => summary, Err(error) => error },
                }));
                match &result {
                    Ok(summary) => crate::log_info!("Polled {module_id}: {summary}"),
                    Err(error) => crate::log_error!("Polling {module_id} failed: {error}"),
                }
                if let Some(status) = statuses.write().get_mut(module_id) {
                    status.polls += 1;
                    status.last_time = Some(now);
                    if result.is_ok() {
                        status.failures_in_a_row = 0;
                    } else {
                        status.failures += 1;
                        status.failures_in_a_row += 1;
                    }
                    status.last_result = Some(result);
                    status.next_time = module
                        .next_poll(now)
                        .or_else(|| status.interval.map(|interval| next_slot(now, interval)));
                    status.state = if was_paused {
                        PollState::Paused
                    } else if status.failures_in_a_row >= MAX_FAILURES {
                        crate::log_warning!("Paused polling {module_id} after {MAX_FAILURES} failures in a row");
                        PollState::Paused
                    } else {
                        PollState::Waiting
                    };
                }
            }
        }
    });

    // When a new profile's poller replaces this one, as the app quitting is recorded by record_quit.
    use_drop(move || {
        let mut running = RUNNING.lock().unwrap();
        if running.as_ref().is_some_and(|(running_id, _)| *running_id == id) {
            if let Some((_, log)) = running.take() {
                log.record(serde_json::json!({ "event": "stop", "reason": "replaced, e.g. the profile changed" }));
            }
        }
    });

    rsx! {}
}

/// Pauses or resumes polling the module.
pub fn set_paused(mut statuses: PollStatuses, module_id: &str, paused: bool) {
    if let Some(status) = statuses.write().get_mut(module_id) {
        match (&status.state, paused) {
            (PollState::Waiting, true) => status.state = PollState::Paused,
            (PollState::Paused, false) => {
                status.state = PollState::Waiting;
                status.failures_in_a_row = 0;
            }
            _ => {}
        }
    }
}

/// Polls the module as soon as it can, whether it is due or not, unless it is not polled at all.
pub fn poll_now(mut statuses: PollStatuses, module_id: &str) {
    if let Some(status) = statuses.write().get_mut(module_id) {
        if matches!(status.state, PollState::Waiting | PollState::Paused) {
            status.poll_now = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    #[test]
    fn polls_are_on_the_next_multiple_of_the_interval() {
        let five_minutes = Duration::from_secs(5 * 60);
        assert_eq!(
            next_slot(datetime!(2026-10-28 10:03:12 UTC), five_minutes),
            datetime!(2026-10-28 10:05:00 UTC)
        );
        // Not the same time again when it is exactly on one.
        assert_eq!(
            next_slot(datetime!(2026-10-28 10:05:00 UTC), five_minutes),
            datetime!(2026-10-28 10:10:00 UTC)
        );
        assert_eq!(
            next_slot(datetime!(2026-10-28 23:58:59 UTC), five_minutes),
            datetime!(2026-10-29 00:00:00 UTC)
        );
    }
}
