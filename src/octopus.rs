

mod token;
mod decimal;
mod viewer_data_set;
mod property;
mod meter_point;
mod smart_device;
mod charging_session;
mod telemetry;
mod rate_limit;
mod bill;
mod meter;
mod line_items;
mod meter_agreement;
mod measurements;
mod fetch;
mod logging_caller;
mod trend_graph;
mod convert;
mod context;
mod pages;
mod flex_poll;
mod tariff;

pub use tariff::LABELS;

use std::sync::Arc;

use anyhow::anyhow;
use viewer_data_set::ViewerDataSet;
use async_trait::async_trait;

use dioxus::prelude::*;

use context::{OctopusContext, OctopusRoot};
use pages::{AccountRow, BillsPage, EvChargingPage, MeasurementsPage, PropertiesPage};
use serde::{Deserialize, Serialize};

use time_tz::timezones;
use token::{OctopusTokenManager};
use clap::Parser;

use sparko_graphql::TokenManager;
use crate::{CacheConfig, InitRequested, MarcoSparkoContext, Module, ModuleFactory, ModuleRegistration, PageInfo, octopus::token::OctopusAuthenticator};

// include!("octopus/graphql.rs");
include!(concat!(env!("OUT_DIR"), "/graphql.rs"));
include!(concat!(env!("OUT_DIR"), "/crate_info.rs"));

#[cfg(graphql_generation_error)]
compile_error!("graphql built with Errors");


/// Makes the requests of the pages, logging each one, see logging_caller.rs.
pub type RequestManager = logging_caller::LoggingCaller<sparko_graphql::AuthenticatedRequestManager<OctopusTokenManager>>;


/*
 * It is important that the names of all args here begin with octopus_
 * as only args beginning with octopus- on the command line are passed through to the module.
 * 
 * Underscores in the names of fields on this struct are translated into hyphens on the command line automatically by clap.
 */
#[derive(Parser, Clone, PartialEq)]
pub struct OctopusArgs {
    /// The Octopus API_KEY to use
    #[arg(short, long, env)]
    octopus_api_key: Option<String>
}

/// Debug output is used in verbose mode, so the API key is hidden.
impl std::fmt::Debug for OctopusArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OctopusArgs")
            .field("octopus_api_key", &redacted(&self.octopus_api_key))
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub api_key:  Option<String>,
    pub billing_timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_number: Option<String>,
    /// The host name of the API, e.g. api.octopus.energy
    #[serde(default = "default_api_host")]
    pub api_host: String,
    #[serde(skip)]
    // #[serde(default = false)]
    pub init: bool,
}

/// Debug output is used in verbose mode, so the API key is hidden.
impl std::fmt::Debug for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Profile")
            .field("api_key", &redacted(&self.api_key))
            .field("billing_timezone", &self.billing_timezone)
            .field("account_number", &self.account_number)
            .field("api_host", &self.api_host)
            .field("init", &self.init)
            .finish()
    }
}

/// A secret for debug output, showing only whether there is one.
fn redacted(secret: &Option<String>) -> Option<&'static str> {
    secret.as_ref().map(|_| sparko_graphql::redact::REDACTED)
}

/// The API host for a profile which does not have one, i.e. one created before it was added.
fn default_api_host() -> String {
    "api.octopus.energy".to_string()
}

impl Profile {
    pub fn new() -> Profile {
        Profile {
            api_key: None,
            billing_timezone: Some("Europe/London".to_string()),
            account_number: None,
            api_host: "api.oegb-kraken.energy".to_string(),
            init: true,
        }
    }
}

pub struct OctopusModule{
    octopus_args: OctopusArgs,
    account_manager: ViewerDataSet,
    /// The module's settings, which the account page changes the account of, see AccountRow.
    profile: Profile,
    /// Shared with the pages, see OctopusRoot.
    context: Arc<OctopusContext>,
    /// The requests polling has made, see flex_poll.rs.
    call_budget: flex_poll::CallBudget,
    charging_device: flex_poll::ChargingDevice,
    /// The polls the profile's capture devices took, see poll.
    capture: crate::capture_device::CaptureReader,
    profile_name: String,
}

