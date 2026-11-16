//! The GloBird module, for GloBird Energy's customers in Australia, from its customer portal, see client.rs.
//!
//! It fetches the smart meter's 5 minute readings and each day's costs into the cache, and shows the readings on its
//! Meters page and Home's Electricity page, see power.rs, and each day's totals and costs, and its bills, beside the
//! days' costs, see bills.rs.

mod bill_pages;
mod bills;
mod client;
mod electricity;
mod fetch;
mod rates;

pub use fetch::LABELS;

use std::sync::Arc;

use async_trait::async_trait;
use clap::Parser;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::cache_browser::CacheBrowser;
use crate::util::SharedProp;
use crate::{AppContext, CacheConfig, InitRequested, Module, ModuleFactory, ModuleRegistration, PageInfo};

use client::GlobirdClient;
use fetch::DaySummary;

const MODULE_ID: &str = "globird";

/*
 * It is important that the names of all args here begin with globird_
 * as only args beginning with globird- on the command line are passed through to the module.
 */
#[derive(Parser, Clone, PartialEq)]
pub struct GlobirdArgs {
    /// The email address of the GloBird portal login
    #[arg(long, env)]
    globird_email: Option<String>,
    /// The password of the GloBird portal login
    #[arg(long, env)]
    globird_password: Option<String>,
}

/// Debug output is used in verbose mode, so the password is hidden.
impl std::fmt::Debug for GlobirdArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlobirdArgs")
            .field("globird_email", &self.globird_email)
            .field("globird_password", &redacted(&self.globird_password))
            .finish()
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub email: Option<String>,
    /// Kept so that the app can log in again when the session expires.
    pub password: Option<String>,
}

/// Debug output is used in verbose mode, so the password is hidden.
impl std::fmt::Debug for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Profile")
            .field("email", &self.email)
            .field("password", &redacted(&self.password))
            .finish()
    }
}

/// The session's cookies, in the module's cache file, see AppContext::update_cache, so that the app does not
/// log in each time it starts, see GlobirdClient::session_token. The field's name has token in it so that it is hidden
/// in verbose output, see bruce_graphql::redact.
#[derive(Serialize, Deserialize)]
struct StoredSession {
    session_token: String,
}

/// A secret for debug output, showing only whether there is one.
fn redacted(secret: &Option<String>) -> Option<&'static str> {
    secret.as_ref().map(|_| bruce_graphql::redact::REDACTED)
}

/// How long GloBird's readings and costs are used before they are fetched again, when a page which shows them opens, see
/// GlobirdContext::fetch_if_due.
const FRESH_FOR: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// What the pages of the module share.
pub struct GlobirdContext {
    client: GlobirdClient,
    cache_config: Arc<CacheConfig>,
    /// To save the session when it changes, see save_session.
    context: Arc<AppContext>,
    /// The bills read from their PDFs, by document ID, or why they could not be, see bill, kept as reading one takes a
    /// while and a bill never changes.
    bills: std::sync::Mutex<std::collections::BTreeMap<i64, Result<bills::Bill, String>>>,
}

impl GlobirdContext {
    /// Fetches the latest data, see fetch::fetch, and saves the session if it changed, e.g. after a login.
    async fn fetch(&self, progress: &mut dyn FnMut(String)) -> anyhow::Result<()> {
        let result = fetch::fetch(&self.client, &self.cache_config, progress).await;
        self.save_session()?;
        result.map_err(|error| self.handle_error(error))
    }

    /// Whether the readings and costs are due to be fetched: never fetched, or not for FRESH_FOR, see fetch_if_due.
    fn fetch_due(&self) -> bool {
        !self.cache_config.offline
            && fetch::fetched_at(&self.cache_config)
                .is_none_or(|fetched| time::OffsetDateTime::now_utc() - fetched >= FRESH_FOR)
    }

    /// Fetches the readings and costs if they are due, see fetch_due, when a page which shows them opens, as Octopus's
    /// pages fetch theirs, so that GloBird is asked once a day at most, however often they are opened. Says what it did
    /// with progress.
    async fn fetch_if_due(&self, progress: &mut dyn FnMut(String)) -> anyhow::Result<bool> {
        if !self.fetch_due() {
            return Ok(false);
        }
        self.fetch(progress).await?;
        Ok(true)
    }

