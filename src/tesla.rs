//! The Tesla module, for Powerwalls, from the gateway's local API on the home network, see local.rs, and later from
//! Tesla's Fleet API.
//!
//! For now it only takes snapshots into the cache and shows the latest, to see what the data is like.

mod electricity;
mod local;
pub mod snapshot;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use clap::Parser;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::cache_browser::CacheBrowser;
use crate::util::SharedProp;
use crate::{CacheConfig, InitRequested, MarcoSparkoContext, Module, ModuleFactory, ModuleRegistration, PageInfo};

use local::{LocalClient, PasswordRejected};
use snapshot::Meter;

const MODULE_ID: &str = "tesla";

/// How often a snapshot is taken while the app is open, see polling.rs.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/*
 * It is important that the names of all args here begin with tesla_
 * as only args beginning with tesla- on the command line are passed through to the module.
 */
#[derive(Parser, Clone, PartialEq, Debug)]
pub struct TeslaArgs {
    /// The address of the Powerwall's gateway on the home network, instead of the one in the profile
    #[arg(long, env)]
    tesla_gateway_host: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// The address of the gateway on the home network.
    pub gateway_host: Option<String>,
    /// The gateway's serial number, to find it again if its address changes, see local::find_gateway.
    pub din: Option<String>,
    /// The email and password of the gateway's customer login, kept so that it can log in again when its token expires.
    pub email: Option<String>,
    pub password: Option<String>,
}

/// Debug output is used in verbose mode, so the password is hidden.
impl std::fmt::Debug for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Profile")
            .field("gateway_host", &self.gateway_host)
            .field("din", &self.din)
            .field("email", &self.email)
            .field("password", &self.password.as_ref().map(|_| sparko_graphql::redact::REDACTED))
            .finish()
    }
}

/// The token the gateway gave, in the module's cache file, see MarcoSparkoContext::update_cache.
#[derive(Serialize, Deserialize)]
struct StoredToken {
    token: String,
}

/// What the pages of the module share.
pub struct TeslaContext {
    client: LocalClient,
    /// The Powerwall samples the profile's capture devices took, see TeslaContext::take_snapshot.
    capture: crate::capture_device::CaptureReader,
    /// Where they, and the app's own, are saved.
    /// The profile's store, which the snapshots are kept in, see snapshot.rs.
    store: Arc<bruce_db::Store>,
    cache_config: Arc<CacheConfig>,
    context: Arc<MarcoSparkoContext>,
    profile: Mutex<Profile>,
}

impl TeslaContext {
    fn save_profile(&self, profile: Profile) -> anyhow::Result<()> {
        crate::profile::update_profile(&self.context.profile.active_profile.name, MODULE_ID, &profile)?;
        *self.profile.lock().unwrap() = profile;
        Ok(())
    }

    /// Reads the module's profile again, as the pages and the poller each build their own module, see polling::build,
    /// so a change saved by one, e.g. a rejected password removed, is not otherwise seen by the other until the app
    /// restarts.
    fn reload_profile(&self) {
        let name = self.context.profile.active_profile.name.clone();
        let profile = crate::profile::fetch_active_profile(&Some(name))
            .ok()
            .and_then(|active| active.active_profile.modules.get(MODULE_ID).cloned())
            .and_then(|json| serde_json::from_value::<Profile>(json).ok());
        if let Some(profile) = profile {
            *self.profile.lock().unwrap() = profile;
        }
    }

