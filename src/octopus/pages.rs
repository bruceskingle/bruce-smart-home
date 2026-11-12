/*
 * Page components.
 *
 * Each page which needs hooks is a real component so that its hooks are owned by that component
 * rather than by the enclosing Module component, which renders different pages at different times.
 */

mod account;
mod bills;
mod charging_sessions;
mod electricity;
mod ev_charging;
mod measurements;
mod properties;
mod telemetry;

pub use account::AccountRow;
pub use bills::BillsPage;
pub use electricity::OctopusElectricity;
pub use ev_charging::EvChargingPage;
pub use measurements::MeasurementsPage;
pub use properties::PropertiesPage;