    /// Brings the list of bills up to date, see bills::refresh, returning how many requests it made, and saves the
    /// session if it changed.
    async fn refresh_bills(&self) -> anyhow::Result<usize> {
        let result = bills::refresh(&self.client, &self.cache_config).await;
        self.save_session()?;
        result.map_err(|error| self.handle_error(error))
    }

    /// Fetches the PDF of the bill emailed in email_id, see bills::fetch_pdf, and saves the session if it changed.
    async fn fetch_bill_pdf(&self, email_id: i64) -> anyhow::Result<()> {
        let result = bills::fetch_pdf(&self.client, &self.cache_config, email_id).await;
        self.save_session()?;
        result.map_err(|error| self.handle_error(error))
    }

    /// Reports the saved login if GloBird refused it, e.g. as the password has been changed, so that GloBird's page asks
    /// for the login again, see login.rs. The password is kept, so that the page can show what it was.
    fn handle_error(&self, error: anyhow::Error) -> anyhow::Error {
        if error.chain().any(|cause| cause.is::<client::LoginRefused>()) {
            return crate::login::refused(&self.context, MODULE_ID, format!("{error:#}"));
        }
        error
    }

    /// The cached bill of a document, read from its PDF the first time it is asked for, see bills::read_bill.
    fn bill(&self, document_id: i64) -> Result<bills::Bill, String> {
        if let Some(bill) = self.bills.lock().unwrap().get(&document_id) {
            return bill.clone();
        }
        let bill = bills::document(&self.cache_config, document_id)
            .and_then(|pdf| bills::read_bill(&pdf))
            .map_err(|error| format!("{error:#}"));
        self.bills.lock().unwrap().insert(document_id, bill.clone());
        bill
    }

    fn save_session(&self) -> anyhow::Result<()> {
        if let Some(session_token) = self.client.take_new_session() {
            self.context.update_cache(MODULE_ID, &StoredSession { session_token })?;
        }
        Ok(())
    }
}

pub struct GlobirdModule {
    context: Arc<GlobirdContext>,
}

impl GlobirdModule {
    pub fn registration() -> ModuleRegistration {
        ModuleRegistration {
            module_id: MODULE_ID.to_string(),
            constructor: Arc::new(GlobirdModule::constructor),
            convert: Some(fetch::convert_legacy),
        }
    }

    pub fn constructor(
        context: Arc<AppContext>,
        json_profile: Option<serde_json::Value>,
    ) -> anyhow::Result<Arc<dyn ModuleFactory>> {
        let args = GlobirdArgs::parse_from(context.args.module_args(MODULE_ID).unwrap_or_default());
        let mut profile: Profile = match json_profile {
            Some(json) => serde_json::from_value(json)?,
            None => Profile::default(),
        };
        // The args are used instead of the profile, but not saved in it.
        if args.globird_email.is_some() {
            profile.email = args.globird_email.clone();
        }
        if args.globird_password.is_some() {
            profile.password = args.globird_password.clone();
        }
        let verbose = context.args.app_args.verbose;
        if verbose {
            println!("GloBird Module Args: {:?}", args);
            println!("GloBird Module Profile: {:?}", profile);
        }
        let cache_config = context.create_cache_config(MODULE_ID)?;
        Ok(Arc::new(GlobirdModuleFactory {
            context,
            profile,
            cache_config,
            verbose,
        }))
    }
}

#[async_trait(?Send)]
impl Module for GlobirdModule {
    async fn fetch_latest_data(&self) -> anyhow::Result<()> {
        self.context.fetch(&mut |message| println!("{message}")).await
    }

    /// The meter's readings.
    fn electricity_source(&self) -> Option<Arc<dyn crate::power::ElectricitySource>> {
        Some(Arc::new(electricity::GlobirdElectricity {
            context: self.context.clone(),
            register: None,
        }))
    }

