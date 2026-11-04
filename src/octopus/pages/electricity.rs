//! Octopus's electricity for Home's Electricity page, see views/electricity.rs: imported and exported, combined from the
//! meters' half hourly readings and the Home Mini's readings every 10 seconds, and split into what it was used for by
//! whatever measured that, e.g. the Tesla module, see power::measured, with Octopus's split of the car's import as a
//! line over the bars, to compare.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use dioxus::prelude::*;
use sparko_graphql::types::DateTime;
use time_tz::Tz;

use crate::data_set::CancelToken;
use crate::octopus::context::OctopusContext;
use crate::power::combine::{CheapTimes, Estimate};
use crate::power::grid::GridReading;
use crate::octopus::graphql::ElectricityDirection;
use crate::octopus::graphql::meter::active_meter_points_with_agreements::ElectricityTariffType;
use crate::octopus::measurements::MeasurementsDataSet;
use crate::octopus::meter_point::MeterPointDataSet;
use crate::octopus::property::PropertyDataSet;
use crate::octopus::smart_device::SmartDeviceDataSet;
use crate::octopus::telemetry;
use crate::power::{Electricity, ElectricitySource, EvComparison, LoadResult, LoadedElectricity, SiteData, SourceKind};
use crate::components::error_text::ErrorText;
use crate::views::electricity::ev_comparison;
use super::measurements::graphql_error_descriptions;

/// What Octopus's split of the car's import in the half hours outside the cheap times did to the bill, compared with
/// the measured split, see EvComparison::octopus_outside: the car's import there is at the low rate, and the house's at
/// the standard one, so each kWh more of the car's is the difference between them less. rates are the standard and low
/// rates in pence a kWh, None if they are not known.
fn outside_cheap_times(comparison: &EvComparison, rates: Option<(f64, f64)>) -> String {
    let more = comparison.octopus_outside - comparison.measured_outside;
    if comparison.octopus_outside.abs() < 0.005 && comparison.measured_outside.abs() < 0.005 {
        return String::from("All of it was in the cheap times, where the split does not change what it costs.");
    }
    let worth = rates.map(|(standard, low)| {
        let pounds = more.abs() * (standard - low) / 100.0;
        format!(", worth £{pounds:.2} {}", if more >= 0.0 { "less on the bill than the measured split" } else { "more on the bill than the measured split" })
    }).unwrap_or_default();
    format!("Outside the cheap times, where the split decides the rate: measured {:.2} kWh, Octopus {:.2} kWh, so Octopus put {:.2} kWh {} at the low rate{worth}.",
        comparison.measured_outside, comparison.octopus_outside, more.abs(), if more >= 0.0 { "more" } else { "less" })
}

struct ElectricityMeterPoint {
    mpan: String,
    direction: Option<ElectricityDirection>,
    has_active_agreement: bool,
    cheap_times: Option<CheapTimes>,
    intelligent: bool,
    /// Its standard and low rates, in pence a kWh, for what Octopus's split of the car's import is worth, if it is a
    /// day and night tariff, as Intelligent Octopus Go is.
    rates: Option<(f64, f64)>,
}

fn meter_point_for_direction(
    meter_points: &[ElectricityMeterPoint],
    direction: ElectricityDirection,
) -> Option<&ElectricityMeterPoint> {
    meter_points.iter()
        .find(|point| point.has_active_agreement && point.direction.as_ref() == Some(&direction))
        .or_else(|| meter_points.iter().find(|point| point.direction.as_ref() == Some(&direction)))
}

#[cfg(test)]
mod meter_point_selection_tests {
    use super::{ElectricityMeterPoint, meter_point_for_direction};
    use crate::octopus::graphql::ElectricityDirection;

    fn point(mpan: &str, direction: ElectricityDirection, has_active_agreement: bool) -> ElectricityMeterPoint {
        ElectricityMeterPoint {
            mpan: mpan.to_string(),
            direction: Some(direction),
            has_active_agreement,
            cheap_times: None,
            intelligent: false,
            rates: None,
        }
    }