const MODULE_ID: &str = "octopus";


#[derive(Deserialize, Clone)]
struct LoginForm {
    email: Option<String>,
    password: Option<String>,
    api_key: Option<String>,
    login_method: String,
}

/// Debug output is used in verbose mode, so the password and API key are hidden.
impl std::fmt::Debug for LoginForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginForm")
            .field("email", &self.email)
            .field("password", &redacted(&self.password))
            .field("api_key", &redacted(&self.api_key))
            .field("login_method", &self.login_method)
            .finish()
    }
}

impl OctopusModule {
    async fn new(octopus_args: OctopusArgs, cache_config: Arc<CacheConfig>, store: Arc<bruce_db::Store>, profile: Profile,
        request_manager: Arc<RequestManager>, _verbose: bool) -> anyhow::Result<OctopusModule> {   

        let billing_timezone = Self::get_billing_timezone(&profile);
        let account_manager = ViewerDataSet::new(&cache_config, &*request_manager, profile.account_number.as_deref()).await?;

        let context = Arc::new(OctopusContext {
            account_id: account_manager.get_selected_account_id().to_string(),
            billing_timezone,
            cache_config,
            request_manager,
            store,
        });

        Ok(OctopusModule {
            octopus_args,
            account_manager,
            profile,
            context,
            call_budget: flex_poll::CallBudget::default(),
            charging_device: flex_poll::ChargingDevice::default(),
            capture: crate::capture_device::CaptureReader::new("capture/octopus/records"),
            // Set by OctopusModuleFactory::build.
            profile_name: String::new(),
        })
    }

    fn get_billing_timezone(profile: &Profile) -> &'static time_tz::Tz {
        // if let Some(profile) = profile {
            if let Some(name) = &profile.billing_timezone {
                if let Some(tz) =  timezones::get_by_name(&name) {
                    return tz;
                }
                panic!("Unable to load billing_timezone '{}'", name);
            }
        // }
        return timezones::db::europe::LONDON;
    }

    pub fn registration() -> ModuleRegistration {
        ModuleRegistration {
            module_id: MODULE_ID.to_string(),
            constructor: Arc::new(OctopusModule::constructor),
            convert: Some(convert::convert_legacy),
        }
    }
    
    pub fn constructor(context: Arc<MarcoSparkoContext>, 
        json_profile: Option<serde_json::Value>) -> anyhow::Result<Arc<dyn ModuleFactory>> {

            let builder =OctopusModule::builder(context, json_profile)?;

            Ok(Arc::new(builder.build()?))
    }

    pub fn builder(context: Arc<MarcoSparkoContext>, 
        json_profile: Option<serde_json::Value>
    ) -> anyhow::Result<OctopusModuleFactoryBuilder> {

        OctopusModuleFactoryBuilder::new(context, json_profile)
    }
}


#[async_trait(?Send)]
impl Module for OctopusModule {
    async fn fetch_latest_data(&self) -> anyhow::Result<()> {
        fetch::fetch_latest_data(&self.context).await
    }

    fn poll_interval(&self) -> Option<std::time::Duration> {
        Some(flex_poll::POLL_INTERVAL)
    }

    /// If any of the profile's capture devices polls Octopus's smart charging, reads the polls they took instead, so
    /// that the app does not poll it too.
    async fn poll(&self, slot: Option<time::OffsetDateTime>) -> anyhow::Result<String> {
        let context = &self.context;
        let devices = crate::capture_device::adopted_devices(&self.profile_name);
        let since = flex_poll::latest(context).and_then(|(time, _)| time);
        if !context.cache_config.offline
            && self.capture.read(&context.store, &devices, since, &mut |device, record| flex_poll::save(context, record, Some(&device.id)), &mut |_| {}).await? {
            return Ok(flex_poll::latest(context).map(|(_, summary)| summary).unwrap_or_default());
        }
        flex_poll::poll(&self.context, slot, &self.call_budget, &self.charging_device).await
    }