    /// Takes a snapshot, finding the gateway again first if it does not answer at its address, and saving the token it
    /// gives. If the password is rejected it is removed from the profile, so that it is not tried again.
    ///
    /// If any of the profile's capture devices polls the gateway, reads the samples they took instead, see capture.rs,
    /// so that the app does not poll it too.
    async fn take_snapshot(&self, slot: Option<time::OffsetDateTime>, progress: &mut dyn FnMut(String)) -> anyhow::Result<()> {
        self.reload_profile();
        let devices = crate::capture_device::adopted_devices(&self.context.profile.active_profile.name);
        // Those after the latest snapshot, the first time, and only into an online profile, see snapshot::take.
        let since = snapshot::latest(&self.store).as_ref().and_then(snapshot::time_of);
        let save = &mut |device: &crate::capture_device::DeviceProfile, record: &serde_json::Value| snapshot::save(&self.store, record, Some(&device.id));
        if !self.cache_config.offline && self.capture.read(&self.store, &devices, since, save, progress).await? {
            return Ok(());
        }
        let din = self.profile.lock().unwrap().din.clone();
        if let Some(din) = din.filter(|_| !self.cache_config.offline) {
            if let Err(error) = self.client.get("status").await {
                let unreachable = error.downcast_ref::<reqwest::Error>().is_some_and(|error| error.is_connect() || error.is_timeout());
                if !unreachable {
                    return Err(self.handle_error(error));
                }
                progress(format!("The gateway does not answer at {}, searching the network for it", self.client.host()));
                let Some((host, _)) = local::find_gateway(Some(&din), Some(&self.client.host())).await? else {
                    return Err(anyhow::anyhow!("The gateway was not found on the network"));
                };
                progress(format!("Found the gateway at {host}"));
                self.client.set_host(host.clone());
                let profile = Profile { gateway_host: Some(host), ..self.profile.lock().unwrap().clone() };
                self.save_profile(profile)?;
            }
        }
        let result = snapshot::take(&self.client, &self.store, self.cache_config.offline, slot, progress).await;
        if let Some(token) = self.client.token() {
            self.context.update_cache(MODULE_ID, &StoredToken { token })?;
        }
        result.map_err(|error| self.handle_error(error))
    }

    /// The error, after removing the password from the profile if it was rejected.
    fn handle_error(&self, error: anyhow::Error) -> anyhow::Error {
        if error.is::<PasswordRejected>() {
            let profile = Profile { password: None, ..self.profile.lock().unwrap().clone() };
            if let Err(save_error) = self.save_profile(profile) {
                crate::log_error!("Unable to remove the rejected Tesla password from the profile, {save_error:#}");
            }
            crate::log_error!("The Tesla gateway rejected the password, log in again");
        }
        error
    }
}

pub struct TeslaModule {
    context: Arc<TeslaContext>,
}

impl TeslaModule {
    pub fn registration() -> ModuleRegistration {
        ModuleRegistration {
            module_id: MODULE_ID.to_string(),
            constructor: Arc::new(TeslaModule::constructor),
            convert: Some(snapshot::convert_legacy),
        }
    }

    pub fn constructor(context: Arc<MarcoSparkoContext>, json_profile: Option<serde_json::Value>) -> anyhow::Result<Arc<dyn ModuleFactory>> {
        let args = TeslaArgs::parse_from(context.args.module_args(MODULE_ID).unwrap_or_default());
        let mut profile: Profile = match json_profile {
            Some(json) => serde_json::from_value(json)?,
            None => Profile::default(),
        };
        // The arg is used instead of the profile, but not saved in it.
        if args.tesla_gateway_host.is_some() {
            profile.gateway_host = args.tesla_gateway_host.clone();
        }
        let verbose = context.args.marco_sparko_args.verbose;
        if verbose {
            println!("Tesla Module Args: {:?}", args);
            println!("Tesla Module Profile: {:?}", profile);
        }
        let cache_config = context.create_cache_config(MODULE_ID)?;
        Ok(Arc::new(TeslaModuleFactory { context, profile, cache_config, verbose }))
    }
}

#[async_trait(?Send)]
impl Module for TeslaModule {
    /// The grid's import and export through the gateway, for a profile with no supplier's.
    fn electricity_source(&self) -> Option<Arc<dyn crate::power::ElectricitySource>> {
        Some(Arc::new(electricity::TeslaElectricity { store: self.context.store.clone() }))
    }

    /// What the gateway measured of the site's use, which splits a supplier's import by use.
    fn measurement_source(&self) -> Option<Arc<dyn crate::power::measured::MeasurementSource>> {
        Some(Arc::new(electricity::TeslaMeasurements { store: self.context.store.clone() }))
    }

    /// The gateway's meters.
    fn channels(&self) -> Vec<crate::power::topology::Channel> {
        use crate::power::measured::{BATTERY_CHANNEL, LOAD_CHANNEL, SITE_CHANNEL, SOLAR_CHANNEL};
        use crate::power::topology::{Channel, Use};
        [
            (SITE_CHANNEL, "The gateway's site meter, its grid CT clamp", Use::Board),
            (BATTERY_CHANNEL, "The Powerwalls, charging and discharging", Use::Battery),
            (SOLAR_CHANNEL, "The solar panels", Use::Solar),
            (LOAD_CHANNEL, "The house, as the gateway works it out from its meters", Use::house()),
        ].map(|(id, description, use_)| Channel { id: id.to_string(), description: description.to_string(), use_ }).to_vec()
    }

