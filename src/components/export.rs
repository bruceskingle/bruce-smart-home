//! Export of tabular data to a file chosen with the system save dialog, as CSV or JSON.

use std::path::{Path, PathBuf};

use dioxus::prelude::*;
use dioxus_free_icons::{
    Icon,
    icons::ld_icons::{LdChevronDown, LdFileDown},
};
use serde_json::{Map, Number, Value};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use time_tz::{OffsetDateTimeExt, Tz};

/// The number of decimal places numbers are rounded to, to hide floating point noise from summing values.
const DECIMAL_PLACES: i32 = 6;

/// The file formats which a table can be exported as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExportFormat {
    Csv,
    Json,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 2] = [ExportFormat::Csv, ExportFormat::Json];

    pub fn name(&self) -> &'static str {
        match self {
            ExportFormat::Csv => "CSV",
            ExportFormat::Json => "JSON",
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
        }
    }
}

/// A value in an exported table.
#[derive(Clone, Debug, PartialEq)]
pub enum ExportValue {
    Text(String),
    Number(f64),
    /// A time, written in CSV as local time without the offset so that Excel recognises it, e.g. 2026-10-01 22:00:00,
    /// and in JSON as RFC 3339, e.g. 2026-10-01T22:00:00+01:00
    Time(OffsetDateTime),
    /// No value, an empty CSV field or a JSON null.
    Empty,
}

impl ExportValue {
    pub fn number(value: f64) -> Self {
        let factor = 10f64.powi(DECIMAL_PLACES);
        ExportValue::Number((value * factor).round() / factor)
    }

    /// A time, in the offset it already has.
    pub fn time(time: &OffsetDateTime) -> Self {
        ExportValue::Time(*time)
    }

    /// A unix timestamp as a time in the given timezone.
    pub fn unix_time(timestamp: i64, tz: &Tz) -> Self {
        match OffsetDateTime::from_unix_timestamp(timestamp) {
            Ok(time) => Self::time(&time.to_timezone(tz)),
            Err(_) => ExportValue::Empty,
        }
    }
}

/// Data to export, the column names and rows of values in the same order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportTable {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<ExportValue>>,
}

impl ExportTable {
    /// CSV with a header row.
    pub fn to_csv(&self) -> String {
        let mut csv = String::new();
        let mut write_line = |fields: Vec<String>| {
            csv.push_str(&fields.join(","));
            csv.push_str("\r\n");
        };

        write_line(self.columns.iter().map(|column| csv_field(column)).collect());
        for row in &self.rows {
            write_line(
                row.iter()
                    .map(|value| match value {
                        ExportValue::Text(text) => csv_field(text),
                        ExportValue::Number(number) => number.to_string(),
                        ExportValue::Time(time) => time.format(&csv_time_format()).unwrap_or_default(),
                        ExportValue::Empty => String::new(),
                    })
                    .collect(),
            );
        }
        csv
    }

    /// A JSON array with an object for each row, keyed by column name.
    pub fn to_json(&self) -> anyhow::Result<String> {
        let rows: Vec<Value> = self
            .rows
            .iter()
            .map(|row| {
                let object: Map<String, Value> = self
                    .columns
                    .iter()
                    .zip(row)
                    .map(|(column, value)| {
                        let value = match value {
                            ExportValue::Text(text) => Value::String(text.clone()),
                            ExportValue::Number(number) => {
                                Number::from_f64(*number).map(Value::Number).unwrap_or(Value::Null)
                            }
                            ExportValue::Time(time) => time.format(&Rfc3339).map(Value::String).unwrap_or(Value::Null),
                            ExportValue::Empty => Value::Null,
                        };
                        (column.clone(), value)
                    })
                    .collect();
                Value::Object(object)
            })
            .collect();
        Ok(serde_json::to_string_pretty(&rows)?)
    }

    pub fn write(&self, path: &Path, format: ExportFormat) -> anyhow::Result<()> {
        let contents = match format {
            ExportFormat::Csv => self.to_csv(),
            ExportFormat::Json => self.to_json()?,
        };
        std::fs::write(path, contents)?;
        Ok(())
    }
}

/// The format of times in CSV, which Excel recognises as a date and time. There is no offset, so a time in the hour
/// repeated when the clocks go back is ambiguous.
fn csv_time_format() -> Vec<time::format_description::BorrowedFormatItem<'static>> {
    time::format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]").unwrap()
}

/// Quote a CSV field if it contains a comma, quote or line break.
fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// The suggested name for an export, without an extension, e.g. measurements_1012779334755_2026-10-01_to_2026-10-31_rate
/// The end is exclusive, so the last day in the name is the day before it if it is midnight.
pub fn export_file_name(prefix: &str, id: &str, start: &OffsetDateTime, end: &OffsetDateTime, what: &str) -> String {
    let date_format = time::format_description::parse("[year]-[month]-[day]").unwrap();
    let last = *end - time::Duration::SECOND;
    format!(
        "{prefix}_{id}_{}_to_{}_{what}",
        start.format(&date_format).unwrap_or_default(),
        last.format(&date_format).unwrap_or_default()
    )
}

