//! The myenergi module, for myenergi devices such as the zappi EV charger, see client.rs for the API.
//!
//! For now it only fetches a few days into the cache and shows what it fetched, to see what the data is like.

mod client;
mod digest;
mod electricity;
mod fetch;

use std::sync::Arc;

use async_trait::async_trait;
use clap::Parser;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::cache_browser::CacheBrowser;
use crate::util::SharedProp;
use crate::{CacheConfig, InitRequested, MarcoSparkoContext, Module, ModuleFactory, ModuleRegistration, PageInfo};

use client::MyenergiClient;
use fetch::DaySummary;

const MODULE_ID: &str = "myenergi";

/*
 * It is important that the names of all args here begin with myenergi_
 * as only args beginning with myenergi- on the command line are passed through to the module.
 */
#[derive(Parser, Clone, PartialEq)]
pub struct MyenergiArgs {
    /// The serial number of the myenergi hub
    #[arg(long, env)]
    myenergi_hub_serial: Option<String>,
    /// An API key made for the hub in the myenergi app
    #[arg(long, env)]
    myenergi_api_key: Option<String>,
}

/// Debug output is used in verbose mode, so the API key is hidden.
impl std::fmt::Debug for MyenergiArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MyenergiArgs")
            .field("myenergi_hub_serial", &self.myenergi_hub_serial)
            .field("myenergi_api_key", &redacted(&self.myenergi_api_key))
            .finish()
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub hub_serial: Option<String>,
    pub api_key: Option<String>,
    /// The server with the hub's data, which the director said, see MyenergiClient::host.
    pub host: Option<String>,
}

/// Debug output is used in verbose mode, so the API key is hidden.
impl std::fmt::Debug for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Profile")
            .field("hub_serial", &self.hub_serial)
            .field("api_key", &redacted(&self.api_key))
            .field("host", &self.host)
            .finish()
    }
}

/// A secret for debug output, showing only whether there is one.
fn redacted(secret: &Option<String>) -> Option<&'static str> {
    secret.as_ref().map(|_| sparko_graphql::redact::REDACTED)
}

/// What the pages of the module share.
pub struct MyenergiContext {
    client: MyenergiClient,
    cache_config: Arc<CacheConfig>,
    /// The profile's store, which the statuses and records are kept in, see fetch.rs.
    store: Arc<bruce_db::Store>,
}

pub struct MyenergiModule {
    context: Arc<MyenergiContext>,
    /// The day poll last fetched whole days on, see fetch::poll.
    whole_days_fetched: std::sync::Mutex<Option<time::Date>>,
    /// The statuses the profile's capture devices took, see poll.
    capture: crate::capture_device::CaptureReader,
    profile_name: String,
}

/// How often the status is polled.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

impl MyenergiModule {
    pub fn registration() -> ModuleRegistration {
        ModuleRegistration {
            module_id: MODULE_ID.to_string(),
            constructor: Arc::new(MyenergiModule::constructor),
            convert: Some(fetch::convert_legacy),
        }
    }

    pub fn constructor(context: Arc<MarcoSparkoContext>, json_profile: Option<serde_json::Value>) -> anyhow::Result<Arc<dyn ModuleFactory>> {
        let args = MyenergiArgs::parse_from(context.args.module_args(MODULE_ID).unwrap_or_default());
        let mut profile: Profile = match json_profile {
            Some(json) => serde_json::from_value(json)?,
            None => Profile::default(),
        };
        // The args are used instead of the profile, but not saved in it.
        if args.myenergi_hub_serial.is_some() {
            profile.hub_serial = args.myenergi_hub_serial.clone();
        }
        if args.myenergi_api_key.is_some() {
            profile.api_key = args.myenergi_api_key.clone();
        }
        let verbose = context.args.marco_sparko_args.verbose;
        if verbose {
            println!("myenergi Module Args: {:?}", args);
            println!("myenergi Module Profile: {:?}", profile);
        }
        let cache_config = context.create_cache_config(MODULE_ID)?;
        Ok(Arc::new(MyenergiModuleFactory { context, profile, cache_config, verbose }))
    }
}

#[async_trait(?Send)]
impl Module for MyenergiModule {
    async fn fetch_latest_data(&self) -> anyhow::Result<()> {
        fetch::fetch(&self.context.client, &self.context.store, self.context.cache_config.offline, &mut |message| println!("{message}")).await
    }

    fn poll_interval(&self) -> Option<std::time::Duration> {
        Some(POLL_INTERVAL)
    }

    fn next_poll(&self, now: time::OffsetDateTime) -> Option<time::OffsetDateTime> {
        self.capture.next_read(now, POLL_INTERVAL)
    }