    async fn fetch_latest_data(&self) -> anyhow::Result<()> {
        self.context.take_snapshot(None, &mut |message| println!("{message}")).await
    }

    fn poll_interval(&self) -> Option<std::time::Duration> {
        Some(POLL_INTERVAL)
    }

    fn next_poll(&self, now: time::OffsetDateTime) -> Option<time::OffsetDateTime> {
        self.context.capture.next_read(now, POLL_INTERVAL)
    }

    async fn poll(&self, slot: Option<time::OffsetDateTime>) -> anyhow::Result<String> {
        self.context.take_snapshot(slot, &mut |_| {}).await?;
        let latest = snapshot::latest(&self.context.store).ok_or_else(|| anyhow::anyhow!("The snapshot was not saved"))?;
        let mut summary: Vec<String> = snapshot::meters(&latest).iter()
            .map(|meter| format!("{} {:.2} kW", meter.name, meter.power / 1000.0))
            .collect();
        if let Some(charge) = latest.pointer("/responses/system_status~1soe/percentage").and_then(|value| value.as_f64()) {
            summary.push(format!("battery charge {charge:.1}%"));
        }
        Ok(summary.join(", "))
    }

    fn cli_debug(&self) -> anyhow::Result<()> {
        println!("Tesla Module: gateway={}", self.context.client.host());
        Ok(())
    }

    fn module_id(&self) -> &'static str {
        MODULE_ID
    }

    fn get_page_list(&self) -> Vec<PageInfo> {
        vec![
            PageInfo { label: "Powerwall", path: "powerwall" },
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
            "powerwall" => rsx! { PowerwallPage { context } },
            "samples" => rsx! { SamplesPage { context } },
            "cache" => rsx! {
                h1 { "Cache Files" }
                CacheBrowser { dir: self.context.cache_config.dir_path().to_path_buf(), generation: 0 }
            },
            _ => rsx! { div { "Unknown page_id, {page_id}" } },
        }
    }
}

/// The latest snapshot's meters, and a button to take a new one.
#[component]
fn PowerwallPage(context: SharedProp<TeslaContext>) -> Element {
    let mut generation = use_signal(|| 0u32);
    let mut messages = use_signal(Vec::<String>::new);
    let mut busy = use_signal(|| false);
    // Read so that a new snapshot, which changes it, is shown.
    generation();
    let latest = snapshot::latest(&context.0.store);
    let time = latest.as_ref().and_then(snapshot::time_of).map(crate::util::local_date_time).unwrap_or_default();
    let meters = latest.as_ref().map(snapshot::meters).unwrap_or_default();
    let charge = latest.as_ref().and_then(|snapshot| snapshot.pointer("/responses/system_status~1soe/percentage")).and_then(|value| value.as_f64());
    // Which the button reads, if the profile has any, see TeslaContext::take_snapshot.
    let has_devices = !use_context::<Signal<crate::profile::Settings>>().read().capture_devices.is_empty();
    let take = move |_| {
        let context = context.clone();
        spawn(async move {
            busy.set(true);
            messages.set(Vec::new());
            if let Err(error) = context.0.take_snapshot(None, &mut |message| messages.write().push(message)).await {
                messages.write().push(format!("Failed: {error:#}"));
            }
            busy.set(false);
            generation += 1;
        });
    };

    rsx! {
        h1 { "Powerwall" }
        p {
            "What the gateway said at the time of the latest snapshot. Power is in kW, positive when importing from the grid or discharging the battery. Energy is in kWh since the gateway was installed."
        }
        if has_devices {
            button { disabled: busy(), onclick: take, if busy() { "Reading the capture devices..." } else { "Read the Capture Devices" } }
        } else {
            button { disabled: busy(), onclick: take, if busy() { "Taking a snapshot..." } else { "Take a Snapshot" } }
        }
        for message in messages.read().iter() {
            p { "{message}" }
        }
        if latest.is_some() {
            h2 { "{time}" }
            if let Some(charge) = charge {
                p { {format!("Battery charge {charge:.1}%")} }
            }
            table { class: "display",
                tr {
                    th { "Meter" }
                    th { "Power kW" }
                    th { "Imported kWh" }
                    th { "Exported kWh" }
                }
                for meter in meters {
                    MeterRow { meter }
                }
            }
        }
    }
}

/// How many samples the Samples page shows.
const SAMPLES_SHOWN: usize = 200;