    #[test]
    fn intelligent_octopus_tariffs_are_known_by_their_product_codes() {
        use super::is_intelligent;
        assert!(is_intelligent("IOG-VAR-24-10-29") && is_intelligent("INTELLI-VAR-22-10-14") && !is_intelligent("GO-VAR-22-10-14"));
    }

    #[test]
    fn prefers_a_meter_point_with_an_active_agreement() {
        let meter_points = [
            point("inactive", ElectricityDirection::Import, false),
            point("active", ElectricityDirection::Import, true),
        ];
        assert_eq!(
            meter_point_for_direction(&meter_points, ElectricityDirection::Import).map(|point| point.mpan.as_str()),
            Some("active")
        );
    }

    #[test]
    fn falls_back_to_a_meter_point_without_an_active_agreement() {
        let meter_points = [point("only", ElectricityDirection::Import, false)];
        assert_eq!(
            meter_point_for_direction(&meter_points, ElectricityDirection::Import).map(|point| point.mpan.as_str()),
            Some("only")
        );
    }
}

/// Whether a tariff's product code is an Intelligent Octopus one, e.g. IOG-VAR-24-10-29 (Intelligent Octopus Go) or
/// INTELLI-VAR-22-10-14 (Intelligent Octopus), on which charging the car outside the cheap times is at the low rate.
fn is_intelligent(product_code: &str) -> bool {
    product_code.starts_with("IOG") || product_code.starts_with("INTELLI")
}

/// The account's import and export meter points, its Home Mini's meter, and what is known of its tariff and charger,
/// see Estimate, looked up each time the electricity is loaded, from their caches.
struct Account {
    import_mpan: String,
    export_mpan: Option<String>,
    esme_id: Option<String>,
    estimate: Estimate,
    /// The standard and low rates, in pence a kWh, for what Octopus's split of the car's import is worth.
    rates: Option<(f64, f64)>,
}

