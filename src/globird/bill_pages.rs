//! The Bills page, GloBird's bills, read from their PDFs, see bills.rs, each beside what the cached days add up to
//! over its period, the page of one bill, its charges beside the days', and a window of a bill's PDF, see
//! open_pdf.

use std::collections::BTreeMap;

use dioxus::prelude::*;
use time::Date;

use crate::components::error_text::ErrorText;
use crate::util::SharedProp;

use super::GlobirdContext;
use super::bills::{self, Bill, BillLine};
use super::fetch::{self, DaySummary};
use super::rates::{SOLAR, SUPPLY, USAGE};

/// What the cached days add up to over a bill's period, to set beside it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Comparison {
    /// How many days the bill is for.
    pub days: usize,
    /// How many of them have costs cached.
    pub days_cached: usize,
    /// For each of the bill's lines, in order: the meter's kWh of its charge type, or the days, for the daily charge,
    /// None for a line the days have nothing for.
    pub lines: Vec<Option<f64>>,
    /// For each category, e.g. USAGE: what the bill's lines of it add up to, and the days' costs of it.
    pub categories: BTreeMap<String, (f64, f64)>,
    /// What the days' costs add up to.
    pub total: f64,
}

impl Comparison {
    /// Whether every day of the bill is cached, without which the totals are not comparable.
    pub fn complete(&self) -> bool {
        self.days > 0 && self.days_cached == self.days
    }
}

/// The category of the days' costs which a line of a bill is, e.g. USAGE for Peak Usage, an import charge type of the
/// days, None if it is none of them, e.g. a credit.
fn category(line: &BillLine, days: &[&DaySummary]) -> Option<&'static str> {
    if line.is_daily() {
        Some(SUPPLY)
    } else if days.iter().any(|day| day.import.contains_key(&line.description)) {
        Some(USAGE)
    } else if days
        .iter()
        .any(|day| day.export_by_type.contains_key(&line.description))
    {
        Some(SOLAR)
    } else {
        None
    }
}

/// The cached days over the bill's period, see Comparison, None if it has no period.
pub fn compare(bill: &Bill, summaries: &[DaySummary]) -> Option<Comparison> {
    let (first, last) = bill.period?;
    let days: Vec<&DaySummary> = summaries
        .iter()
        .filter(|day| day.date.is_some_and(|date| first <= date && date <= last) && !day.cost.is_empty())
        .collect();
    let mut comparison = Comparison {
        days: ((last - first).whole_days() + 1).max(0) as usize,
        days_cached: days.len(),
        total: days.iter().map(|day| day.total_cost()).sum(),
        ..Default::default()
    };
    for line in &bill.lines {
        let category = category(line, &days);
        comparison.lines.push(match category {
            Some(SUPPLY) => Some(days.iter().filter(|day| day.cost.contains_key(SUPPLY)).count() as f64),
            Some(USAGE) => Some(days.iter().filter_map(|day| day.import.get(&line.description)).sum()),
            Some(SOLAR) => Some(
                days.iter()
                    .filter_map(|day| day.export_by_type.get(&line.description))
                    .sum(),
            ),
            _ => None,
        });
        if let Some(category) = category {
            comparison.categories.entry(category.to_string()).or_default().0 += line.amount;
        }
    }
    for category in comparison.categories.clone().keys() {
        let cost = days.iter().filter_map(|day| day.cost.get(category)).sum();
        comparison.categories.entry(category.clone()).or_default().1 = cost;
    }
    Some(comparison)
}

/// The name of the asset handler which gives a PDF window's web view the PDF, see BillPdfWindow.
const PDF_HANDLER: &str = "globird-bill";