/// The latest snapshots, one a row, newest first.
#[component]
fn SamplesPage(context: SharedProp<TeslaContext>) -> Element {
    let samples = snapshot::samples(&context.0.store, SAMPLES_SHOWN);
    rsx! {
        h1 { "Samples" }
        p {
            "The latest {SAMPLES_SHOWN} snapshots, newest first. Power is in kW, positive when importing from the grid or discharging the battery. Energy is in kWh since the gateway was installed. Charge and reserve are as the gateway says, which the Tesla app shows less 5%. Source is what took the snapshot: the app, or a capture device."
        }
        table { class: "display",
            tr {
                th { "Time" }
                th { "Grid kW" }
                th { "Battery kW" }
                th { "Solar kW" }
                th { "Home kW" }
                th { "Charge %" }
                th { "Reserve %" }
                th { "Mode" }
                th { "Grid Status" }
                th { "Grid In kWh" }
                th { "Grid Out kWh" }
                th { "Battery In kWh" }
                th { "Battery Out kWh" }
                th { "Solar kWh" }
                th { "Home kWh" }
                th { "Source" }
            }
            for sample in samples {
                SampleRow { sample }
            }
        }
    }
}

#[component]
fn SampleRow(sample: snapshot::Sample) -> Element {
    let power = |name: &str| sample.meter(name).map(|meter| format!("{:.2}", meter.power / 1000.0)).unwrap_or_default();
    let energy = |name: &str, imported: bool| sample.meter(name)
        .map(|meter| format!("{:.3}", if imported { meter.imported } else { meter.exported }))
        .unwrap_or_default();
    let percent = |value: Option<f64>| value.map(|value| format!("{value:.1}")).unwrap_or_default();
    rsx! {
        tr {
            td { {sample.time.map(crate::util::local_date_time).unwrap_or_default()} }
            td { class: "number", {power("site")} }
            td { class: "number", {power("battery")} }
            td { class: "number", {power("solar")} }
            td { class: "number", {power("load")} }
            td { class: "number", {percent(sample.charge)} }
            td { class: "number", {percent(sample.reserve)} }
            td { {sample.mode.clone().unwrap_or_default()} }
            td { {sample.grid_status.clone().unwrap_or_default()} }
            td { class: "number", {energy("site", true)} }
            td { class: "number", {energy("site", false)} }
            td { class: "number", {energy("battery", true)} }
            td { class: "number", {energy("battery", false)} }
            td { class: "number", {energy("solar", false)} }
            td { class: "number", {energy("load", true)} }
            td { {sample.captured_by.clone().unwrap_or_else(|| "App".to_string())} }
        }
    }
}

#[component]
fn MeterRow(meter: Meter) -> Element {
    rsx! {
        tr {
            td { "{meter.name}" }
            td { class: "number", {format!("{:.2}", meter.power / 1000.0)} }
            td { class: "number", {format!("{:.3}", meter.imported)} }
            td { class: "number", {format!("{:.3}", meter.exported)} }
        }
    }
}

pub struct TeslaModuleFactory {
    context: Arc<MarcoSparkoContext>,
    profile: Profile,
    cache_config: Arc<CacheConfig>,
    verbose: bool,
}

impl TeslaModuleFactory {
    /// Finds the gateway if host is empty, logs in once, and saves the login in the profile and the token in the cache.
    async fn login(context: Arc<MarcoSparkoContext>, profile: Profile, host: String, email: String, password: String,
        verbose: bool, progress: &mut dyn FnMut(String)) -> anyhow::Result<()> {
        let (host, din) = if host.is_empty() {
            progress("Searching the network for the gateway".to_string());
            match local::find_gateway(None, profile.gateway_host.as_deref()).await? {
                Some((host, status)) => (host, status["din"].as_str().map(String::from)),
                None => return Err(anyhow::anyhow!("No gateway was found on the network, enter its address")),
            }
        }
        else {
            (host, profile.din.clone())
        };
        progress(format!("Logging in to the gateway at {host}"));
        let client = LocalClient::new(host.clone(), email.clone(), password.clone(), None, verbose)?;
        client.login().await?;
        // Its serial number, to find it again if its address changes.
        let din = match din {
            Some(din) => Some(din),
            None => client.get("status").await?["din"].as_str().map(String::from),
        };
        let profile = Profile { gateway_host: Some(host), din, email: Some(email), password: Some(password), ..profile };
        crate::profile::update_profile(&context.profile.active_profile.name, MODULE_ID, &profile)?;
        if let Some(token) = client.token() {
            context.update_cache(MODULE_ID, &StoredToken { token })?;
        }
        crate::log_info!("Logged in to the Tesla gateway");
        Ok(())
    }
}