impl Account {
    async fn load(context: &OctopusContext) -> anyhow::Result<Account> {
        let meter_points = MeterPointDataSet::new(&context.account_id, &context.cache_config, &*context.request_manager).await
            .context("Failed to load the meter points")?;
        let properties = PropertyDataSet::new(&context.account_id, &context.cache_config, &*context.request_manager).await
            .context("Failed to load the properties")?;
        // The import meter point's cheap times, from the time of use scheme of its current agreement.
        let meter_points = meter_points.meter_points.iter()
            .map(|meter_point| {
                let has_active_agreement = !meter_point.agreements_.is_empty();
                let (cheap_times, intelligent, rates) = if meter_point.direction_ == Some(ElectricityDirection::Import)
                    && has_active_agreement
                {
                    let timeslots: Vec<(String, String, String)> = meter_point.agreements_.iter()
                        .filter_map(|agreement| agreement.time_of_use_scheme_.as_ref())
                        .flat_map(|scheme| scheme.timeslots_.iter())
                        .map(|slot| (
                            slot.timeslot_.clone(),
                            slot.active_from_.as_str().unwrap_or_default().to_string(),
                            slot.active_to_.as_str().unwrap_or_default().to_string(),
                        ))
                        .collect();
                    let cheap_times = CheapTimes::from_timeslots(&timeslots);
                    let intelligent = meter_point.agreements_.iter()
                        .filter_map(|agreement| agreement.tariff_.as_ref())
                        .filter_map(|tariff| match tariff {
                            ElectricityTariffType::StandardTariff(tariff) => tariff.product_code_.as_deref(),
                            ElectricityTariffType::DayNightTariff(tariff) => tariff.product_code_.as_deref(),
                            ElectricityTariffType::ThreeRateTariff(tariff) => tariff.product_code_.as_deref(),
                            ElectricityTariffType::FourRateEvTariff(tariff) => tariff.product_code_.as_deref(),
                            ElectricityTariffType::HalfHourlyTariff(tariff) => tariff.product_code_.as_deref(),
                            ElectricityTariffType::PrepayTariff(tariff) => tariff.product_code_.as_deref(),
                        })
                        .any(is_intelligent);
                    let rates = meter_point.agreements_.iter()
                        .filter_map(|agreement| agreement.tariff_.as_ref())
                        .find_map(|tariff| match tariff {
                            ElectricityTariffType::DayNightTariff(tariff) => tariff.day_rate_.zip(tariff.night_rate_),
                            _ => None,
                        });
                    (cheap_times, intelligent, rates)
                }
                else {
                    (None, false, None)
                };
                ElectricityMeterPoint {
                    mpan: meter_point.mpan_.clone(),
                    direction: meter_point.direction_.clone(),
                    has_active_agreement,
                    cheap_times,
                    intelligent,
                    rates,
                }
            })
            .collect::<Vec<_>>();
        let esme_id = properties.property_map.values()
            .flat_map(|property| property.smart_device_networks_.iter())
            .flat_map(|network| network.smart_devices_.iter())
            .find(|device| device.type_.to_string().trim_matches('"') == "ESME")
            .map(|device| device.device_id_.clone());
        let import_point = meter_point_for_direction(&meter_points, ElectricityDirection::Import)
            .ok_or_else(|| anyhow!("There is no electricity import meter point on this account"))?;
        // The charger, for the power at which the car is charging. Without it, the rate is still estimated from the
        // cheap times, but not while charging outside them, and the car's share of the import is not placed where it
        // was charging.
        let charger_watts = match SmartDeviceDataSet::new(&context.account_id, &context.cache_config, &*context.request_manager).await {
            Ok(devices) => devices.devices.iter()
                .filter(|device| device.charging.is_some())
                .filter_map(|device| device.power.as_ref().and_then(|power| power.to_string().parse::<f64>().ok()))
                .map(|kw| kw * 1000.0)
                .reduce(f64::min),
            Err(_) => None,
        };
        Ok(Account {
            import_mpan: import_point.mpan.clone(),
            export_mpan: meter_point_for_direction(&meter_points, ElectricityDirection::Export).map(|point| point.mpan.clone()),
            esme_id,
            estimate: Estimate { cheap: import_point.cheap_times.clone(), cheap_charging: import_point.intelligent, charger_watts },
            rates: import_point.rates,
        })
    }
}

/// The account's electricity: its meters and Home Mini, see power::combine, for Home's Electricity page.
pub struct OctopusElectricity {
    pub context: Arc<OctopusContext>,
}

/// The readings for the range shown, and why the Home Mini's could not be fetched, if they could not, as the meters'
/// are still worth showing.
struct Readings {
    data: SiteData,
    home_mini_error: Option<String>,
    /// The standard and low rates, see Account::rates.
    rates: Option<(f64, f64)>,
}

impl LoadedElectricity for Readings {
    fn data(&self) -> &SiteData {
        &self.data
    }

    fn notes(&self, electricity: &Electricity) -> Element {
        let cheap_known = self.data.estimate.cheap.is_some();
        let rates = self.rates;
        rsx! {
            if let Some(comparison) = ev_comparison(&electricity.points) {
                p { class: "derived",
                    "The car, where it was measured and Octopus has split the half hours: measured {comparison.measured:.2} kWh, Octopus {comparison.octopus:.2} kWh, "
                    {format!("difference {:+.2} kWh ({:+.1}%)", comparison.octopus - comparison.measured,
                        if comparison.measured > 0.0 { (comparison.octopus - comparison.measured) / comparison.measured * 100.0 } else { 0.0 })}
                }
                if cheap_known {
                    p { class: "derived", {outside_cheap_times(&comparison, rates)} }
                }
            }
        }
    }

    fn warning(&self) -> Option<String> {
        self.home_mini_error.as_ref().map(|error| format!("The Home Mini's readings could not be fetched, only the meters' are shown: {error}"))
    }
}

