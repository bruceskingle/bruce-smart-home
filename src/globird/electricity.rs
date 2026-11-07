//! GloBird's electricity for Home's Electricity page and its own Meters page, see views/electricity.rs: the smart
//! meter's 5 minute readings, from the cache, by the tariff's charge types, e.g. Peak Usage, see fetch::readings, and
//! their costs at the rates worked out from GloBird's daily costs, see rates.rs.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use dioxus::prelude::*;
use sparko_graphql::types::{Currency, DateTime};
use time::{Date, OffsetDateTime};
use time_tz::Tz;

use crate::data_set::CancelToken;
use crate::period::Period;
use crate::power::combine::MeterReading;
use crate::power::{ElectricitySource, LoadResult, LoadedElectricity, SiteData, SourceKind};

use super::GlobirdContext;
use super::fetch::{self, DaySummary, Reading};
use super::rates::{self, SOLAR, SUPPLY, USAGE};

/// The site's timezone, for its days and the times shown. For now every GloBird customer's is Sydney's, which is the
/// time on the clocks in New South Wales, Victoria, the ACT and Tasmania, until a site has a timezone of its own, see
/// docs/PowerSchema.md. GloBird's own days are in AEST all year, so in summer a day here starts an hour before one of
/// theirs, and the days' totals differ from the Days page's.
const SITE_TIMEZONE: &Tz = time_tz::timezones::db::australia::SYDNEY;

/// The 5 minute readings in a day, over which its daily charge is shared out.
const READINGS_A_DAY: f64 = 288.0;

/// The periods which can be chosen: none shorter than the readings, which are 5 minutes.
const PERIODS: [Period; 6] = [Period::FiveMinutes, Period::TenMinutes, Period::HalfHour, Period::Day, Period::Week, Period::Month];

/// The meter's readings, of all its registers, or of one, e.g. E1, for the Meters page, see register.
pub struct GlobirdElectricity {
    pub context: Arc<GlobirdContext>,
    /// The register shown, e.g. E1, all of them if None. A register's readings are shown as import, whichever way
    /// they go, as on Octopus's page of one meter.
    pub register: Option<String>,
}

/// The cached readings of a range.
struct GlobirdReadings {
    data: SiteData,
}

impl LoadedElectricity for GlobirdReadings {
    fn data(&self) -> &SiteData {
        &self.data
    }
}

/// The readings as meter readings, each by its charge type: the import registers' as import and the export
/// registers' as export, or those of register as import, if it is one, with their costs, in cents: the kWh at the rate
/// of the charge type on the reading's NEM day, worked out from GloBird's daily costs, see rates::tariffs, as GloBird
/// gives no rates, negative for the export's credit, and an import reading's 5 minutes' share of the day's supply
/// charge, as GloBird charged it, or else the tariff's. A reading whose charge type has no rate, or whose day no
/// tariff, has no cost, as it can not be known.
fn site_data(readings: &[Reading], days: &[DaySummary], register: Option<&str>) -> SiteData {
    let tariffs = rates::tariffs(days);
    let supply: BTreeMap<Date, f64> = days.iter()
        .filter_map(|day| {
            let date = day.date?;
            Some((date, day.cost.get(SUPPLY).copied().or_else(|| rates::tariff_of(&tariffs, date)?.daily)?))
        })
        .collect();
    let mut data = SiteData { currency: Currency::from_code("AUD"), ..SiteData::default() };
    for reading in readings {
        let import = reading.register.starts_with('E');
        let date = OffsetDateTime::from_unix_timestamp(reading.start).ok().map(|time| time.to_offset(fetch::nem_offset()).date());
        let cost = date.and_then(|date| rates::tariff_of(&tariffs, date))
            .and_then(|tariff| tariff.rate(if import { USAGE } else { SOLAR }, &reading.band))
            .map(|rate| reading.kwh * rate * 100.0);
        let meter_reading = MeterReading {
            start: reading.start,
            end: reading.end,
            units: BTreeMap::from([(reading.band.clone(), reading.kwh)]),
            costs: cost.map(|cost| BTreeMap::from([(reading.band.clone(), cost)])).unwrap_or_default(),
            standing_charge: date.filter(|_| import).and_then(|date| supply.get(&date)).map_or(0.0, |daily| daily * 100.0 / READINGS_A_DAY),
            ..MeterReading::default()
        };
        match register {
            Some(register) if reading.register == register => data.import.push(meter_reading),
            Some(_) => {},
            None if import => data.import.push(meter_reading),
            None => data.export.push(meter_reading),
        }
    }
    data
}