    fn next_poll(&self, now: time::OffsetDateTime) -> Option<time::OffsetDateTime> {
        self.capture.next_read(now, flex_poll::POLL_INTERVAL)
    }

    /// Its meters and Home Mini put together, see pages/electricity.rs.
    fn electricity_source(&self) -> Option<Arc<dyn crate::power::ElectricitySource>> {
        Some(Arc::new(pages::OctopusElectricity { context: self.context.clone() }))
    }

    /// The billing meter, whose half hours and Home Mini readings are one place, see power::topology.
    fn channels(&self) -> Vec<crate::power::topology::Channel> {
        vec![crate::power::topology::Channel {
            id: String::from("octopus:electricity"),
            description: String::from("The electricity meter Octopus bills from, and its Home Mini's readings"),
            use_: crate::power::topology::Use::Grid,
        }]
    }

    fn cli_debug(&self) -> Result<(), anyhow::Error> {
        println!("Args: {:?}", self.octopus_args);
        println!("Octopus Module: account_id={}", self.context.account_id);
        Ok(())
    }

    fn module_id(&self) -> &'static str {
        MODULE_ID
    }

    fn get_page_list(&self) -> Vec<PageInfo> {
        vec!(
            PageInfo {
                label: "User", 
                path: "user",
            },
            PageInfo {
                label: "Account", 
                path: "account",
            },
            PageInfo {
                label: "Bills",
                path: "bills",
            },
            PageInfo {
                label: "Properties",
                path: "properties",
            },
            PageInfo {
                label: "Meters",
                path: "measurements",
            },
            PageInfo {
                label: "EV Charging",
                path: "ev",
            },
        )
    }

    fn help_topic(&self, page_id: &str, path: &[String]) -> String {
        let topic = match (page_id, path.first().map(String::as_str)) {
            ("user", _) => "user",
            ("account", _) => "account",
            ("bills", _) => "bills",
            ("properties", None) => "properties",
            ("properties", Some(_)) => "property",
            ("measurements", None) => "meters",
            ("measurements", Some("telemetry")) => "home-mini",
            ("measurements", Some(_)) => "meter",
            ("ev", None) => "ev-charging",
            ("ev", Some(_)) => "charging-sessions",
            _ => "index",
        };
        format!("{MODULE_ID}/{topic}.md")
    }

    fn get_component(&self, page_id: &str, path: Vec<String>) -> Element {
        let page = match page_id {
            "user" => {
                // let format = time::format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]").unwrap();
                let date_format = time::format_description::parse("[year]-[month]-[day]").unwrap();
                let account_user = &self.account_manager.data_set.data.viewer_;
                let dob = if let Some(date) = &account_user.date_of_birth_ {
                    date.format(&date_format).unwrap()
                }
                else {
                    "".to_string()
                };

                        // tr {
                        //    th { class: "row-header","accounts"} td{"{account_user.accounts_}"}
                        // }
                rsx! {
                    h1 { "Viewer (Current User)" }
                    table { class: "display",
                        tr {
                            th { class: "row-header", "ID" }
                            td { "{account_user.id_}" }
                        }
                        tr {
                            th { class: "row-header", "Full Name" }
                            td { "{account_user.full_name_}" }
                        }
                        tr {
                            th { class: "row-header", "Title" }
                            td { "{account_user.title_}" }
                        }
                        tr {
                            th { class: "row-header", "Preferred Name" }
                            td { "{account_user.preferred_name_}" }
                        }
                        tr {
                            th { class: "row-header", "Given Name" }
                            td { "{account_user.given_name_}" }
                        }
                        tr {
                            th { class: "row-header", "Family Name" }
                            td { "{account_user.family_name_}" }
                        }
                        tr {
                            th { class: "row-header", "Pronouns" }
                            td { {account_user.pronouns_.as_deref().unwrap_or("")} }
                        }
                        tr {
                            th { class: "row-header", "Email" }
                            td { "{account_user.email_}" }
                        }

                        tr {
                            th { class: "row-header", "Mobile" }
                            td { "{account_user.mobile_}" }
                        }
                        tr {
                            th { class: "row-header", "Landline" }
                            td { "{account_user.landline_}" }
                        }
                        tr {
                            th { class: "row-header", "API Key" }
                            td { {account_user.live_secret_key_.as_deref().unwrap_or("")} }
                        }
                        tr {
                            th { class: "row-header", "Is Deceased" }
                            td { "{account_user.is_deceased_}" }
                        }
                        tr {
                            th { class: "row-header", "Date of Birth" }
                            td { {dob} }
                        }
                        tr {
                            th { class: "row-header", "Alternative Phone Numbers" }
                            td { {format!("{:?}", account_user.alternative_phone_numbers_)} }
                        }
                        tr {
                            th { class: "row-header", "Has Family Issues" }
                            td { "{account_user.has_family_issues_}" }
                        }
                        tr {
                            th { class: "row-header", "Is in Hardship" }
                            td { "{account_user.is_in_hardship_}" }
                        }
                        tr {
                            th { class: "row-header", "Is opted in to Wheel of Fortune" }
                            td { "{account_user.is_opted_in_to_wof_}" }
                        }
                    }

                    h2 { "Accounts" }

                    for account in &account_user.accounts_ {
                        div {
                            h3 { "Account {account.number_}" }
                            table { class: "display",
                                tr {
                                    th { class: "row-header", "Brand" }
                                    td { "{account.brand_}" }
                                }
                                tr {
                                    th { class: "row-header", "Overdue Balance" }
                                    td { "{account.overdue_balance_}" }
                                }
                                tr {
                                    th { class: "row-header", "Billing Name" }
                                    td { "{account.billing_name_}" }
                                }
                                tr {
                                    th { class: "row-header", "Billing Sub Name" }
                                    td { {account.billing_sub_name_.as_deref().unwrap_or("")} }
                                }
                                tr {
                                    th { class: "row-header", "Billing EMail" }
                                    td { {account.billing_email_.as_deref().unwrap_or("")} }
                                }
                            }
                        }
                    }
                }
            },
            "account" => {
                let account_user = &self.account_manager.data_set.data.viewer_;
                rsx! {
                    table { class: "display",
                        AccountRow {
                            accounts: account_user.accounts_.clone(),
                            selected_account_id: self.context.account_id.clone(),
                            profile: self.profile.clone(),
                            profile_name: self.profile_name.clone(),
                        }
                        tr {
                            th { class: "row-header", "ID" }
                            td { "{account_user.id_}" }
                        }
                        tr {
                            th { class: "row-header", "Full Name" }
                            td { "{account_user.full_name_}" }
                        }
                        tr {
                            th { class: "row-header", "API Key" }
                            td { {account_user.live_secret_key_.as_deref().unwrap_or("")} }
                        }
                    
                    }
                }
            },
            "bills" => {
                rsx! {
                    BillsPage { path }
                }
            },
            "properties" => {
                rsx! {
                    PropertiesPage { path }
                }
            },
            "measurements" => {
                rsx! {
                    MeasurementsPage { path }
                }
            },
            "ev" => {
                rsx! {
                    EvChargingPage { path }
                }
            },
            _ => {
                rsx! {
                    div { "Unknown page_id, {page_id}" }
                }
            },
        };

        // The key changes if the module is rebuilt, so that OctopusRoot is re-created and provides the new context.
        let context_key = format!("{:p}", Arc::as_ptr(&self.context));
        rsx! {
            OctopusRoot { key: "{context_key}", context: self.context.clone(), {page} }
        }
    }
}