/// Opens the cached PDF of a bill in a window of the app's own, see child_windows, or the one already open, shown by
/// the web view's own PDF viewer, which WebKit on macOS and WebView2 on Windows have. WebKitGTK on Linux has none, so
/// there it is written to a private temporary file and opened in the system's PDF viewer instead.
fn open_pdf(cache_config: &crate::CacheConfig, title: String, document_id: i64) {
    let pdf = match bills::document(cache_config, document_id) {
        Ok(pdf) => pdf,
        Err(error) => {
            crate::log_warning!("Unable to open the bill's PDF, {error:#}");
            return;
        }
    };
    if cfg!(target_os = "linux") {
        let path = std::env::temp_dir().join(format!("globird-bill-{document_id}.pdf"));
        let written = crate::private_file::create_private_file(&path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, &pdf));
        if let Err(error) = written
            .map_err(anyhow::Error::from)
            .and_then(|_| Ok(webbrowser::open(&format!("file://{}", path.display()))?))
        {
            crate::log_warning!("Unable to open the bill's PDF, {error:#}");
        }
        return;
    }
    crate::components::child_windows::open_shaped(
        &format!("globird-bill-{document_id}"),
        title,
        None,
        Some(pdf_window_width),
        || {
            VirtualDom::new_with_props(
                BillPdfWindow,
                BillPdfWindowProps {
                    pdf: SharedProp(std::sync::Arc::new(pdf)),
                },
            )
        },
    );
}

/// The width of a bill's PDF window for its height, at which exactly one page shows down the window: the viewer fits
/// the page to the window's width, less its margins, MARGIN_ACROSS in all, and shows it MARGIN_ABOVE below the top, so
/// the page is the window's height less that above it and SPACE_BELOW below, which is less than the gap between pages,
/// about 17, so that none of the next page shows, and a bill is an A4 page, 210 by 297 mm. The margins are WebKit's on
/// macOS, measured from screenshots, including the scrollbar's width.
fn pdf_window_width(height: f64) -> f64 {
    const MARGIN_ABOVE: f64 = 20.0;
    const SPACE_BELOW: f64 = 10.0;
    const MARGIN_ACROSS: f64 = 49.0;
    (height - MARGIN_ABOVE - SPACE_BELOW) * 210.0 / 297.0 + MARGIN_ACROSS
}

/// The root of a bill's PDF window, the web view's PDF viewer filling it, given the PDF, as it was cached, by an asset
/// handler of the window, as it can not read the store itself. Escape closes it.
#[component]
fn BillPdfWindow(pdf: SharedProp<Vec<u8>>) -> Element {
    crate::components::child_windows::use_child_window();
    dioxus::desktop::use_asset_handler(PDF_HANDLER, move |_request, responder| {
        let response = dioxus::desktop::wry::http::Response::builder()
            .header("Content-Type", "application/pdf")
            .body(pdf.0.as_ref().clone());
        match response {
            Ok(response) => responder.respond(response),
            Err(error) => crate::log_warning!("Unable to give the bill's PDF, {error:#}"),
        }
    });
    use_future(|| async {
        let mut keys = document::eval(
            "document.addEventListener('keydown', (event) => { if (event.key === 'Escape') { dioxus.send(true); } });",
        );
        while keys.recv::<bool>().await.is_ok() {
            dioxus::desktop::window().close();
        }
    });
    rsx! {
        // The window's head hides the page until the stylesheet has loaded, see app.rs INITIAL_HEAD.
        document::Link { rel: "stylesheet", href: crate::components::app::MAIN_CSS }
        embed {
            src: "/{PDF_HANDLER}/bill.pdf",
            r#type: "application/pdf",
            style: "position: fixed; inset: 0; width: 100%; height: 100%; border: none;",
        }
    }
}

/// The title of a bill's PDF window, e.g. GloBird bill 12361365, 2026-09-28 to 2026-10-25.
fn pdf_title(bill: &Bill, name: &str) -> String {
    let invoice = bill.invoice.clone().unwrap_or_else(|| name.to_string());
    match bill.period {
        Some(_) => format!("GloBird bill {invoice}, {}", period_text(bill.period)),
        None => format!("GloBird bill {invoice}"),
    }
}

/// A bill's row on the Bills page: the email it came in, and the bill, if its PDF is cached and could be read.
#[derive(Clone, PartialEq)]
struct BillRow {
    email: bills::ListItem,
    document: Option<bills::Document>,
    bill: Option<Result<Bill, String>>,
}

/// The rows of the cached bills, newest first, a row for each bill's email whose documents are not cached yet.
fn bill_rows(context: &GlobirdContext, cached: &bills::Cached) -> Vec<BillRow> {
    let mut rows = Vec::new();
    for email in cached.bills() {
        let documents = cached
            .details
            .get(&email.id)
            .map(|detail| detail.document_list.clone())
            .unwrap_or_default();
        if documents.is_empty() {
            rows.push(BillRow {
                email: email.clone(),
                document: None,
                bill: None,
            });
        }
        for document in documents {
            let bill = cached
                .documents
                .contains_key(&document.document_id)
                .then(|| context.bill(document.document_id));
            rows.push(BillRow {
                email: email.clone(),
                document: Some(document),
                bill,
            });
        }
    }
    rows
}

