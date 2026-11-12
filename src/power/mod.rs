//! A site's electricity, the same whoever supplies it and whatever measures it, so that one Electricity page,
//! views/electricity.rs, shows any module's, see docs/PowerSchema.md.
//!
//! A module gives an ElectricitySource, which loads what it has for a range of time, e.g. Octopus's meters, Home Mini
//! and the Tesla gateway's registers, see octopus/electricity.rs, or GloBird's 5 minute readings, and what it loaded
//! makes the Electricity of the range in periods of any length. Import is split into labels, e.g. Home Low Rate or Peak
//! Usage, which are the rates it is billed at, and for Octopus what it was used for, e.g. EV Low Rate.

pub mod check;
pub mod combine;
pub mod flows;
pub mod grid;
pub mod live;
pub mod measured;
pub mod topology;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bruce_graphql::types::{Currency, DateTime};
use dioxus::prelude::*;
use time_tz::Tz;

use crate::components::error_text::ErrorText;
use crate::data_set::CancelToken;
use crate::period::Period;
use measured::Split;

pub const EXPORT_LABEL: &str = "Export";
/// Octopus's EV share, as a dashed line over the bars, and the measured one to compare it with, as a solid line, each
/// as much every second of a half hour, see EvComparison.
pub const OCTOPUS_EV_LABEL: &str = "Octopus's EV split";
pub const MEASURED_EV_LABEL: &str = "Measured EV, half hourly";

/// The electricity from start to end, as unix timestamps, in periods of period, put together from what the modules
/// gave, see combine::combine: the meters' readings of the first which has any, e.g. a supplier's, the grid readings of
/// the first which has any, and the import split by use by split, if there is one, see topology::TopologySplit.
pub fn electricity(
    loaded: &[&dyn LoadedElectricity],
    measurements: Option<&dyn Split>,
    start: i64,
    end: i64,
    period: Period,
    tz: &'static Tz,
) -> Electricity {
    let empty = SiteData::default();
    let meters = loaded
        .iter()
        .map(|loaded| loaded.data())
        .find(|data| !data.import.is_empty() || !data.export.is_empty())
        .unwrap_or(&empty);
    let grid = loaded
        .iter()
        .map(|loaded| loaded.data())
        .find(|data| !data.grid.is_empty())
        .unwrap_or(&empty);
    let electricity = combine::combine(
        &meters.import,
        &meters.export,
        &grid.grid,
        &grid.grid_name,
        measurements,
        start,
        end,
        period,
        &meters.estimate,
        tz,
    );
    Electricity {
        currency: meters.currency,
        ..electricity
    }
}

/// The app's own labels, with their default colours, see crate::known_labels.
pub const LABELS: [(&str, &str); 2] = [(EXPORT_LABEL, "#f59e0b"), (OCTOPUS_EV_LABEL, "#ffffff")];

/// The electricity in a period, in kWh, with its start and end as unix timestamps.
#[derive(Clone, Debug, PartialEq)]
pub struct ElectricityPoint {
    pub start: i64,
    pub end: i64,
    /// The energy imported, for each of Electricity::labels.
    pub import: Vec<f64>,
    pub export: f64,
    /// The cost of the import of each of Electricity::labels, of the export, negative for a credit, and the standing
    /// charge, in the minor units of Electricity::currency, see combine::MeterReading, zero where it is not known, e.g.
    /// for import the meter has no reading for yet.
    pub costs: Vec<f64>,
    pub export_cost: f64,
    pub standing_charge: f64,
    /// The Home Mini's average power, in W, net of export, for periods shorter than a half hour from its readings,
    /// which is steadier than their energy, as the readings are not exactly 10 seconds apart.
    pub demand: Option<f64>,
    /// Where it is from, in the order they are shown in, e.g. Home Mini and Tesla.
    pub sources: Vec<String>,
    /// The car's import measured, and as Octopus split it, over the half hours which have both, for comparing.
    pub ev_comparison: Option<EvComparison>,
}

/// The car's import, in kWh, measured and as Octopus split it, each as much every second of a half hour, as Octopus only
/// has the half hour, so that the two can be compared however short the periods are, see ElectricityPoint.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvComparison {
    pub measured: f64,
    pub octopus: f64,
    /// The same, of the half hours outside the tariff's cheap times, where the split decides the rate: the car's import
    /// is at the low rate in a dispatch there, and the rest at the standard rate. Zero if the cheap times are not known.
    pub measured_outside: f64,
    pub octopus_outside: f64,
}

