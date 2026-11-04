/*
 * Page components.
 *
 * Each page which needs hooks is a real component so that its hooks are owned by that component
 * rather than by the enclosing Module component, which renders different pages at different times.
 */

mod bills;
mod account;
mod properties;
mod measurements;
mod charging_sessions;
mod ev_charging;
mod telemetry;
mod electricity;

pub use bills::BillsPage;
pub use account::AccountRow;
pub use properties::PropertiesPage;
pub use measurements::MeasurementsPage;
pub use ev_charging::EvChargingPage;
pub use electricity::OctopusElectricity;