fn dollars(amount: f64) -> String {
    format!("{amount:.2}")
}

fn period_text(period: Option<(Date, Date)>) -> String {
    period
        .map(|(first, last)| format!("{first} to {last}"))
        .unwrap_or_default()
}

/// The bills, and Fetch, which makes one request each time, see bills::next_request.
#[component]
pub fn BillsPage(context: SharedProp<GlobirdContext>) -> Element {
    let mut generation = use_signal(|| 0u32);
    let mut message = use_signal(String::new);
    let mut busy = use_signal(|| false);
    generation();
    let cache_config = context.0.cache_config.clone();
    let cached = bills::cached(&cache_config);
    let account_cached = fetch::account(&cache_config).is_some();
    let next = bills::next_request(&cached, account_cached).map(|request| bills::describe(&request, &cached));
    let rows = bill_rows(&context.0, &cached);
    let summaries = fetch::summaries(&cache_config);
    let fetch = move |_| {
        let context = context.clone();
        spawn(async move {
            busy.set(true);
            message.set(match context.0.fetch_bills().await {
                Ok(Some(fetched)) => format!("Fetched {fetched}"),
                Ok(None) => String::from("There was nothing more to fetch"),
                Err(error) => format!("Failed: {error:#}"),
            });
            busy.set(false);
            generation += 1;
        });
    };

    rsx! {
        h1 { "GloBird Bills" }
        p {
            "The bills GloBird has emailed, read from their PDFs, each beside what the cached days' costs add up to over its period, which are in pink, as numbers the app works out are. Click a bill to see its charges. Each Fetch makes one request to GloBird, the next which is needed, the newest bill first, so that a long history is fetched a little at a time; press it again for more. A bill never changes once it is issued, so each is fetched once."
        }
        div { style: "margin-bottom: 1em;",
            button { disabled: busy() || next.is_none(), onclick: fetch, if busy() { "Fetching..." } else { "Fetch" } }
            match &next {
                Some(next) => rsx! { " Next: {next}" },
                None => rsx! { " Everything is fetched" },
            }
        }
        if !message().is_empty() {
            p { "{message}" }
        }
        if let Some(total) = cached.total_count {
            p { "{cached.items.len()} of GloBird's {total} emails are listed, of which {cached.bills().count()} are bills." }
        }
        table { class: "display",
            tr {
                th { "Emailed" }
                th { "Invoice" }
                th { "Period" }
                th { "Total $" }
                th { "Days' total $" }
                th { "Difference $" }
                th { "" }
            }
            for row in rows {
                BillRowView { row, summaries: summaries.clone(), cache_config: SharedProp(cache_config.clone()) }
            }
        }
    }
}

