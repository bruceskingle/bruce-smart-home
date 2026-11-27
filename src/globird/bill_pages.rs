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

/// A bill's row on the Bills page: the email it came in, whether that email is cached, and the bill, if its PDF is
/// cached and could be read.
#[derive(Clone, PartialEq)]
struct BillRow {
    email: bills::ListItem,
    email_cached: bool,
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
                email_cached: cached.details.contains_key(&email.id),
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
                email_cached: true,
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

/// Where a bill's window opens in the main window, just right of the column of invoices, so that the others can still
/// be clicked, and below the navigation bar, as Octopus's bills' do: the right of the column and the top of the page.
const ANCHOR: &str = "
    const rights = [...document.querySelectorAll('.bill-ref')].map((cell) => cell.getBoundingClientRect().right);
    const page = document.querySelector('.app-page');
    return [rights.length ? Math.max(...rights) : 0, page ? page.getBoundingClientRect().top : 0];
";
/// The space between the column of invoices and a bill's window.
const ANCHOR_GAP: f64 = 8.0;

/// Opens the bill of the cached document in a window of its own, see BillWindow, beside the column of invoices, or
/// the one already open.
async fn open_bill(context: SharedProp<GlobirdContext>, document_id: i64) {
    let anchor = document::eval(ANCHOR)
        .join::<(f64, f64)>()
        .await
        .ok()
        .map(|(x, y)| dioxus::desktop::LogicalPosition::new(x + ANCHOR_GAP, y));
    let title = match context.0.bill(document_id) {
        Ok(bill) => pdf_title(&bill, &document_id.to_string()),
        Err(_) => format!("GloBird bill {document_id}"),
    };
    crate::components::child_windows::open(&format!("globird-bill-{document_id}-details"), title, anchor, || {
        VirtualDom::new_with_props(BillWindow, BillWindowProps { context, document_id })
    });
}

/// The bills, from the list of GloBird's emails, which is brought up to date when the page opens, if it is a day old,
/// see bills::refresh. Each bill opens in a window of its own, see BillWindow, once its PDF is fetched, which its
/// Fetch does, see bills::fetch_pdf. What is read from the bills' PDFs is in the scraped colour.
#[component]
pub fn BillsPage(context: SharedProp<GlobirdContext>) -> Element {
    let mut generation = use_signal(|| 0u32);
    let mut checking = use_signal(|| false);
    let mut problem = use_signal(|| None::<String>);
    generation();
    let cache_config = context.0.cache_config.clone();
    let cached = bills::cached(&cache_config);
    let rows = bill_rows(&context.0, &cached);
    use_future({
        let context = context.clone();
        move || {
            let context = context.clone();
            async move {
                let cache_config = &context.0.cache_config;
                let account_cached = fetch::account(cache_config).is_some();
                if cache_config.offline || bills::list_request(&bills::cached(cache_config), account_cached).is_none() {
                    return;
                }
                checking.set(true);
                problem.set(context.0.refresh_bills().await.err().map(|error| format!("{error:#}")));
                checking.set(false);
                generation += 1;
            }
        }
    });

    rsx! {
        h1 { "GloBird Bills" }
        p {
            "The bills GloBird has emailed, newest first. Click a bill's invoice to see its charges, beside what the days add up to, in a window of its own, or Fetch, to fetch its PDF first. The list is fetched from GloBird when this page opens, once a day, and a bill once, as it never changes."
        }
        if checking() {
            p { "Looking for new bills..." }
        }
        if let Some(problem) = problem() {
            p { class: "error", ErrorText { error: problem, message: String::from("Unable to look for new bills") } }
        }
        if let Some(total) = cached.total_count {
            p { "{cached.items.len()} of GloBird's {total} emails are listed, of which {cached.bills().count()} are bills." }
        }
        table {
            tr {
                th { "Emailed" }
                th { "Invoice" }
                th { "Period" }
                th { class: "number", "Total $" }
            }
            for row in rows {
                BillRowView { row, context: context.clone(), on_fetched: move |_| generation += 1 }
            }
        }
    }
}

#[component]
fn BillRowView(row: BillRow, context: SharedProp<GlobirdContext>, on_fetched: EventHandler<()>) -> Element {
    let mut fetching = use_signal(|| false);
    let mut fetch_problem = use_signal(|| None::<String>);
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
    // Its PDF can be fetched if it is not cached, unless its email is, and has none.
    let fetchable = row.bill.is_none() && !(row.email_cached && row.document.is_none());
    let status = if row.document.is_none() && row.email_cached {
        "Its email has no PDF"
    } else if reversed {
        "Reversed"
    } else {
        ""
    };
    let document_id = row.document.as_ref().map(|document| document.document_id);
    let email_id = row.email.id;
    let open = {
        let context = context.clone();
        move |_| {
            if let Some(document_id) = document_id {
                spawn(open_bill(context.clone(), document_id));
            }
        }
    };
    // Fetches its PDF, and its email first if need be, then opens it.
    let fetch_and_open = move |_| {
        let context = context.clone();
        spawn(async move {
            fetching.set(true);
            let fetched = context.0.fetch_bill_pdf(email_id).await;
            fetching.set(false);
            on_fetched.call(());
            match fetched {
                Ok(()) => {
                    let document_id = bills::cached(&context.0.cache_config)
                        .details
                        .get(&email_id)
                        .and_then(|detail| detail.document_list.first())
                        .map(|document| document.document_id);
                    fetch_problem.set(None);
                    if let Some(document_id) = document_id {
                        open_bill(context, document_id).await;
                    }
                }
                Err(error) => fetch_problem.set(Some(format!("{error:#}"))),
            }
        });
    };
    rsx! {
        tr {
            td { "{emailed}" }
            td { class: "bill-ref",
                // A link, in the links' colour, though what it says is read from the bill.
                if row.bill.is_some() {
                    span { class: "link", onclick: open,
                        {bill.as_ref().and_then(|bill| bill.invoice.clone()).unwrap_or(name.clone())}
                    }
                }
                else if fetchable {
                    if fetching() {
                        "Fetching..."
                    }
                    else {
                        span { class: "link", onclick: fetch_and_open, "Fetch" }
                    }
                }
            }
            td { class: "scraped",
                {period_text(bill.as_ref().and_then(|bill| bill.period))}
                if let Some(problem) = fetch_problem() {
                    span { class: "error", ErrorText { error: problem, message: String::from("Unable to fetch the bill") } }
                }
                if let Some(error) = error {
                    span { class: "error", ErrorText { error, message: String::from("Unable to read the bill") } }
                }
                "{status}"
            }
            td { class: "number scraped", {bill.as_ref().and_then(|bill| bill.total).map(dollars).unwrap_or_default()} }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct BillWindowProps {
    context: SharedProp<GlobirdContext>,
    document_id: i64,
}

/// The root of a bill's window, which has its own VirtualDom, so it links the stylesheet itself, as Octopus's bill's
/// window does. Escape closes it.
#[component]
fn BillWindow(props: BillWindowProps) -> Element {
    crate::components::child_windows::use_child_window();
    // The same in every window, see help::use_help_shown.
    let mut show_help = crate::help::use_help_shown();
    use_future(|| async {
        let mut keys = document::eval(
            "document.addEventListener('keydown', (event) => { if (event.key === 'Escape') { dioxus.send(true); } });",
        );
        while keys.recv::<bool>().await.is_ok() {
            dioxus::desktop::window().close();
        }
    });
    rsx! {
        document::Link { rel: "stylesheet", href: crate::components::app::MAIN_CSS }
        crate::components::tooltips::Tooltips {}
        div { class: "bill-window-layout",
            div { class: "bill-window",
                if !show_help() {
                    button {
                        r#type: "button",
                        class: "icon-button help-reopen",
                        title: "Show the help",
                        onclick: move |_| {
                            crate::help::set_shown(true);
                            show_help.set(true);
                        },
                        dioxus_free_icons::Icon { icon: dioxus_free_icons::icons::ld_icons::LdCircleHelp, width: 16, height: 16 }
                    }
                }
                BillDetail { context: props.context.clone(), document_id: props.document_id }
            }
            if show_help() {
                crate::views::help_panel::HelpPanel {
                    topic: String::from("globird/index.md"),
                    on_close: move |_| {
                        crate::help::set_shown(false);
                        show_help.set(false);
                    },
                }
            }
        }
    }
}

/// One bill: its charges, as it says them, in the scraped colour, beside the days', and its PDF, which opens in a
/// window of its own, see open_pdf.
#[component]
fn BillDetail(context: SharedProp<GlobirdContext>, document_id: i64) -> Element {
    let cache_config = context.0.cache_config.clone();
    let bill = match context.0.bill(document_id) {
        Ok(bill) => bill,
        Err(error) => {
            return rsx! { div { class: "error", ErrorText { error, message: String::from("Unable to read the bill") } } };
        }
    };
    // Drawn again once Fetch has fetched the days.
    let mut generation = use_signal(|| 0u32);
    let mut fetching = use_signal(|| false);
    let mut fetch_problem = use_signal(|| None::<String>);
    generation();
    let summaries = fetch::summaries(&context.0.cache_config);
    let comparison = compare(&bill, &summaries).unwrap_or_default();
    // The days' figures only when they are all of the bill's, which they can only be compared with then.
    let complete = comparison.complete();
    // Its missing days can be fetched, as GloBird keeps them, unless the domain is offline.
    let period = bill.period;
    let fetchable = !complete && period.is_some() && !context.0.cache_config.offline;
    let fetch_days = {
        let context = context.clone();
        move |_| {
            let context = context.clone();
            let Some((first, last)) = period else {
                return;
            };
            spawn(async move {
                fetching.set(true);
                let fetched = context.0.fetch_days(first, last, &mut |_| {}).await;
                fetch_problem.set(fetched.err().map(|error| format!("{error:#}")));
                fetching.set(false);
                generation += 1;
            });
        }
    };
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
        h1 { "GloBird Bill " span { class: "scraped", {bill.invoice.clone().unwrap_or_default()} } }
        table { class: "display",
            tr {
                th { class: "row-header", "Period:" }
                td { class: "scraped", {period_text(bill.period)} }
            }
            if let Some(plan) = &bill.plan {
                tr {
                    th { class: "row-header", "Plan:" }
                    td { class: "scraped", "{plan}" }
                }
            }
            if let Some(issued) = bill.issued {
                tr {
                    th { class: "row-header", "Issued:" }
                    td { class: "scraped", "{issued}" }
                }
            }
            if let Some(due) = bill.due {
                tr {
                    th { class: "row-header", "Due:" }
                    td { class: "scraped", "{due}" }
                }
            }
        }
        p { button { onclick: open, "View PDF" } }
        if !complete {
            p {
                if fetchable {
                    button { disabled: fetching(), onclick: fetch_days, if fetching() { "Fetching..." } else { "Fetch" } }
                    " "
                }
                "{comparison.days_cached} of the bill's {comparison.days} days are cached, so the days' figures are not all of its period."
            }
            if let Some(problem) = fetch_problem() {
                p { class: "error", ErrorText { error: problem, message: String::from("Unable to fetch the days") } }
            }
        }
        h2 { "Charges" }
        p { "Each charge as the bill says it, beside the meter's kWh of its charge type over the bill's days, from the Days page, or, for the daily charge, the days whose costs have one. Rates are in c/kWh, and $ a day for the daily charge." }
        table {
            tr {
                th { "Charge" }
                th { class: "number", "Quantity" }
                th { "Unit" }
                th { class: "number", "Rate" }
                th { class: "number", "$" }
                if complete {
                    th { class: "number", "The days' quantity" }
                }
            }
            for (index, line) in bill.lines.iter().enumerate() {
                tr {
                    td { class: "scraped", "{line.description}" }
                    td { class: "number scraped", {format!("{}", line.quantity)} }
                    td { class: "scraped", "{line.unit}" }
                    td { class: "number scraped", {unit_rate(line)} }
                    td { class: "number scraped", {dollars(line.amount)} }
                    if complete {
                        td { class: "number derived", {ours(index, line)} }
                    }
                }
            }
            tr {
                td { "Total, including GST of " span { class: "scraped", {bill.gst.map(dollars).unwrap_or_default()} } }
                td {}
                td {}
                td {}
                td { class: "number scraped", {bill.total.map(dollars).unwrap_or_default()} }
                if complete {
                    td {}
                }
            }
        }
        h2 { "By category" }
        p { "The bill's charges added up by the categories of the days' costs, e.g. USAGE for Peak and Offpeak Usage, beside what the days' costs of each add up to." }
        table {
            tr {
                th { "Category" }
                th { class: "number", "The bill's $" }
                if complete {
                    th { class: "number", "The days' $" }
                    th { class: "number", "Difference $" }
                }
            }
            for (category, (billed, days)) in comparison.categories.iter() {
                tr {
                    td { "{category}" }
                    td { class: "number derived", {dollars(*billed)} }
                    if complete {
                        td { class: "number derived", {dollars(*days)} }
                        td { class: "number derived", {format!("{:+.2}", days - billed)} }
                    }
                }
            }
            tr {
                td { "Total" }
                td { class: "number scraped", {bill.total.map(dollars).unwrap_or_default()} }
                if complete {
                    td { class: "number derived", {dollars(comparison.total)} }
                    td { class: "number derived", {bill.total.map(|total| format!("{:+.2}", comparison.total - total)).unwrap_or_default()} }
                }
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