pub struct OctopusModuleFactory {
    context: Arc<MarcoSparkoContext>,
    octopus_args: OctopusArgs,
    cache_config: Arc<CacheConfig>,
    token_manager: Arc<OctopusTokenManager>,
    request_manager: Arc<sparko_graphql::RequestManager>,
    profile: Profile,
    verbose: bool,
    /// Why the saved login could not be used, shown on the login page. None if there was no saved login.
    saved_login_error: std::sync::Mutex<Option<String>>,
}

/// An error from logging in, for display, including the response if the server rejected the request.
fn describe_login_error(error: &sparko_graphql::Error) -> String {
    match error {
        // The response often explains the error, e.g. that a firewall blocked the request.
        sparko_graphql::Error::HttpError(status, message) if !message.is_empty() =>
            format!("the server responded {}: {}", status, message),
        sparko_graphql::Error::HttpError(status, _) => format!("the server responded {}", status),
        _ => error.to_string(),
    }
}

impl OctopusModuleFactory {
    pub async fn do_build(&self) -> anyhow::Result<OctopusModule> {

        let authenticated_request_manager = Arc::new(logging_caller::LoggingCaller(sparko_graphql::AuthenticatedRequestManager::new(self.request_manager.clone(), self.token_manager.clone())?));
       
        let mut client = OctopusModule::new(self.octopus_args.clone(), self.cache_config.clone(), self.context.store()?, self.profile.clone(),
            authenticated_request_manager, self.verbose
        ).await?;
        client.profile_name = self.context.profile.active_profile.name.clone();

        if self.profile.init {
            crate::profile::update_profile(&self.context.profile.active_profile.name, MODULE_ID, &self.profile)?;
        }
        
        Ok(client)
    }