#[component]
fn BillRowView(row: BillRow, summaries: Vec<DaySummary>, cache_config: SharedProp<crate::CacheConfig>) -> Element {
    let emailed = row.email.date().map(|date| date.to_string()).unwrap_or_default();
    let reversed = row
        .document
        .as_ref()
        .is_some_and(|document| document.is_reversed == Some(true));
    let name = row
        .document
        .as_ref()
        .and_then(|document| document.name.clone())
        .unwrap_or_default();
    let (bill, error) = match &row.bill {
        Some(Ok(bill)) => (Some(bill.clone()), None),
        Some(Err(error)) => (None, Some(error.clone())),
        None => (None, None),
    };
    let comparison = bill.as_ref().and_then(|bill| compare(bill, &summaries));
    let status = match (&row.document, &row.bill) {
        (None, _) => String::from("Fetch to get its email"),
        (Some(_), None) => String::from("Fetch to get its PDF"),
        _ if reversed => String::from("Reversed"),
        _ => match &comparison {
            Some(comparison) if !comparison.complete() => {
                format!("{} of its {} days are cached", comparison.days_cached, comparison.days)
            }
            _ => String::new(),
        },
    };
    let document_id = row.document.as_ref().map(|document| document.document_id);
    let pdf = match (&row.document, &row.bill) {
        (Some(document), Some(_)) => Some((
            cache_config.clone(),
            bill.as_ref()
                .map(|bill| pdf_title(bill, &name))
                .unwrap_or_else(|| format!("GloBird bill {name}")),
            document.document_id,
        )),
        _ => None,
    };
    rsx! {
        tr {
            td { "{emailed}" }
            td {
                if let (Some(document_id), Some(_)) = (document_id, &bill) {
                    div {
                        class: "link",
                        onclick: move |_| {
                            let mut path_signal = use_context::<Signal<Vec<String>>>();
                            path_signal.set(vec![String::from("bills"), document_id.to_string()]);
                        },
                        {bill.as_ref().and_then(|bill| bill.invoice.clone()).unwrap_or(name.clone())}
                    }
                }
                else {
                    "{name}"
                }
            }
            td { {period_text(bill.as_ref().and_then(|bill| bill.period))} }
            td { class: "number", {bill.as_ref().and_then(|bill| bill.total).map(dollars).unwrap_or_default()} }
            if let (Some(bill), Some(comparison)) = (&bill, comparison.as_ref().filter(|comparison| comparison.complete())) {
                td { class: "number derived", {dollars(comparison.total)} }
                td { class: "number derived", {bill.total.map(|total| format!("{:+.2}", comparison.total - total)).unwrap_or_default()} }
            }
            else {
                td {}
                td {}
            }
            td {
                if let Some((cache_config, title, document_id)) = pdf {
                    button { onclick: move |_| open_pdf(&cache_config.0, title.clone(), document_id), "PDF" }
                    " "
                }
                if let Some(error) = error {
                    span { class: "error", ErrorText { error, message: String::from("Unable to read the bill") } }
                }
                "{status}"
            }
        }
    }
}