    /// The billing meter, its import and export registers.
    fn channels(&self) -> Vec<crate::power::topology::Channel> {
        vec![crate::power::topology::Channel {
            id: String::from("globird:electricity"),
            description: String::from("The electricity meter GloBird bills from"),
            use_: crate::power::topology::Use::Grid,
            phases: None,
        }]
    }

    fn cli_debug(&self) -> anyhow::Result<()> {
        println!("GloBird Module");
        Ok(())
    }

    fn module_id(&self) -> &'static str {
        MODULE_ID
    }

    fn get_page_list(&self) -> Vec<PageInfo> {
        vec![
            PageInfo {
                label: "Meters",
                path: "meters",
            },
            PageInfo {
                label: "Rates",
                path: "rates",
            },
            PageInfo {
                label: "Days",
                path: "days",
            },
            PageInfo {
                label: "Bills",
                path: "bills",
            },
            PageInfo {
                label: "Cache Files",
                path: "cache",
            },
        ]
    }

    fn help_topic(&self, _page_id: &str, _path: &[String]) -> String {
        format!("{MODULE_ID}/index.md")
    }

    fn get_component(&self, page_id: &str, path: Vec<String>) -> Element {
        let context = SharedProp(self.context.clone());
        match page_id {
            "meters" => match path.first() {
                // One register's readings, as Octopus's Meters page shows one meter's.
                Some(register) => {
                    let direction = if register.starts_with('E') { "import" } else { "export" };
                    let source: Arc<dyn crate::power::ElectricitySource> = Arc::new(electricity::GlobirdElectricity {
                        context: self.context.clone(),
                        register: Some(register.clone()),
                    });
                    rsx! {
                        crate::views::electricity::ElectricityExplorer {
                            key: "{register}",
                            title: "Register {register} ({direction})",
                            sources: vec![SharedProp(source)],
                            measurements: Vec::new(),
                        }
                    }
                }
                None => rsx! { MetersPage { context } },
            },
            "rates" => rsx! { RatesPage { context } },
            "days" => rsx! { DaysPage { context } },
            // A bill opens in a window of its own, see bill_pages::BillWindow.
            "bills" => rsx! { bill_pages::BillsPage { context } },
            "cache" => rsx! {
                h1 { "Cache Files" }
                CacheBrowser { dir: self.context.cache_config.dir_path().to_path_buf(), generation: 0 }
            },
            _ => rsx! { div { "Unknown page_id, {page_id}" } },
        }
    }
}

/// Fetches GloBird's readings and costs when the page it is on opens, if they are due, see
/// GlobirdContext::fetch_if_due, saying what it is doing, and bumps generation once it has, so that the page shows them.
#[component]
fn FetchIfDue(context: SharedProp<GlobirdContext>, generation: Signal<u32>) -> Element {
    let mut generation = generation;
    let mut progress = use_signal(|| None::<String>);
    let mut problem = use_signal(|| None::<String>);
    use_future(move || {
        let context = context.clone();
        async move {
            if !context.0.fetch_due() {
                return;
            }
            progress.set(Some(String::from("Fetching from GloBird...")));
            let result = context.0.fetch_if_due(&mut |message| progress.set(Some(message))).await;
            progress.set(None);
            problem.set(result.err().map(|error| format!("{error:#}")));
            generation += 1;
        }
    });
    rsx! {
        if let Some(progress) = progress() {
            p { "{progress}" }
        }
        if let Some(problem) = problem() {
            p { class: "error", crate::components::error_text::ErrorText { error: problem, message: String::from("Unable to fetch from GloBird") } }
        }
    }
}