    async fn login(errors: &mut Vec<String>, token_manager: &Arc<OctopusTokenManager>) -> anyhow::Result<()> {

            match token_manager.get_authenticator(true).await {
                Ok(_token) => {
                    crate::log_info!("Logged in to Octopus");
                    Ok(())
                },
                Err(error) => {
                    if let sparko_graphql::Error::GraphQLError(graphql_errors) = &error {
                        let graphql_errors = &**graphql_errors;
                        for graphql_error in graphql_errors {
                            if let Some(error_code) = graphql_error.extensions.get("errorCode") {
                                if error_code == "KT-CT-1138" {
                                    errors.push(format!("Username or password is incorrect."));
                                    return Err(anyhow!(error));
                                }
                                if error_code == "KT-CT-1139" {
                                    errors.push(format!("API KEY is incorrect."));
                                    return Err(anyhow!(error));
                                }
                            }
                        }
                 
                    }
                    errors.push(format!("Login failed, {}", describe_login_error(&error)));
                    crate::log_error!("Login to Octopus failed, {}", describe_login_error(&error));
                    return Err(anyhow!(error));
                },
            }
    }

    async fn handle_login(
        token_manager: Arc<OctopusTokenManager>,
        profile: Profile,
        context: Arc<MarcoSparkoContext>,
        values: LoginForm, 
        error_signal: &mut Signal<Vec<String>>) {
        let mut errors = Vec::new();

        println!("Octopus login submitted for: {:?}", values);
        
        let login_method = values.login_method.trim().to_string();
        
        if login_method == "email" {
            let email = values.email.as_ref().unwrap_or(&String::new()).trim().to_string();
            let password = values.password.as_ref().unwrap_or(&String::new()).trim().to_string();
            
            if email.is_empty() {
                errors.push("Email is required".to_string());
            }
            if password.is_empty() {
                errors.push("Password is required".to_string());
            }

            if errors.is_empty() {
                println!("Performing login for email: {}", email);
                token_manager.set_authenticator(
                    OctopusAuthenticator::from_email_password(email.clone(), password.clone())
                ).await;

                if let Ok(_) =  Self::login(&mut errors, &token_manager).await {
                    let init_signal = try_consume_context::<Signal<InitRequested>>();
                    if let Some(mut init_sig) = init_signal {
                        init_sig.set(InitRequested(true));
                    }
                }

            }
        } else if login_method == "api_key" {
            let api_key = values.api_key.as_ref().unwrap_or(&String::new()).trim().to_string();
            
            if api_key.is_empty() {
                errors.push("API Key is required".to_string());
            }

            if errors.is_empty() {
                println!("Performing login with API key");
                token_manager.set_authenticator(
                    OctopusAuthenticator::from_api_key(api_key.clone())
                ).await;
                if let Ok(_) = Self::login(&mut errors, &token_manager).await {
                    println!("Login successful!");
                    // Store the api_key into the profile
                    let new_profile = Profile {
                        api_key: Some(values.api_key.as_ref().unwrap_or(&String::new()).trim().to_string()),
                        ..profile.clone()
                    };

                    crate::profile::update_profile(&context.profile.active_profile.name, MODULE_ID, &new_profile).unwrap_or_else(|e| println!("profile update failed: {}", e));

                    // Reset the app initialization to reload context with new profile
                    let init_signal = try_consume_context::<Signal<InitRequested>>();
                    if let Some(mut init_sig) = init_signal {
                        init_sig.set(InitRequested(true));
                    }
                }
            }
        } else {
            errors.push("Invalid login method".to_string());
        }

        error_signal.set(errors);
    }
}

