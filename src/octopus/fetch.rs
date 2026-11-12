//! Fetches the latest data for every page without a window, see Module::fetch_latest_data, e.g. from a scheduled task, so
//! that when the app is opened its data is already up to date, and data which Octopus only keeps for a while, e.g. the
//! Home Mini's readings, is never lost. It fetches what the pages would if each was shown with the last FETCH_DAYS days,
//! so what is already cached and up to date is not fetched again.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use super::bill::{BillDataSet, BillTransactionBreakDown};
use super::charging_session;
use super::context::OctopusContext;
use super::graphql::ElectricityDirection;
use super::measurements::MeasurementsDataSet;
use super::meter_point::MeterPointDataSet;
use super::property::PropertyDataSet;
use super::smart_device::SmartDeviceDataSet;
use super::telemetry;
use crate::FETCH_DAYS;
use crate::components::date_range_picker::{LocalSlot, RangeSelection};
use crate::data_set::CancelToken;

/// A progress monitor which prints each message with what it is for. A rate limit's countdown changes every second, so
/// a message which only differs from the last in its countdown is printed once a minute.
fn printer(what: String) -> impl Fn(String) {
    let last = RefCell::new((String::new(), None::<Instant>));
    move |message: String| {
        let key = message.split(", continuing in ").next().unwrap_or_default().to_string();
        let mut last = last.borrow_mut();
        if key != last.0 || last.1.is_none_or(|time| time.elapsed() >= Duration::from_secs(60)) {
            println!("  {what}: {message}");
            *last = (key, Some(Instant::now()));
        }
    }
}

/// Fetches the data of each page for the last FETCH_DAYS days, see the module comment. Each part is fetched even if one
/// before it failed, the error is the number which failed.
pub async fn fetch_latest_data(context: &OctopusContext) -> anyhow::Result<()> {
    let tz = context.billing_timezone;
    let account = &context.account_id;
    let config = &context.cache_config;
    let request_manager = &*context.request_manager;
    let cancel = CancelToken::new();
    // The 30 Days quick range of the date range picker.
    let today = crate::clock::today(tz);
    let range = RangeSelection {
        start: LocalSlot {
            date: today - time::Duration::days(FETCH_DAYS),
            slot: 0,
        },
        end: None,
    };
    let (start, end) = range.to_date_times(tz).map_err(|error| anyhow::anyhow!(error))?;
    println!("Fetching account {account} from {start} to {end}");

    let mut failures = 0;
    let mut failed = |what: &str, error: anyhow::Error| {
        println!("  {what}: FAILED: {error:#}");
        crate::log_error!("Fetching {what} failed: {error:#}");
        failures += 1;
    };

    // Bills, and the transactions and line items of those issued in the range, as when one is opened.
    match BillDataSet::new(account, config, request_manager).await {
        Ok(bills) => {
            println!("  Bills: {} bills", bills.data_set.data.len());
            for (id, (_, bill)) in &bills.data_set.data {
                if bill
                    .as_bill_interface()
                    .issued_date_
                    .as_ref()
                    .is_some_and(|issued| issued.at_midnight(tz) >= start)
                {
                    match BillTransactionBreakDown::for_statement(
                        account.clone(),
                        id.clone(),
                        tz,
                        config,
                        request_manager,
                    )
                    .await
                    {
                        Ok(transactions) => println!("  Bill {id}: {} transactions", transactions.len()),
                        Err(error) => failed(&format!("Bill {id}"), error),
                    }
                }
            }
        }
        Err(error) => failed("Bills", error),
    }

    // The properties, which have the meters' devices, e.g. the Home Mini.
    let properties = PropertyDataSet::new(account, config, request_manager).await;
    if let Err(error) = &properties {
        failed("Properties", anyhow::anyhow!("{error:#}"));
    }

    // Each meter point's readings.
    match MeterPointDataSet::new(account, config, request_manager).await {
        Ok(meter_points) => {
            for meter_point in &meter_points.meter_points {
                let Some(direction) = &meter_point.direction_ else {
                    continue;
                };
                let export = matches!(direction, ElectricityDirection::Export);
                let what = format!("Meter {}", meter_point.mpan_);
                match MeasurementsDataSet::new(
                    account,
                    &meter_point.mpan_,
                    export,
                    &start,
                    &end,
                    None,
                    tz,
                    &printer(what.clone()),
                    &cancel,
                    config,
                    request_manager,
                )
                .await
                {
                    Ok(_) => println!("  {what}: up to date"),
                    Err(error) => failed(&what, error),
                }
            }
        }
        Err(error) => failed("Meter points", error),
    }

    // Each charger's charging sessions.
    match SmartDeviceDataSet::new(account, config, request_manager).await {
        Ok(devices) => {
            for device in devices.devices.iter().filter(|device| device.charging.is_some()) {
                let what = format!("Charging sessions of {}", device.name.as_deref().unwrap_or(&device.id));
                match charging_session::fetch(
                    account,
                    &device.id,
                    &start,
                    &end,
                    None,
                    tz,
                    config,
                    request_manager,
                    &printer(what.clone()),
                    &cancel,
                )
                .await
                {
                    Ok(sessions) => println!("  {what}: {} sessions", sessions.len()),
                    Err(error) => failed(&what, error),
                }
            }
        }
        Err(error) => failed("Smart devices", error),
    }

    // The readings every 10 seconds of each meter with a consumer access device, e.g. a Home Mini, which Octopus only
    // keeps for 30 days.
    if let Ok(properties) = &properties {
        let meters = properties
            .property_map
            .values()
            .flat_map(|property| property.smart_device_networks_.iter())
            .filter(|network| {
                network
                    .smart_devices_
                    .iter()
                    .any(|device| device.type_.to_string().trim_matches('"') == "CAD")
            })
            .flat_map(|network| network.smart_devices_.iter())
            .filter(|device| matches!(device.type_.to_string().trim_matches('"'), "ESME" | "GSME"));
        for meter in meters {
            let what = format!("Home Mini readings of {}", meter.device_id_);
            match telemetry::fetch(
                account,
                &meter.device_id_,
                &start,
                &end,
                None,
                tz,
                config,
                request_manager,
                &printer(what.clone()),
                &cancel,
            )
            .await
            {
                Ok(readings) => println!("  {what}: {} readings", readings.len()),
                Err(error) => failed(&what, error),
            }
        }
    }

    match failures {
        0 => Ok(()),
        _ => Err(anyhow::anyhow!("{failures} of the fetches failed")),
    }
}