impl ElectricitySource for GlobirdElectricity {
    fn kind(&self) -> SourceKind {
        SourceKind::Supplier
    }

    fn timezone(&self) -> &'static Tz {
        SITE_TIMEZONE
    }

    fn periods(&self) -> Vec<Period> {
        PERIODS.to_vec()
    }

    /// The smart meter's 5 minute readings, as GloBird's usageArray gives them.
    fn reading_period(&self) -> Option<Period> {
        Some(Period::FiveMinutes)
    }

    fn reload_title(&self) -> String {
        String::from("Fetch the latest readings from GloBird")
    }

    /// The cached readings, after fetching the latest if reload is set, as GloBird's are fetched a day at a time, see
    /// fetch::fetch, whatever the range.
    fn load(&self, start: DateTime, end: DateTime, reload: Option<(DateTime, DateTime)>, mut progress: Signal<String>,
        _cancel: CancelToken) -> Pin<Box<dyn Future<Output = LoadResult>>>
    {
        let context = self.context.clone();
        let register = self.register.clone();
        Box::pin(async move {
            if reload.is_some() {
                context.fetch(&mut |message| progress.set(message)).await?;
            }
            let (start, end) = (start.unix_timestamp(), end.unix_timestamp());
            let readings: Vec<Reading> = fetch::cached_readings(&context.cache_config).into_iter()
                .filter(|reading| reading.end > start && reading.start < end)
                .collect();
            let data = site_data(&readings, &fetch::summaries(&context.cache_config), register.as_deref());
            let loaded: Arc<dyn LoadedElectricity> = Arc::new(GlobirdReadings { data });
            anyhow::Ok(loaded)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::combine::{Estimate, combine};

    fn reading(start: i64, register: &str, band: &str, kwh: f64) -> Reading {
        Reading { start, end: start + 300, register: register.to_string(), band: band.to_string(), kwh }
    }

    // 2026-11-01 00:00 UTC, 11:00 in Sydney.
    const START: i64 = 1_793_491_200;

    fn readings() -> [Reading; 4] {
        [
            reading(START, "E1", "Peak Usage", 0.1),
            reading(START + 300, "E1", "Offpeak Usage", 0.2),
            reading(START + 300, "B1", "Standard Solar Feed in", 0.5),
            reading(START + 1800, "E1", "Peak Usage", 0.4),
        ]
    }

    #[test]
    fn readings_are_added_up_into_periods_by_charge_type_with_the_export() {
        let data = site_data(&readings(), &[], None);
        let electricity = combine(&data.import, &data.export, &[], "", None, START, START + 3600, Period::HalfHour, &Estimate::default(), SITE_TIMEZONE);
        assert_eq!(electricity.labels, ["Offpeak Usage", "Peak Usage"]);
        assert_eq!(electricity.points.len(), 2);
        assert_eq!((electricity.points[0].start, electricity.points[0].end), (START, START + 1800));
        assert_eq!(electricity.points[0].import, [0.2, 0.1]);
        assert_eq!(electricity.points[0].export, 0.5);
        assert_eq!(electricity.points[1].import, [0.0, 0.4]);
        // In 10 minute periods, two readings in each.
        let electricity = combine(&data.import, &data.export, &[], "", None, START, START + 3600, Period::TenMinutes, &Estimate::default(), SITE_TIMEZONE);
        assert_eq!(electricity.points[0].end - electricity.points[0].start, 600);
        assert_eq!(electricity.points[0].import, [0.2, 0.1]);
    }

    #[test]
    fn a_register_is_shown_as_import_whichever_way_it_goes() {
        let data = site_data(&readings(), &[], Some("B1"));
        assert!(data.export.is_empty());
        assert_eq!(data.import.len(), 1);
        assert_eq!(data.import[0].units.get("Standard Solar Feed in"), Some(&0.5));
    }

    /// Days from 1 November 2026 whose costs are their kWh at 37 and 20 cents a kWh, and a credit of 3 cents a kWh
    /// exported, with a daily charge of $1.20, as GloBird would give them, rounded to the cent.
    fn days() -> Vec<DaySummary> {
        (0..10).map(|index| {
            let (peak, offpeak, export) = (1.0 + index as f64 * 0.7, 20.0 + (index % 3) as f64 * 2.3, 2.0 + (index % 4) as f64);
            let cent = |amount: f64| (amount * 100.0).round() / 100.0;
            DaySummary {
                date: Some(time::macros::date!(2026 - 11 - 01) + time::Duration::days(index)),
                import: BTreeMap::from([(String::from("Peak Usage"), peak), (String::from("Offpeak Usage"), offpeak)]),
                export,
                export_by_type: BTreeMap::from([(String::from("Standard Solar Feed in"), export)]),
                readings: 288,
                cost: BTreeMap::from([
                    (String::from(USAGE), cent(peak * 0.37 + offpeak * 0.20)),
                    (String::from(SOLAR), cent(-export * 0.03)),
                    (String::from(SUPPLY), 1.2),
                ]),
                quantity: BTreeMap::from([(String::from(USAGE), peak + offpeak), (String::from(SOLAR), export)]),
                ..Default::default()
            }
        }).collect()
    }

    #[test]
    fn readings_cost_their_kwh_at_their_days_rates_with_the_daily_charge_shared_out() {
        // 1 November 2026 in NEM time.
        let day_start = fetch::nem_day_start(time::macros::date!(2026 - 11 - 01));
        let readings = [
            reading(day_start, "E1", "Peak Usage", 0.5),
            reading(day_start + 300, "B1", "Standard Solar Feed in", 1.0),
            reading(day_start + 600, "E1", "", 1.0),
        ];
        let data = site_data(&readings, &days(), None);
        assert_eq!(data.currency, Currency::from_code("AUD"));
        assert!((data.import[0].costs["Peak Usage"] - 18.5).abs() < 0.1, "{:?}", data.import[0]);
        assert!((data.export[0].costs["Standard Solar Feed in"] + 3.0).abs() < 0.1, "{:?}", data.export[0]);
        // A reading in no charge type has no rate, but its share of the daily charge.
        assert!(data.import[1].costs.is_empty());
        assert_eq!(data.import[1].standing_charge, 120.0 / 288.0);
        assert_eq!(data.export[0].standing_charge, 0.0);
        // An export register's page has its credits under its charge type.
        let data = site_data(&readings, &days(), Some("B1"));
        assert_eq!(data.import.len(), 1);
        assert!(data.import[0].costs["Standard Solar Feed in"] < 0.0);
    }

    #[test]
    fn a_day_is_a_day_in_sydney() {
        // 2026-10-31 13:00 UTC is midnight on 1 November in Sydney, in summer time.
        let midnight = 1_793_451_600;
        let data = site_data(&[reading(midnight - 300, "E1", "Offpeak Usage", 1.0), reading(midnight, "E1", "Offpeak Usage", 2.0)], &[], None);
        let electricity = combine(&data.import, &data.export, &[], "", None, midnight - 86400, midnight + 86400, Period::Day, &Estimate::default(), SITE_TIMEZONE);
        assert_eq!(electricity.points.iter().map(|point| point.import[0]).collect::<Vec<_>>(), [1.0, 2.0]);
        assert_eq!(electricity.points[1].start, midnight);
    }
}