#[async_trait]
impl ModuleFactory for OctopusModuleFactory {

    async fn is_ready(&self) -> anyhow::Result<bool> {
        // An offline profile has no login, it only shows its cached data, see Profile::offline.
        if self.context.profile.active_profile.offline {
            return Ok(true);
        }
        let (ready, error) = match self.token_manager.get_authenticator(false).await {
            Ok(_token) => (true, None),
            // There is no saved login, so the login page is shown without an error.
            Err(sparko_graphql::Error::MissingRequiredValueError(_)) => (false, None),
            Err(error) => (false, Some(format!("Unable to log in with the saved login details, {}", describe_login_error(&error)))),
        };
        if let Some(error) = &error {
            crate::log_error!("{error}");
        }
        *self.saved_login_error.lock().unwrap() = error;
        Ok(ready)
    }

    fn init_page(&self) -> Element {
        let mut email = use_signal(|| String::new());
        let mut password = use_signal(|| String::new());
        let mut api_key = use_signal(|| String::new());
        let mut login_method = use_signal(|| "email".to_string());
        let saved_login_error = self.saved_login_error.lock().unwrap().clone();
        let mut errors: Signal<Vec<String>>   = use_signal(|| saved_login_error.into_iter().collect());

        let context: Arc<MarcoSparkoContext> = self.context.clone();
        let token_manager: Arc<OctopusTokenManager> = self.token_manager.clone();
        let profile: Profile = self.profile.clone();
        rsx! {
            for error in errors.read().iter() {
                div { class: "error", "{error}" }
            }
            div {
                h1 { "Octopus Login" }
                form {
                    onsubmit: move |evt: FormEvent| {
                        let context = context.clone();
                        let token_manager = token_manager.clone();
                        let profile = profile.clone();
                        async move {
                            // Prevent the default browser navigation behavior
                            evt.prevent_default();

                            // Extract the form values into the LoginForm struct
                            let values: LoginForm = evt
                                .parsed_values()
                                .expect("Failed to parse form values");

                            Self::handle_login(
                                    token_manager,
                                    profile,
                                    context.clone(),
                                    values,
                                    &mut errors,
                                )
                                .await;
                        }
                    },
                    table {
                        tr {
                            td { colspan: "2",
                                label {
                                    input {
                                        r#type: "radio",
                                        name: "login_method",
                                        value: "email",
                                        checked: login_method() == "email",
                                        onchange: move |_| login_method.set("email".to_string()),
                                    }
                                    " Login with Email and Password"
                                }
                            }
                        }
                        tr {
                            td { colspan: "2",
                                label {
                                    input {
                                        r#type: "radio",
                                        name: "login_method",
                                        value: "api_key",
                                        checked: login_method() == "api_key",
                                        onchange: move |_| login_method.set("api_key".to_string()),
                                    }
                                    " Use API Key"
                                }
                            }
                        }
                        if login_method() == "email" {
                            tr {
                                td {
                                    label { "Email:" }
                                }
                                td {
                                    input {
                                        r#type: "email",
                                        id: "email",
                                        name: "email",
                                        value: "{email}",
                                        oninput: move |e| email.set(e.value().clone()),
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
                                        id: "password",
                                        name: "password",
                                        value: "{password}",
                                        oninput: move |e| password.set(e.value().clone()),
                                    }
                                }
                            }
                        }
                        if login_method() == "api_key" {
                            tr {
                                td {
                                    label { "API Key:" }
                                }
                                td {
                                    input {
                                        r#type: "text",
                                        id: "api_key",
                                        name: "api_key",
                                        value: "{api_key}",
                                        oninput: move |e| api_key.set(e.value().clone()),
                                    }
                                }
                            }
                        }
                        tr {
                            td {
                                button { r#type: "submit", "Log In" }
                            }
                        }
                    }
                }
            }
        }
    }
    
    async fn build(&self) -> anyhow::Result<Box<dyn crate::Module + Send>> {
        Ok(Box::new(self.do_build().await?))
    }
}

pub struct OctopusModuleFactoryBuilder {
    context: Arc<MarcoSparkoContext>, 
    octopus_args: OctopusArgs,
    profile: Profile,
    authenticator: Option<OctopusAuthenticator>,
    url: Option<String>,
    verbose: bool,
}

impl OctopusModuleFactoryBuilder {
    fn new(
            context: Arc<MarcoSparkoContext>,
            json_profile: Option<serde_json::Value>
        ) -> anyhow::Result<OctopusModuleFactoryBuilder> {

        let module_args = if let Some(args) = context.args.module_args(MODULE_ID) {
            args
        }
        else {
            Vec::new()
        };

        let octopus_args = OctopusArgs::parse_from(module_args);
        let profile = if let Some(json) = json_profile {
            serde_json::from_value(json)?
        }
        else {
            Profile::new()
        };

        let option_api_key = if let Some(api_key) = &octopus_args.octopus_api_key {
            Some(api_key.to_string())
        }
        else {
            if let Some(api_key) = &profile.api_key {
                Some(api_key.to_string())
            }
            else {
                None
            }
        };

        let authenticator = if let Some(api_key) = option_api_key {
            Some(OctopusAuthenticator::from_api_key(api_key))
        }
        else {
            None
        };

        let verbose = context.args.marco_sparko_args.verbose;

        if verbose {
            println!("Octopus Module Args: {:?}", octopus_args);
            println!("Octopus Module Profile: {:?}", profile);
        }  

        Ok(OctopusModuleFactoryBuilder {
            context,
            octopus_args,
            profile,
            authenticator,
            url: None,
            verbose,
        })
    }