/// Ask the user where to save the table with the system save dialog and write it there in the given format.
/// Returns the path written, or None if the user cancelled.
pub async fn save_table(
    table: &ExportTable,
    default_name: &str,
    format: ExportFormat,
) -> anyhow::Result<Option<PathBuf>> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title(format!("Export data as {}", format.name()))
        .set_file_name(format!("{default_name}.{}", format.extension()))
        .add_filter(format.name(), &[format.extension()])
        .set_can_create_directories(true)
        .save_file()
        .await
    else {
        return Ok(None);
    };

    let mut path = file.path().to_path_buf();
    if path.extension().is_none() {
        path.set_extension(format.extension());
    }
    table.write(&path, format)?;
    Ok(Some(path))
}

/// The result of the last export, shown next to the export button.
#[derive(Clone, PartialEq)]
enum ExportStatus {
    /// The file name written, and whether it has been copied to the clipboard.
    Exported {
        file_name: String,
        copied: bool,
    },
    Failed(String),
}

/// Copy text to the system clipboard.
fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    arboard::Clipboard::new()?.set_text(text)?;
    Ok(())
}

/// Build the table and save it in the background, putting the result of the export in status.
fn start_export(
    table: Callback<(), Option<ExportTable>>,
    default_name: String,
    format: ExportFormat,
    mut status: Signal<Option<ExportStatus>>,
) {
    let Some(table) = table.call(()) else {
        return;
    };
    spawn(async move {
        let result = match save_table(&table, &default_name, format).await {
            Ok(Some(path)) => Some(ExportStatus::Exported {
                file_name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default(),
                copied: false,
            }),
            Ok(None) => None,
            Err(error) => Some(ExportStatus::Failed(format!("Export failed: {error}"))),
        };
        status.set(result);
    });
}

/// A button with a menu of formats which exports the table it is given in the chosen format. The table is only built
/// when a format is chosen, None means there is nothing to export.
#[component]
pub fn ExportButton(
    /// The suggested file name, without an extension.
    default_name: String,
    disabled: bool,
    table: Callback<(), Option<ExportTable>>,
) -> Element {
    // The result of the last export.
    let mut status = use_signal(|| None::<ExportStatus>);
    let mut menu_open = use_signal(|| false);

    rsx! {
        span { class: "export",
            match status() {
                Some(ExportStatus::Exported { file_name, copied }) => rsx! {
                    span { class: "export-status",
                        "Exported "
                        span {
                            class: "export-file-name",
                            title: "Copy the file name",
                            onclick: {
                                let file_name = file_name.clone();
                                move |_| {
                                    status.set(Some(match copy_to_clipboard(&file_name) {
                                        Ok(()) => ExportStatus::Exported { file_name: file_name.clone(), copied: true },
                                        Err(error) => ExportStatus::Failed(format!("Copy failed: {error}")),
                                    }));
                                }
                            },
                            "{file_name}"
                        }
                        if copied {
                            " (copied)"
                        }
                    }
                },
                Some(ExportStatus::Failed(message)) => rsx! {
                    span { class: "export-status", "{message}" }
                },
                None => rsx! {},
            }
            span { class: "menu-anchor",
                button {
                    r#type: "button",
                    title: "Export the data in this view",
                    disabled,
                    onclick: move |_| menu_open.set(!menu_open()),
                    Icon {
                        class: "inline-icon",
                        icon: LdFileDown,
                        width: 14,
                        height: 14,
                    }
                    " Export "
                    Icon {
                        class: "inline-icon",
                        icon: LdChevronDown,
                        width: 14,
                        height: 14,
                    }
                }
                if menu_open() && !disabled {
                    // Covers the page so that clicking anywhere else closes the menu.
                    div { class: "menu-backdrop", onclick: move |_| menu_open.set(false) }
                    div { class: "dropdown-menu",
                        for format in ExportFormat::ALL {
                            button {
                                r#type: "button",
                                onclick: {
                                    let default_name = default_name.clone();
                                    move |_| {
                                        menu_open.set(false);
                                        start_export(table, default_name.clone(), format, status);
                                    }
                                },
                                "{format.name()}"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ExportTable, ExportValue};

    fn sample() -> ExportTable {
        ExportTable {
            columns: vec![String::from("Start"), String::from("Units, kwh")],
            rows: vec![
                vec![
                    ExportValue::time(&time::macros::datetime!(2026-10-01 00:00 +1)),
                    ExportValue::number(0.1 + 0.2),
                ],
                vec![ExportValue::Text(String::from("say \"hi\"")), ExportValue::Empty],
            ],
        }
    }

    #[test]
    fn file_name_has_underscore_separators() {
        let start = time::macros::datetime!(2026-10-01 00:00 +1);
        let end = time::macros::datetime!(2026-11-01 00:00 +0);
        assert_eq!(
            super::export_file_name("measurements", "1012779334755", &start, &end, "rate"),
            "measurements_1012779334755_2026-10-01_to_2026-10-31_rate"
        );
    }

    #[test]
    fn csv_quotes_fields_which_need_it() {
        assert_eq!(
            sample().to_csv(),
            "Start,\"Units, kwh\"\r\n2026-10-01 00:00:00,0.3\r\n\"say \"\"hi\"\"\",\r\n"
        );
    }

    #[test]
    fn json_has_an_object_per_row() {
        let json: serde_json::Value = serde_json::from_str(&sample().to_json().unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!([
                { "Start": "2026-10-01T00:00:00+01:00", "Units, kwh": 0.3 },
                { "Start": "say \"hi\"", "Units, kwh": null },
            ])
        );
    }
}
