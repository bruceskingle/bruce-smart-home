//! Converting what the Octopus module kept in its directory of the cache before the store, see crate::convert: the
//! smart charging polls, `<day>#flex-HHMMSS`, the tariffs' history, `<account>#TariffHistory`, and the Home Mini's
//! telemetry, `<day>#<account>#<device>#Telemetry`. The module's DataSets import their own files the first time each is
//! used, see data_set::Records::import_legacy.

use std::path::Path;
use std::sync::Arc;

use bruce_db::{Item, Micros, Store, Version};

use crate::CacheConfig;

/// The account's number, from the tariffs' history's file name, or the cached viewer's first account.
fn account_number(dir: &Path) -> Option<String> {
    let from_history = std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        entry
            .file_name()
            .to_str()?
            .strip_suffix("#TariffHistory")
            .map(str::to_string)
    });
    from_history.or_else(|| {
        let viewer: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("#Viewer")).ok()?).ok()?;
        viewer["viewer"]["accounts"][0]["number"].as_str().map(str::to_string)
    })
}

/// A reading of telemetry as an older version of the app kept it, see TelemetryReading, as the server gives one, so
/// that it is read as one is, the server's decimals given as text, as it gives them.
fn telemetry_as_given(reading: &serde_json::Value) -> Option<serde_json::Value> {
    let read_at = Micros(reading.get("read_at")?.as_i64()? * 1_000_000).to_text();
    let decimal = |name: &str| {
        reading
            .get(name)
            .and_then(|value| value.as_f64())
            .map(|value| serde_json::Value::String(value.to_string()))
    };
    Some(serde_json::json!({
        "readAt": read_at,
        "demand": decimal("demand"),
        "consumptionDelta": decimal("consumption_delta"),
        "costDelta": decimal("cost_delta"),
        "costDeltaWithTax": decimal("cost_delta_with_tax"),
        "consumption": decimal("consumption"),
        "export": decimal("export"),
    }))
}

/// Converts the module's old files, see the module documentation.
pub fn convert_legacy(
    store: &Arc<Store>,
    dir: &Path,
    progress: &mut dyn bruce_db::progress::Progress,
) -> anyhow::Result<String> {
    let Some(account) = account_number(dir) else {
        return Ok(String::from("nothing, as the account is not known"));
    };
    let config = CacheConfig::new(dir.to_path_buf(), false).with_store(store.clone());
    let (mut polls, mut tariffs, mut readings) = (0, 0, 0);
    let files = crate::convert::legacy_files(dir);
    for (index, file) in files.iter().enumerate() {
        crate::convert::step(progress, index, files.len())?;
        if file.key.starts_with("flex-") {
            polls += super::flex_poll::save_to(store, &account, &file.json()?, None)? as usize;
        } else if file.key == "TariffHistory" {
            for record in file.lines()? {
                let (Some(seen), Some(meter), Some(id)) = (
                    record["seen"].as_str().and_then(|seen| Micros::parse(seen).ok()),
                    record["meter"].as_str(),
                    record["agreement"]["id"].as_i64(),
                ) else {
                    continue;
                };
                let hk = super::meter_agreement::history_key(store, &account, meter)?;
                store.put(&Item::new(
                    &hk,
                    &format!("{}#{id}", seen.to_text()),
                    Version::new(seen, &crate::store::origin()),
                    record,
                ))?;
                tariffs += 1;
            }
        } else if file.key.ends_with("#Telemetry") {
            let records = crate::data_set::Records::open(&config, &file.key)?;
            for reading in file.lines()? {
                if let (Some(read_at), Some(given)) = (reading["read_at"].as_i64(), telemetry_as_given(&reading)) {
                    records.put(&Micros(read_at * 1_000_000).to_text(), given)?;
                    readings += 1;
                }
            }
            if let Some(date) = file.date {
                records.put(&format!("#fetched/{date}"), serde_json::json!({}))?;
            }
        }
    }
    Ok(format!(
        "{polls} smart charging polls, {tariffs} tariffs, {readings} telemetry readings"
    ))
}