    /// If any of the profile's capture devices polls the status, reads the statuses they took instead, so that the app
    /// does not poll it too, but still fetches the minute records, which myenergi keeps.
    async fn poll(&self, slot: Option<time::OffsetDateTime>) -> anyhow::Result<String> {
        let (store, offline) = (&self.context.store, self.context.cache_config.offline);
        let devices = crate::capture_device::adopted_devices(&self.profile_name);
        let captured = !offline && self.capture.read(&devices, fetch::latest_status_time(store),
            &mut |device, record| fetch::save_status(store, record, Some(&device.id)), &mut |_| {}).await?;
        fetch::poll(&self.context.client, store, offline, slot, &self.whole_days_fetched, captured).await
    }

    /// What the zappis charged the car with, and their grid clamps, see electricity.rs.
    fn measurement_source(&self) -> Option<Arc<dyn crate::power::measured::MeasurementSource>> {
        Some(Arc::new(electricity::ZappiMeasurementSource { store: self.context.store.clone() }))
    }

    fn channels(&self) -> Vec<crate::power::topology::Channel> {
        electricity::channels(&self.context.store)
    }

    fn cli_debug(&self) -> anyhow::Result<()> {
        println!("myenergi Module: host={}", self.context.client.host());
        Ok(())
    }

    fn module_id(&self) -> &'static str {
        MODULE_ID
    }

    fn get_page_list(&self) -> Vec<PageInfo> {
        vec![
            PageInfo { label: "Days", path: "days" },
            PageInfo { label: "Samples", path: "samples" },
            PageInfo { label: "Cache Files", path: "cache" },
        ]
    }

    fn help_topic(&self, _page_id: &str, _path: &[String]) -> String {
        format!("{MODULE_ID}/index.md")
    }

    fn get_component(&self, page_id: &str, _path: Vec<String>) -> Element {
        let context = SharedProp(self.context.clone());
        match page_id {
            "days" => rsx! { DaysPage { context } },
            "samples" => rsx! { SamplesPage { context } },
            "cache" => rsx! {
                h1 { "Cache Files" }
                CacheBrowser { dir: self.context.cache_config.dir_path().to_path_buf(), generation: 0 }
            },
            _ => rsx! { div { "Unknown page_id, {page_id}" } },
        }
    }
}

/// The totals of each cached day, and a button to fetch the last few days.
#[component]
fn DaysPage(context: SharedProp<MyenergiContext>) -> Element {
    let mut generation = use_signal(|| 0u32);
    let mut messages = use_signal(Vec::<String>::new);
    let mut busy = use_signal(|| false);
    // Read so that a fetch, which changes it, shows the new days.
    generation();
    let summaries = fetch::summaries(&context.0.store);
    let ct_names = fetch::ct_names(&context.0.store);
    let mut date = use_signal(String::new);
    let today = crate::clock::now_utc().date().to_string();
    let fetch_date = {
        let context = context.clone();
        move |_| {
            let context = context.clone();
            spawn(async move {
                messages.set(Vec::new());
                let format = time::macros::format_description!("[year]-[month]-[day]");
                let Ok(day) = time::Date::parse(&date(), &format) else {
                    messages.write().push(format!("Choose a day to fetch, the date field says {:?}", date()));
                    return;
                };
                busy.set(true);
                if let Err(error) = fetch::fetch_date(&context.0.client, &context.0.store, context.0.cache_config.offline, day, &mut |message| messages.write().push(message)).await {
                    messages.write().push(format!("Failed: {error:#}"));
                }
                busy.set(false);
                generation += 1;
            });
        }
    };
    let fetch = move |_| {
        let context = context.clone();
        spawn(async move {
            busy.set(true);
            messages.set(Vec::new());
            if let Err(error) = fetch::fetch(&context.0.client, &context.0.store, context.0.cache_config.offline, &mut |message| messages.write().push(message)).await {
                messages.write().push(format!("Failed: {error:#}"));
            }
            busy.set(false);
            generation += 1;
        });
    };

    rsx! {
        h1 { "myenergi Days" }
        p {
            "Each device's records of a UTC day, in kWh, and of each of its CT clamps, positive and negative. Missing is how many minutes so far myenergi has no record of, whose energy is not in the totals, and Extra how many records were left out as they are of the next day or a minute which already has one. Diverted is what the device used of surplus generation, and Boosted what it used from the grid. Fetch gets the status and the last {fetch::DAYS} days of each device, days which have ended are only fetched once. Fetch Day gets the day chosen of each device, again even if it was fetched before, to see how far back myenergi has records."
        }
        div { style: "margin-bottom: 1em;",
            button { disabled: busy(), onclick: fetch, if busy() { "Fetching..." } else { "Fetch" } }
            " or one day: "
            input {
                r#type: "date",
                max: "{today}",
                value: "{date}",
                onchange: move |e| date.set(e.value()),
            }
            button { disabled: busy(), onclick: fetch_date, "Fetch Day" }
        }
        for message in messages.read().iter() {
            p { "{message}" }
        }
        table { class: "display",
            tr {
                th { "Date" }
                th { "Device" }
                th { "Minutes" }
                th { "Missing" }
                th { "Extra" }
                th { "Imported kWh" }
                th { "Exported kWh" }
                th { "Generated kWh" }
                th { "Diverted kWh" }
                th { "Boosted kWh" }
                for name in ct_names {
                    th { "{name} + kWh" }
                    th { "{name} − kWh" }
                }
            }
            for summary in summaries {
                DayRow { summary }
            }
        }
    }
}