/// The meters, and the registers of the cached readings, each a link to its readings.
#[component]
fn MetersPage(context: SharedProp<GlobirdContext>) -> Element {
    let generation = use_signal(|| 0u32);
    generation();
    let meters = fetch::meters(&context.0.cache_config);
    let registers = fetch::registers(&context.0.cache_config);
    let tariffs = rates::tariffs(&fetch::summaries(&context.0.cache_config));
    rsx! {
        h1 { "Meters" }
        FetchIfDue { context: context.clone(), generation }
        p { "The meters GloBird lists for the account, and the registers of the smart meter's readings: E1 is what was imported from the grid, and B1 what was exported to it. Click a register to see its readings, by the tariff's charge types." }
        table { class: "display",
            tr {
                th { "Serial Number" }
                th { "Type" }
                th { "Status" }
            }
            for meter in meters {
                tr {
                    td { "{meter.serial}" }
                    td { "{meter.read_type}" }
                    td { "{meter.status}" }
                }
            }
        }
        h2 { "Registers" }
        table { class: "display",
            tr {
                th { "Register" }
                th { "Direction" }
                th { "Charge Types" }
                th { "Days" }
            }
            for register in registers {
                RegisterRow { register }
            }
        }
        if let Some(tariff) = tariffs.last() {
            h2 { "Rates" }
            p { "The latest rates, worked out from the days' costs, as GloBird does not give them; the Rates page has the earlier ones, and how they are worked out." }
            TariffTable { tariff: tariff.clone() }
        }
    }
}

#[component]
fn RegisterRow(register: fetch::Register) -> Element {
    let name = register.name.clone();
    rsx! {
        tr {
            td {
                div {
                    class: "link",
                    onclick: move |_| {
                        let mut path_signal = use_context::<Signal<Vec<String>>>();
                        path_signal.set(vec![String::from("meters"), name.clone()]);
                    },
                    "{register.name}"
                }
            }
            td { if register.is_import() { "import" } else { "export" } }
            td { {register.bands.join(", ")} }
            td { class: "number", "{register.days}" }
        }
    }
}