#[async_trait]
impl ModuleFactory for TeslaModuleFactory {
    async fn is_ready(&self) -> anyhow::Result<bool> {
        // An offline profile has no login, it only shows its cached data, see Profile::offline.
        Ok(self.context.profile.active_profile.offline
            || !self.context.profile.active_profile.settings.capture_devices.is_empty()
            || (self.profile.gateway_host.is_some() && self.profile.email.is_some() && self.profile.password.is_some()))
    }

    fn init_page(&self) -> Element {
        let mut host = use_signal(|| self.profile.gateway_host.clone().unwrap_or_default());
        let mut email = use_signal(|| self.profile.email.clone().unwrap_or_default());
        let mut password = use_signal(String::new);
        let mut errors = use_signal(Vec::<String>::new);
        let mut messages = use_signal(Vec::<String>::new);
        let mut busy = use_signal(|| false);
        let context = self.context.clone();
        let profile = self.profile.clone();
        let verbose = self.verbose;
        // Saying so, as it was removed because the gateway rejected it, see TeslaContext::handle_error.
        let password_removed = self.profile.email.is_some() && self.profile.password.is_none();

        rsx! {
            for error in errors.read().iter() {
                div { class: "error", "{error}" }
            }
            div {
                h1 { "Tesla Powerwall Login" }
                if password_removed {
                    p { "The gateway did not accept the saved password. Check it before logging in again, as repeated failures lock out the customer login for a while, for Home Assistant too." }
                }
                form {
                    onsubmit: move |evt: FormEvent| {
                        evt.prevent_default();
                        let context = context.clone();
                        let profile = profile.clone();
                        async move {
                            let host = host().trim().to_string();
                            let email = email().trim().to_string();
                            let password = password().trim().to_string();
                            let mut found = Vec::new();
                            if email.is_empty() {
                                found.push("The email is required".to_string());
                            }
                            if password.is_empty() {
                                found.push("The password is required".to_string());
                            }
                            if found.is_empty() {
                                busy.set(true);
                                messages.set(Vec::new());
                                match Self::login(context, profile, host, email, password, verbose, &mut |message| messages.write().push(message)).await {
                                    Ok(()) => {
                                        if let Some(mut init) = try_consume_context::<Signal<InitRequested>>() {
                                            init.set(InitRequested(true));
                                        }
                                    },
                                    Err(error) => {
                                        crate::log_error!("Login to the Tesla gateway failed, {error:#}");
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
                                label { "Gateway Address:" }
                            }
                            td {
                                input {
                                    r#type: "text",
                                    name: "host",
                                    placeholder: "Leave empty to search the network",
                                    value: "{host}",
                                    oninput: move |e| host.set(e.value()),
                                }
                            }
                        }
                        tr {
                            td {
                                label { "Email:" }
                            }
                            td {
                                input {
                                    r#type: "email",
                                    name: "email",
                                    value: "{email}",
                                    oninput: move |e| email.set(e.value()),
                                }
                            }
                        }
                        tr {
                            td {
                                label { "Password:" }
                            }
                            td {
                                input {
                                    r#type: "password",
                                    name: "password",
                                    value: "{password}",
                                    oninput: move |e| password.set(e.value()),
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
                for message in messages.read().iter() {
                    p { "{message}" }
                }
            }
        }
    }

    async fn build(&self) -> anyhow::Result<Box<dyn Module + Send>> {
        let token = self.context.read_cache::<StoredToken>(MODULE_ID).map(|stored| stored.token);
        let client = LocalClient::new(
            self.profile.gateway_host.clone().unwrap_or_default(),
            self.profile.email.clone().unwrap_or_default(),
            self.profile.password.clone().unwrap_or_default(),
            token,
            self.verbose,
        )?;
        Ok(Box::new(TeslaModule {
            context: Arc::new(TeslaContext {
                client,
                capture: crate::capture_device::CaptureReader::new("capture/tesla/records"),
                store: self.context.store()?,
                cache_config: self.cache_config.clone(),
                context: self.context.clone(),
                profile: Mutex::new(self.profile.clone()),
            }),
        }))
    }
}