impl ElectricitySource for OctopusElectricity {
    fn timezone(&self) -> &'static Tz {
        self.context.billing_timezone
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Supplier
    }

    fn compares_ev(&self) -> bool {
        true
    }

    fn reload_title(&self) -> String {
        String::from("Fetch the readings shown from Octopus again")
    }

    fn load(&self, start: DateTime, end: DateTime, reload_period: Option<(DateTime, DateTime)>, mut progress: Signal<String>,
        token: CancelToken) -> Pin<Box<dyn Future<Output = LoadResult>>>
    {
        let context = self.context.clone();
        let account_id = context.account_id.clone();
        let tz = self.timezone();
        Box::pin(async move {
            progress.set(String::from("Loading the meter points"));
            let Account { import_mpan, export_mpan, esme_id, estimate, rates } = Account::load(&context).await?;
            // The progress of each fetch, with what it is fetching.
            let monitor = |what: &'static str| move |message: String| { let mut progress = progress; progress.set(format!("{what}: {message}")) };
            let import = MeasurementsDataSet::new(&account_id, &import_mpan, false, &start, &end, reload_period, tz,
                &monitor("Import meter"), &token, &context.cache_config, &*context.request_manager).await?;
            let export = match &export_mpan {
                Some(export_mpan) => Some(MeasurementsDataSet::new(&account_id, export_mpan, true, &start, &end, reload_period, tz,
                    &monitor("Export meter"), &token, &context.cache_config, &*context.request_manager).await?),
                None => None,
            };
            let (home_mini, home_mini_error) = match &esme_id {
                Some(esme_id) => match telemetry::fetch(&account_id, esme_id, &start, &end, reload_period, tz, &context.cache_config,
                    &*context.request_manager, &monitor("Home Mini"), &token).await
                {
                    Ok(readings) => (readings, None),
                    Err(error) => (Vec::new(), Some(match graphql_error_descriptions(&error) {
                        Some(descriptions) => descriptions.join(", "),
                        None => error.to_string(),
                    })),
                },
                None => (Vec::new(), None),
            };
            // The half hours either side too, for whether the car was charging in them, see electricity::combine.
            let (before, after) = (DateTime::from_unix_timestamp(start.unix_timestamp() - 1800)?, DateTime::from_unix_timestamp(end.unix_timestamp() + 1800)?);
            // The car's charging power, from the most the meter has given the car in a half hour, which is what it
            // actually charges at, e.g. 7.1 kW for a charger rated at 7.4 kW.
            let charging_watts = import.half_hours(&DateTime::from_unix_timestamp(0)?, &DateTime::from_unix_timestamp(i32::MAX as i64)?).iter()
                .map(|half_hour| half_hour.units.iter().filter(|(label, _)| label.starts_with("EV")).map(|(_, units)| units).sum::<f64>())
                .fold(0.0, f64::max) * 2000.0;
            // The charging power measured from the meter's readings, rather than the charger's rating, if it can be.
            let charging_watts = (charging_watts >= 1000.0).then_some(charging_watts);
            let readings: Arc<dyn LoadedElectricity> = Arc::new(Readings {
                data: SiteData {
                    import: import.half_hours(&before, &after),
                    export: export.map(|export| export.half_hours(&start, &end)).unwrap_or_default(),
                    grid: home_mini.iter().map(GridReading::from_sample).collect(),
                    grid_name: String::from("Home Mini"),
                    estimate: Estimate { charger_watts: charging_watts.or(estimate.charger_watts), ..estimate },
                },
                home_mini_error,
                rates,
            });
            anyhow::Ok(readings)
        })
    }

    fn error(&self, error: &anyhow::Error) -> Element {
        match graphql_error_descriptions(error) {
            Some(descriptions) => rsx! {
                div { class: "error",
                    "Octopus did not provide the readings:"
                    ul {
                        for description in descriptions {
                            li { "{description}" }
                        }
                    }
                }
            },
            None => rsx! { div { class: "error", ErrorText { error: format!("{error:#}"), message: "Failed to load the readings" } } },
        }
    }
}
