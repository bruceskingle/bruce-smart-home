//! The Tesla gateway's registers, as the profile's capture devices and the app saved them, see snapshot.rs, for Home's
//! Electricity page: what they measured of the site's use, which splits the grid's import by use, see power::measured,
//! and the site meter's readings of the grid, which are used if the profile has no others, e.g. a supplier's.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use dioxus::prelude::*;
use bruce_graphql::types::DateTime;
use time_tz::Tz;

use crate::data_set::CancelToken;
use crate::period::Period;
use crate::power::grid::GridReading;
use crate::power::measured::{MeasurementSource, Measurements, RegisterMeasurements, SiteRegisters};
use crate::power::{ElectricitySource, LoadResult, LoadedElectricity, SiteData, SourceKind};

/// What a period's electricity is said to be from.
const NAME: &str = "Tesla";

/// The registers from start to end, read from the day files, which are large, off the page's thread.
async fn load_registers(store: Arc<bruce_db::Store>, start: i64, end: i64) -> Vec<SiteRegisters> {
    tokio::task::spawn_blocking(move || super::snapshot::registers(&store, start, end)).await.unwrap_or_default()
}

/// The registers as measurements of the site's use.
pub struct TeslaMeasurements {
    pub store: Arc<bruce_db::Store>,
}

impl MeasurementSource for TeslaMeasurements {
    fn load(&self, start: i64, end: i64) -> Pin<Box<dyn Future<Output = anyhow::Result<Arc<dyn Measurements>>>>> {
        let store = self.store.clone();
        Box::pin(async move {
            let measurements: Arc<dyn Measurements> =
                Arc::new(RegisterMeasurements { name: String::from(NAME), registers: load_registers(store, start, end).await });
            anyhow::Ok(measurements)
        })
    }
}

/// The grid's import and export through the gateway's site meter.
pub struct TeslaElectricity {
    pub store: Arc<bruce_db::Store>,
}

/// The periods which can be chosen: none shorter than a minute, as the registers are read about every 10 seconds.
const PERIODS: [Period; 6] = [Period::Minute, Period::TenMinutes, Period::HalfHour, Period::Day, Period::Week, Period::Month];

impl ElectricitySource for TeslaElectricity {
    fn kind(&self) -> SourceKind {
        SourceKind::Device
    }

    /// The computer's timezone, as the gateway has none the app knows of.
    fn timezone(&self) -> &'static Tz {
        time_tz::system::get_timezone().unwrap_or(time_tz::timezones::db::UTC)
    }

    fn periods(&self) -> Vec<Period> {
        PERIODS.to_vec()
    }

    fn reload_title(&self) -> String {
        String::from("Read the gateway's saved readings again")
    }

    fn load(&self, start: DateTime, end: DateTime, _reload: Option<(DateTime, DateTime)>, _progress: Signal<String>,
        _cancel: CancelToken) -> Pin<Box<dyn Future<Output = LoadResult>>>
    {
        let store = self.store.clone();
        Box::pin(async move {
            let registers = load_registers(store, start.unix_timestamp() - 60, end.unix_timestamp() + 60).await;
            let loaded: Arc<dyn LoadedElectricity> =
                Arc::new(TeslaReadings { data: SiteData { grid: grid(&registers), grid_name: String::from(NAME), ..SiteData::default() } });
            anyhow::Ok(loaded)
        })
    }
}

/// The site meter's readings of a range.
struct TeslaReadings {
    data: SiteData,
}

impl LoadedElectricity for TeslaReadings {
    fn data(&self) -> &SiteData {
        &self.data
    }
}

/// The site meter's registers as grid readings, which the batteries' and the solar panels' registers split by use, see
/// TeslaMeasurements. The car's charger may be outside the site meter, so its import is not in them.
fn grid(registers: &[SiteRegisters]) -> Vec<GridReading> {
    registers.iter()
        .map(|registers| GridReading {
            read_at: registers.time as i64,
            demand: Some(registers.site_power),
            consumption_delta: None,
            consumption: Some(registers.site_imported),
            export: Some(registers.site_exported),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::combine::{Estimate, combine};

    #[test]
    fn the_site_meters_import_is_split_into_the_houses_and_the_powerwalls() {
        // The site importing 7 kW, of which the Powerwalls 5 kW, for an hour, the solar panels making nothing, with the
        // gateway at the grid connection, as there is nothing else to measure the grid.
        let start = 1_793_232_000;
        let registers = crate::power::measured::tests::registers(start - 20, start + 3620, 7000.0, 5000.0);
        let measurements: Vec<Arc<dyn Measurements>> = vec![Arc::new(RegisterMeasurements { name: String::from(NAME), registers: registers.clone() })];
        let mut topology = crate::power::topology::tests::beside_the_gateway();
        topology.children.remove(0);
        let split = crate::power::topology::TopologySplit { topology, measurements };
        let shown = combine(&[], &[], &grid(&registers), NAME, Some(&split), start, start + 3600, Period::HalfHour,
            &Estimate::default(), time_tz::timezones::db::UTC);
        assert_eq!(shown.labels, ["Home", "Battery"]);
        assert_eq!(shown.points.len(), 2);
        let point = &shown.points[0];
        assert!((point.import[0] - 1.0).abs() < 1e-3 && (point.import[1] - 2.5).abs() < 1e-3, "{:?}", point.import);
        assert_eq!(point.sources, [NAME]);
    }
}