/// How many samples the Samples page shows.
const SAMPLES_SHOWN: usize = 200;

/// The zappi's latest statuses, one a row, newest first.
#[component]
fn SamplesPage(context: SharedProp<MyenergiContext>) -> Element {
    let samples = fetch::status_samples(&context.0.store, SAMPLES_SHOWN);
    let ct_names = fetch::ct_names(&context.0.store);
    rsx! {
        h1 { "Samples" }
        p {
            "The zappi's latest {SAMPLES_SHOWN} statuses, newest first, at the time each poll was scheduled for, each followed by the zappi's own record of the minute it was in, as the average power over the minute, if that day has been fetched. Power is in kW. Charge Added is the kWh added since the car was plugged in, which is kept until it is plugged in again. Source is what took the status: the app, or a capture device."
        }
        table { class: "display",
            tr {
                th { "Time" }
                th { "zappi" }
                th { "Plug" }
                th { "Status" }
                th { "Mode" }
                th { "Charge Added kWh" }
                th { "Charging kW" }
                th { "Grid kW" }
                th { "Generation kW" }
                for name in ct_names {
                    th { "{name} kW" }
                }
                th { "Voltage V" }
                th { "Frequency Hz" }
                th { "Source" }
            }
            for sample in samples {
                StatusRow { sample: sample.clone() }
                if let Some(minute) = sample.minute {
                    MinuteRow { minute }
                }
            }
        }
    }
}

#[component]
fn StatusRow(sample: fetch::StatusSample) -> Element {
    let number = |value: Option<f64>, decimals: usize| value.map(|value| format!("{value:.decimals$}")).unwrap_or_default();
    rsx! {
        tr {
            td { {sample.time.map(crate::util::local_date_time).unwrap_or_default()} }
            td { "{sample.serial}" }
            td { "{sample.plug}" }
            td { "{sample.status}" }
            td { "{sample.mode}" }
            td { class: "number", {number(sample.charge_added, 2)} }
            td { class: "number", {kilowatts(sample.charging)} }
            td { class: "number", {kilowatts(sample.grid)} }
            td { class: "number", {kilowatts(sample.generation)} }
            for ct in sample.cts {
                td { class: "number", {kilowatts(ct)} }
            }
            td { class: "number", {number(sample.voltage, 1)} }
            td { class: "number", {number(sample.frequency, 2)} }
            td { {sample.captured_by.clone().unwrap_or_else(|| "App".to_string())} }
        }
    }
}

/// Power in W shown in kW, e.g. 7.01
fn kilowatts(watts: Option<f64>) -> String {
    watts.map(|watts| format!("{:.2}", watts / 1000.0)).unwrap_or_default()
}

/// The zappi's record of the minute a status is from, see fetch::MinuteSample.
#[component]
fn MinuteRow(minute: fetch::MinuteSample) -> Element {
    let format = time::macros::format_description!("[hour]:[minute]");
    let optional = |value: Option<f64>, decimals: usize| value.map(|value| format!("{value:.decimals$}")).unwrap_or_default();
    rsx! {
        tr { class: "minute-row",
            td { {format!("{} UTC minute", minute.time.format(&format).unwrap_or_default())} }
            td { colspan: "5", "average over the minute" }
            td { class: "number", {kilowatts(Some(minute.charging))} }
            td { class: "number", {kilowatts(Some(minute.grid))} }
            td { class: "number", {kilowatts(Some(minute.generation))} }
            for ct in minute.cts {
                td { class: "number", {kilowatts(Some(ct))} }
            }
            td { class: "number", {optional(minute.voltage, 1)} }
            td { class: "number", {optional(minute.frequency, 2)} }
        }
    }
}