/// Each cached day's import, export and costs, each checked against the rates worked out from them, see rates.rs.
#[component]
fn DaysPage(context: SharedProp<GlobirdContext>) -> Element {
    let generation = use_signal(|| 0u32);
    // Read so that a fetch, which changes it, shows the new days.
    generation();
    let summaries = fetch::summaries(&context.0.cache_config);
    let latest = fetch::latest_reading(&context.0.cache_config).map(crate::util::local_date_time);
    let bands: Vec<String> = summaries
        .iter()
        .flat_map(|day| day.import.keys().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let categories: Vec<String> = summaries
        .iter()
        .flat_map(|day| day.cost.keys().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let tariffs = rates::tariffs(&summaries);
    let fetched = fetch::fetched_at(&context.0.cache_config).map(crate::util::local_date_time);

    rsx! {
        h1 { "GloBird Days" }
        p {
            "Each day's energy from and to the grid, in kWh, as the smart meter measured it, by the tariff's charge types, and its costs in AUD, by category. Days run from midnight to midnight in AEST, UTC+10, all year, without daylight saving, as GloBird's do, so in summer a day starts at 1am by the clock in Sydney. Readings is how many of the day's 288 five minute import readings there are. GloBird keeps only a few weeks of readings, so the app keeps every day it fetches. The days not cached yet, and the last {fetch::REFETCH_DAYS} days again, as readings arrive a day or so late, are fetched when this page, the Meters page or the Rates page opens, once a day."
        }
        FetchIfDue { context: context.clone(), generation }
        p {
            if let Some(latest) = latest {
                "Readings up to {latest}. "
            }
            if let Some(fetched) = fetched {
                "Fetched from GloBird at {fetched}, and again when a GloBird page opens a day later."
            }
        }
        table { class: "display",
            tr {
                th { "Date" }
                for band in bands.iter() {
                    th { "{band} kWh" }
                }
                th { "Export kWh" }
                th { "Readings" }
                for category in categories.iter() {
                    th { "{category} $" }
                }
                th { "Total $" }
                th { "Check" }
            }
            for summary in summaries {
                DayRow {
                    tariff: summary.date.and_then(|date| rates::tariff_of(&tariffs, date)).cloned(),
                    summary,
                    bands: bands.clone(),
                    categories: categories.clone(),
                }
            }
        }
    }
}

/// The rates worked out from the cached days' costs, see rates.rs, newest first, as GloBird's portal does not give them.
#[component]
fn RatesPage(context: SharedProp<GlobirdContext>) -> Element {
    let generation = use_signal(|| 0u32);
    // Read so that a fetch, which changes it, shows the new rates.
    generation();
    let tariffs = rates::tariffs(&fetch::summaries(&context.0.cache_config));
    rsx! {
        h1 { "GloBird Rates" }
        FetchIfDue { context: context.clone(), generation }
        p {
            "The rates, as GloBird charges them, worked out from each day's costs and the meter's kWh, as the portal does not give them, newest first. Each day's cost is rounded to the cent, so the rate of a charge type little used, e.g. Peak Usage when batteries run the house then, is less exact, and one with too few kWh is not known. A new set of rates starts where they change; the days whose charges do not fit their rates are said in the Days page's Check column."
        }
        if tariffs.is_empty() {
            p { "No days are cached yet to work them out from." }
        }
        for tariff in tariffs.iter().rev() {
            TariffTable { tariff: tariff.clone() }
        }
    }
}

/// The rates of a run of days, in cents a kWh, and the daily charge.
#[component]
fn TariffTable(tariff: rates::Tariff) -> Element {
    let cents = |rate: Option<f64>| {
        rate.map(|rate| format!("{:.2}", rate.abs() * 100.0))
            .unwrap_or_else(|| String::from("not known"))
    };
    rsx! {
        p { "From {tariff.first} to {tariff.last}, worked out from {tariff.days} days:" }
        table { class: "display",
            tr {
                th { "Charge type" }
                th { "c/kWh" }
            }
            for rate in tariff.usage.iter() {
                tr {
                    td { "{rate.charge_type}" }
                    td { class: "number derived", {cents(rate.rate)} }
                }
            }
            for rate in tariff.feed_in.iter() {
                tr {
                    td { "{rate.charge_type}, a credit" }
                    td { class: "number derived", {cents(rate.rate)} }
                }
            }
            if let Some(daily) = tariff.daily {
                tr {
                    td { "Daily charge, $ a day" }
                    td { class: "number derived", {format!("{daily:.2}")} }
                }
            }
        }
    }
}

/// What a day's check says: nothing if it fits its rates, or how far each category is out, and why, if it is known,
/// e.g. GloBird charged for 25.3 kWh, the meter says 30.3.
fn check_text(summary: &DaySummary, tariff: Option<&rates::Tariff>) -> String {
    let Some(tariff) = tariff else { return String::new() };
    rates::check(summary, tariff)
        .iter()
        .map(|check| {
            let why = check
                .kwh
                .map(|(charged, meter)| format!(", charged for {charged:.2} kWh, the meter says {meter:.2}"))
                .unwrap_or_default();
            format!(
                "{} {:+.2} from the rates{why}",
                check.category,
                check.charged - check.expected
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The split of a day's cost of a category, by charge type, at its rates, e.g. Peak Usage 0.23 kWh x 37.31c = $0.09.
fn cost_split(summary: &DaySummary, category: &str, tariff: Option<&rates::Tariff>) -> Option<String> {
    let tariff = tariff?;
    let (kwh, rates) = match category {
        rates::USAGE => (&summary.import, &tariff.usage),
        rates::SOLAR => (&summary.export_by_type, &tariff.feed_in),
        _ => return None,
    };
    let lines: Vec<String> = rates
        .iter()
        .filter_map(|rate| {
            let kwh = *kwh.get(&rate.charge_type)?;
            let cents = rate.rate? * 100.0;
            Some(format!(
                "{} {kwh:.2} kWh x {cents:.2}c = ${:.2}",
                rate.charge_type,
                kwh * cents / 100.0
            ))
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[component]
fn DayRow(summary: DaySummary, bands: Vec<String>, categories: Vec<String>, tariff: Option<rates::Tariff>) -> Element {
    let optional =
        |value: Option<&f64>, decimals: usize| value.map(|value| format!("{value:.decimals$}")).unwrap_or_default();
    let readings = if summary.estimated {
        format!("{} estimated", summary.readings)
    } else {
        summary.readings.to_string()
    };
    let check = check_text(&summary, tariff.as_ref());
    rsx! {
        tr {
            td { {summary.date.map(|date| date.to_string()).unwrap_or_default()} }
            for band in bands.iter() {
                td { class: "number", {optional(summary.import.get(band), 3)} }
            }
            td { class: "number", {format!("{:.3}", summary.export)} }
            td { class: "number derived", "{readings}" }
            for category in categories.iter() {
                td { class: "number", title: cost_split(&summary, category, tariff.as_ref()), {optional(summary.cost.get(category), 2)} }
            }
            td { class: "number derived",
                if summary.cost.is_empty() { "" } else { {format!("{:.2}", summary.total_cost())} }
            }
            td { class: if check.is_empty() { "" } else { "error" }, "{check}" }
        }
    }
}

pub struct GlobirdModuleFactory {
    context: Arc<AppContext>,
    profile: Profile,
    cache_config: Arc<CacheConfig>,
    verbose: bool,
}

impl GlobirdModuleFactory {
    /// Logs in once, which checks the email and password, and saves them in the profile, and the session in the module's
    /// cache file.
    async fn login(context: Arc<AppContext>, email: String, password: String, verbose: bool) -> anyhow::Result<()> {
        let client = GlobirdClient::new(email.clone(), password.clone(), None, verbose)?;
        client.login().await?;
        context.update_cache(
            MODULE_ID,
            &StoredSession {
                session_token: client.session_token(),
            },
        )?;
        let profile = Profile {
            email: Some(email),
            password: Some(password),
        };
        crate::profile::update_profile(&context.profile.active_profile.name, MODULE_ID, &profile)?;
        crate::login::cleared(&context, MODULE_ID);
        Ok(())
    }
}

#[async_trait]
impl ModuleFactory for GlobirdModuleFactory {
    async fn is_ready(&self) -> anyhow::Result<bool> {
        // An offline profile has no login, it only shows its cached data, see Profile::offline.
        Ok(self.context.profile.active_profile.offline
            || (self.profile.email.is_some() && self.profile.password.is_some()))
    }

    fn init_page(&self) -> Element {
        let mut email = use_signal(|| self.profile.email.clone().unwrap_or_default());
        let password = use_signal(|| self.profile.password.clone().unwrap_or_default());
        let mut errors = use_signal(Vec::<String>::new);
        let mut busy = use_signal(|| false);
        let context = self.context.clone();
        let verbose = self.verbose;

        rsx! {
            for error in errors.read().iter() {
                div { class: "error", "{error}" }
            }
            div {
                h1 { "GloBird Login" }
                p { "The email address and password you log in to the GloBird customer portal with. The app logs in once, and only again when GloBird ends the session. If a login fails it does not try again until you log in here." }
                form {
                    onsubmit: move |evt: FormEvent| {
                        evt.prevent_default();
                        let context = context.clone();
                        async move {
                            let email = email().trim().to_string();
                            let password = password();
                            let mut found = Vec::new();
                            if email.is_empty() {
                                found.push("The email address is required".to_string());
                            }
                            if password.is_empty() {
                                found.push("The password is required".to_string());
                            }
                            if found.is_empty() {
                                busy.set(true);
                                match Self::login(context, email, password, verbose).await {
                                    Ok(()) => {
                                        if let Some(mut init) = try_consume_context::<Signal<InitRequested>>() {
                                            init.set(InitRequested(true));
                                        }
                                    },
                                    Err(error) => {
                                        crate::log_error!("Login to GloBird failed, {error:#}");
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
                                crate::components::secret_input::SecretInput { name: "password", value: password }
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
        let client = GlobirdClient::new(
            self.profile.email.clone().unwrap_or_default(),
            self.profile.password.clone().unwrap_or_default(),
            self.context
                .read_cache::<StoredSession>(MODULE_ID)
                .map(|stored| stored.session_token)
                .as_deref(),
            self.verbose,
        )?;
        Ok(Box::new(GlobirdModule {
            context: Arc::new(GlobirdContext {
                client,
                cache_config: self.cache_config.clone(),
                context: self.context.clone(),
                bills: Default::default(),
            }),
        }))
    }
}