impl std::ops::Add for EvComparison {
    type Output = EvComparison;

    fn add(self, other: EvComparison) -> EvComparison {
        EvComparison {
            measured: self.measured + other.measured,
            octopus: self.octopus + other.octopus,
            measured_outside: self.measured_outside + other.measured_outside,
            octopus_outside: self.octopus_outside + other.octopus_outside,
        }
    }
}

/// The electricity in each period which has any, in time order, with the labels its import is split into.
#[derive(Clone, Debug, PartialEq)]
pub struct Electricity {
    pub labels: Vec<String>,
    /// The use each of labels is of, a topology category, e.g. Car for EV Low Rate, "" where it is not known, e.g. a
    /// supplier's rate not split by use.
    pub uses: Vec<String>,
    pub points: Vec<ElectricityPoint>,
    /// The currency of the points' costs, None if the meters' readings have none.
    pub currency: Option<Currency>,
}

/// What a load of an ElectricitySource gives.
pub type LoadResult = anyhow::Result<Arc<dyn LoadedElectricity>>;

/// What a source of a site's electricity is, which Home's Electricity page chooses by, see views/electricity.rs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceKind {
    /// A supplier's meters, which say what is billed, e.g. Octopus's or GloBird's, chosen first.
    Supplier,
    /// A device on the site, e.g. a Tesla gateway, which measures the grid too, chosen if there is no supplier.
    Device,
}

/// Where a site's electricity comes from, for the Electricity page, see views/electricity.rs.
pub trait ElectricitySource {
    /// Whether it is a supplier's or a device's.
    fn kind(&self) -> SourceKind;

    /// The timezone of the site, for its days and the times shown.
    fn timezone(&self) -> &'static Tz;

    /// The periods which can be chosen, shortest first, e.g. not 10 seconds for readings every 5 minutes.
    fn periods(&self) -> Vec<Period> {
        Period::ALL.to_vec()
    }

    /// The period of the readings as the source gives them, e.g. GloBird's 5 minutes, at which the page of one source
    /// shows them as they are, rather than added up, see ElectricityExplorer. None if they are not known to be as given,
    /// in which case none of its table is shown as derived.
    fn reading_period(&self) -> Option<Period> {
        None
    }

    /// Loads what there is from start to end, fetching what is not cached, and fetching reload again however up to date
    /// it is, saying what it is doing with progress, and stopping if cancel is cancelled.
    fn load(
        &self,
        start: DateTime,
        end: DateTime,
        reload: Option<(DateTime, DateTime)>,
        progress: Signal<String>,
        cancel: CancelToken,
    ) -> Pin<Box<dyn Future<Output = LoadResult>>>;

    /// Whether the points may have Octopus's split of the car's import to compare, see ElectricityPoint::ev_comparison,
    /// for which the table has a column.
    fn compares_ev(&self) -> bool {
        false
    }

    /// What the Reload button's tooltip says it does.
    fn reload_title(&self) -> String;

    /// What is shown if load failed.
    fn error(&self, error: &anyhow::Error) -> Element {
        rsx! { div { class: "error", ErrorText { error: format!("{error:#}"), message: "Failed to load the readings" } } }
    }
}

/// What a module gives the Electricity page of a range, which the app puts together with other modules', see
/// electricity.
#[derive(Clone, Debug, Default)]
pub struct SiteData {
    /// A supplier's meters' readings of the import, by label, e.g. Home Low Rate, and of the export.
    pub import: Vec<combine::MeterReading>,
    pub export: Vec<combine::MeterReading>,
    /// A grid meter's readings every few seconds, e.g. the Octopus Home Mini's, and its name, as the Electricity page
    /// shows where a period is from.
    pub grid: Vec<grid::GridReading>,
    pub grid_name: String,
    /// What is known for estimating the rate of import the meters have no reading for yet.
    pub estimate: combine::Estimate,
    /// The currency of the meters' readings' costs, see combine::MeterReading, None if they have none.
    pub currency: Option<Currency>,
}

/// What an ElectricitySource loaded.
pub trait LoadedElectricity {
    /// What the module has of the range.
    fn data(&self) -> &SiteData;

    /// What is shown under the graph or table, e.g. Octopus's comparison of the car's import, nothing by default.
    fn notes(&self, _electricity: &Electricity) -> Element {
        rsx! {}
    }

    /// A warning that part of it could not be loaded, e.g. the Home Mini's readings, as the rest is still worth showing.
    fn warning(&self) -> Option<String> {
        None
    }
}