#[component]
fn DayRow(summary: DaySummary) -> Element {
    rsx! {
        tr {
            td { "{summary.date}" }
            td { "{summary.device}" }
            td { class: "number", "{summary.minutes}" }
            td { class: "number", "{summary.missing}" }
            td { class: "number", "{summary.extra}" }
            td { class: "number", {format!("{:.3}", summary.imported)} }
            td { class: "number", {format!("{:.3}", summary.exported)} }
            td { class: "number", {format!("{:.3}", summary.generated)} }
            td { class: "number", {format!("{:.3}", summary.diverted)} }
            td { class: "number", {format!("{:.3}", summary.boosted)} }
            for ct in summary.cts {
                td { class: "number", {format!("{:.3}", ct.positive)} }
                td { class: "number", {format!("{:.3}", ct.negative)} }
            }
        }
    }
}

pub struct MyenergiModuleFactory {
    context: Arc<MarcoSparkoContext>,
    profile: Profile,
    cache_config: Arc<CacheConfig>,
    verbose: bool,
}

impl MyenergiModuleFactory {
    /// Checks the hub serial and API key by fetching the status, which also finds the server with the hub's data, and
    /// saves them in the profile.
    async fn login(context: Arc<MarcoSparkoContext>, hub_serial: String, api_key: String, verbose: bool) -> anyhow::Result<()> {
        let client = MyenergiClient::new(hub_serial.clone(), api_key.clone(), None, verbose)?;
        client.status().await?;
        let profile = Profile { hub_serial: Some(hub_serial), api_key: Some(api_key), host: Some(client.host()) };
        crate::profile::update_profile(&context.profile.active_profile.name, MODULE_ID, &profile)?;
        crate::log_info!("Logged in to myenergi");
        Ok(())
    }
}

#[async_trait]
impl ModuleFactory for MyenergiModuleFactory {
    async fn is_ready(&self) -> anyhow::Result<bool> {
        // An offline profile has no login, it only shows its cached data, see Profile::offline.
        Ok(self.context.profile.active_profile.offline || (self.profile.hub_serial.is_some() && self.profile.api_key.is_some()))
    }

    fn init_page(&self) -> Element {
        let mut hub_serial = use_signal(|| self.profile.hub_serial.clone().unwrap_or_default());
        let mut api_key = use_signal(String::new);
        let mut errors = use_signal(Vec::<String>::new);
        let mut busy = use_signal(|| false);
        let context = self.context.clone();
        let verbose = self.verbose;

        rsx! {
            for error in errors.read().iter() {
                div { class: "error", "{error}" }
            }
            div {
                h1 { "myenergi Login" }
                form {
                    onsubmit: move |evt: FormEvent| {
                        evt.prevent_default();
                        let context = context.clone();
                        async move {
                            let hub_serial = hub_serial().trim().to_string();
                            let api_key = api_key().trim().to_string();
                            let mut found = Vec::new();
                            if hub_serial.is_empty() {
                                found.push("The hub serial number is required".to_string());
                            }
                            if api_key.is_empty() {
                                found.push("The API key is required".to_string());
                            }
                            if found.is_empty() {
                                busy.set(true);
                                match Self::login(context, hub_serial, api_key, verbose).await {
                                    Ok(()) => {
                                        if let Some(mut init) = try_consume_context::<Signal<InitRequested>>() {
                                            init.set(InitRequested(true));
                                        }
                                    },
                                    Err(error) => {
                                        crate::log_error!("Login to myenergi failed, {error:#}");
                                        found.push(format!("Login failed, {error:#}"));
                                    },
                                }
                                busy.set(false);
                            }
                            errors.set(found);
                        }
                    },
                    table {
                        tr {
                            td {
                                label { "Hub Serial Number:" }
                            }
                            td {
                                input {
                                    r#type: "text",
                                    name: "hub_serial",
                                    value: "{hub_serial}",
                                    oninput: move |e| hub_serial.set(e.value()),
                                }
                            }
                        }
                        tr {
                            td {
                                label { "API Key:" }
                            }
                            td {
                                input {
                                    r#type: "password",
                                    name: "api_key",
                                    value: "{api_key}",
                                    oninput: move |e| api_key.set(e.value()),
                                }
                            }
                        }
                        tr {
                            td {
                                button { r#type: "submit", disabled: busy(), "Log In" }
                            }
                        }
                    }
                }
            }
        }
    }

    async fn build(&self) -> anyhow::Result<Box<dyn Module + Send>> {
        let client = MyenergiClient::new(
            self.profile.hub_serial.clone().unwrap_or_default(),
            self.profile.api_key.clone().unwrap_or_default(),
            self.profile.host.clone(),
            self.verbose,
        )?;
        Ok(Box::new(MyenergiModule {
            context: Arc::new(MyenergiContext { client, cache_config: self.cache_config.clone(), store: self.context.store()? }),
            whole_days_fetched: std::sync::Mutex::new(None),
            capture: crate::capture_device::CaptureReader::new("capture/myenergi/records"),
            profile_name: self.context.profile.active_profile.name.clone(),
        }))
    }
}