    // pub fn with_url(mut self, url: String) -> anyhow::Result<OctopusModuleFactoryBuilder> {
    //     self.url = Some(url);
    //     Ok(self)
    // }

    // pub fn with_url_if_not_set(mut self, url: String) -> anyhow::Result<OctopusModuleFactoryBuilder> {
    //     if let None = self.url {
    //         self.url = Some(url);
    //     }
    //     Ok(self)
    // }

    // pub fn with_api_key(mut self, api_key: String) -> anyhow::Result<OctopusModuleFactoryBuilder> {
    //     self.authenticator = Some(OctopusAuthenticator::from_api_key(api_key));
    //     Ok(self)
    // }

    // pub fn with_password(mut self, email: String, password: String) -> anyhow::Result<OctopusModuleFactoryBuilder> {
    //     self.authenticator = Some(OctopusAuthenticator::from_email_password(email, password));
    //     Ok(self)
    // }

    pub fn build(self) -> anyhow::Result<OctopusModuleFactory> {
        let url = if let Some(url) = self.url {
            url
        }
        else {
            format!("https://{}/v1/graphql/", self.profile.api_host)
        };

        let verbose = self.context.args.marco_sparko_args.verbose;

        if verbose {
            println!("Octopus API initialized with URL {}", &url);
        }
        
        let request_manager = Arc::new(sparko_graphql::RequestManager::new(url, verbose, create_info::USER_AGENT)?);
        let cache_config = self.context.create_cache_config(crate::octopus::MODULE_ID)?;

        Ok(OctopusModuleFactory {
            context: self.context.clone(),
            octopus_args: self.octopus_args,
            cache_config,
            token_manager: Arc::new(OctopusTokenManager::new(
                self.context,
                 request_manager.clone(),
                 self.authenticator)),
            request_manager,
            profile: self.profile,
            verbose: self.verbose,
            saved_login_error: std::sync::Mutex::new(None),
        })
    }
}