/// One bill: its charges, as it says them, beside the days', and its PDF.
#[component]
pub fn BillPage(context: SharedProp<GlobirdContext>, document_id: i64) -> Element {
    let cache_config = context.0.cache_config.clone();
    let bill = match context.0.bill(document_id) {
        Ok(bill) => bill,
        Err(error) => {
            return rsx! { div { class: "error", ErrorText { error, message: String::from("Unable to read the bill") } } };
        }
    };
    let summaries = fetch::summaries(&context.0.cache_config);
    let comparison = compare(&bill, &summaries).unwrap_or_default();
    let title = pdf_title(&bill, &document_id.to_string());
    let open = move |_| open_pdf(&cache_config, title.clone(), document_id);
    let unit_rate = |line: &BillLine| {
        if line.per_day() {
            format!("{:.5}", line.rate)
        } else {
            format!("{:.3}", line.rate * 100.0)
        }
    };
    let ours = |index: usize, line: &BillLine| {
        comparison
            .lines
            .get(index)
            .copied()
            .flatten()
            .map(|value| {
                if line.is_daily() {
                    format!("{value:.0}")
                } else {
                    format!("{value:.2}")
                }
            })
            .unwrap_or_default()
    };
    rsx! {
        h1 { "GloBird Bill {bill.invoice.clone().unwrap_or_default()}" }
        p {
            "For {period_text(bill.period)}"
            if let Some(plan) = &bill.plan { ", on the {plan} plan" }
            if let Some(issued) = bill.issued { ", issued {issued}" }
            if let Some(due) = bill.due { ", due {due}" }
            ". "
            button { onclick: open, "Open the PDF" }
        }
        if !comparison.complete() {
            p { "{comparison.days_cached} of the bill's {comparison.days} days are cached, so the days' figures, in pink, are not all of its period." }
        }
        h2 { "Charges" }
        p { "Each charge as the bill says it, beside the meter's kWh of its charge type over the bill's days, from the Days page, or, for the daily charge, the days whose costs have one. Rates are in c/kWh, and $ a day for the daily charge." }
        table { class: "display",
            tr {
                th { "Charge" }
                th { "Quantity" }
                th { "Unit" }
                th { "Rate" }
                th { "$" }
                th { "The days' quantity" }
            }
            for (index, line) in bill.lines.iter().enumerate() {
                tr {
                    td { "{line.description}" }
                    td { class: "number", {format!("{}", line.quantity)} }
                    td { "{line.unit}" }
                    td { class: "number", {unit_rate(line)} }
                    td { class: "number", {dollars(line.amount)} }
                    td { class: "number derived", {ours(index, line)} }
                }
            }
            tr {
                td { "Total, including GST of {bill.gst.map(dollars).unwrap_or_default()}" }
                td {}
                td {}
                td {}
                td { class: "number", {bill.total.map(dollars).unwrap_or_default()} }
                td {}
            }
        }
        h2 { "By category" }
        p { "The bill's charges added up by the categories of the days' costs, e.g. USAGE for Peak and Offpeak Usage, beside what the days' costs of each add up to." }
        table { class: "display",
            tr {
                th { "Category" }
                th { "The bill's $" }
                th { "The days' $" }
                th { "Difference $" }
            }
            for (category, (billed, days)) in comparison.categories.iter() {
                tr {
                    td { "{category}" }
                    td { class: "number derived", {dollars(*billed)} }
                    td { class: "number derived", {dollars(*days)} }
                    td { class: "number derived", {format!("{:+.2}", days - billed)} }
                }
            }
            tr {
                td { "Total" }
                td { class: "number", {bill.total.map(dollars).unwrap_or_default()} }
                td { class: "number derived", {dollars(comparison.total)} }
                td { class: "number derived", {bill.total.map(|total| format!("{:+.2}", comparison.total - total)).unwrap_or_default()} }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(day: u8, import: &[(&str, f64)], export: f64, cost: &[(&str, f64)]) -> DaySummary {
        DaySummary {
            date: Some(Date::from_calendar_date(2027, time::Month::January, day).unwrap()),
            import: import.iter().map(|(band, kwh)| (band.to_string(), *kwh)).collect(),
            export,
            export_by_type: BTreeMap::from([(String::from("Standard Solar Feed in"), export)]),
            cost: cost
                .iter()
                .map(|(category, amount)| (category.to_string(), *amount))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_bill_is_set_beside_the_days_of_its_period() {
        let bill = bills::parse_bill(
            "Billing Period: 01-Jan-2027 to 02-Jan-2027 (2 days)
Daily Charge 01-Jan - 02-Jan 2 Days x $1.00000 $2.00
Peak Usage 01-Jan - 02-Jan 3.00 kWh x $0.40000 $1.20
Offpeak Usage 01-Jan - 02-Jan 10.00 kWh x $0.20000 $2.00
Standard Solar Feed in 01-Jan - 02-Jan 4.00 kWh x -$0.05000 -$0.20
Late Payment Credit 01-Jan - 02-Jan 1 Days x -$0.50000 -$0.50
TOTAL (including $0.41 GST) $4.50
",
        );
        let days = [
            day(
                1,
                &[("Peak Usage", 1.0), ("Offpeak Usage", 6.0)],
                3.0,
                &[(USAGE, 1.6), (SOLAR, -0.15), (SUPPLY, 1.0)],
            ),
            day(
                2,
                &[("Peak Usage", 2.0), ("Offpeak Usage", 4.0)],
                1.0,
                &[(USAGE, 1.6), (SOLAR, -0.05), (SUPPLY, 1.0)],
            ),
            // Outside the bill's period.
            day(3, &[("Peak Usage", 9.0)], 9.0, &[(USAGE, 9.0)]),
        ];
        let comparison = compare(&bill, &days).unwrap();
        assert_eq!((comparison.days, comparison.days_cached), (2, 2));
        assert!(comparison.complete());
        assert_eq!(
            comparison.lines,
            vec![Some(2.0), Some(3.0), Some(10.0), Some(4.0), None],
            "a credit of so much a day is not the daily charge"
        );
        assert_eq!(comparison.categories.get(SUPPLY), Some(&(2.0, 2.0)));
        assert_eq!(comparison.categories.get(USAGE), Some(&(3.2, 3.2)));
        assert_eq!(comparison.categories.get(SOLAR), Some(&(-0.2, -0.2)));
        assert!((comparison.total - 5.0).abs() < 1e-9);
    }

    #[test]
    fn a_bill_whose_days_are_not_all_cached_is_not_complete() {
        let bill = Bill {
            period: Some((
                Date::from_calendar_date(2027, time::Month::January, 1).unwrap(),
                Date::from_calendar_date(2027, time::Month::January, 31).unwrap(),
            )),
            ..Default::default()
        };
        let comparison = compare(&bill, &[day(1, &[], 0.0, &[(USAGE, 1.0)])]).unwrap();
        assert_eq!((comparison.days, comparison.days_cached), (31, 1));
        assert!(!comparison.complete());
    }
}
