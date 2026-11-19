//! GloBird's bills, from the account's correspondence: the emails GloBird has sent, each bill's with the bill attached
//! as a PDF, whose text is read for its charges, see parse_bill.
//!
//! What GloBird sent is kept as it sent it, in the profile's store, see store.rs, in DataSets of the account, by the
//! hash of its ID: each email listed, `globird.mail`, by when it was sent and its ID, with a `#list/<offset>` record of
//! each page fetched, saying how many emails there are; each bill's email, `globird.mail.detail`, by its ID; and each
//! bill's PDF in the store's blobs, with a `globird.document` record of it, by the document's ID. A bill never changes
//! once it is issued, so nothing but the first page of the list is fetched again. The list is fetched when the Bills page
//! opens, if it is not cached or is a day old, see refresh, a request or two, and a bill's email and PDF only when the
//! user asks for its PDF, see fetch_pdf, so that GloBird is asked for little, and for nothing the user does not see.

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::anyhow;
use bruce_db::{Id, Item, Micros, Part, Store};
use regex::Regex;
use serde::Deserialize;
use time::Date;

use crate::CacheConfig;

use super::client::GlobirdClient;

/// The kinds of the correspondence, see the module documentation.
const MAIL_KIND: &str = "globird.mail";
const DETAIL_KIND: &str = "globird.mail.detail";
const DOCUMENT_KIND: &str = "globird.document";
/// How many emails a page of the list has, the most the portal's own page offers, as the list is small, about 100
/// bytes an email, so that the whole history is listed in a request or two.
pub const PAGE_SIZE: usize = 100;
/// How long the first page of the list is used before it is fetched again, to find new bills.
const LIST_REFRESH: Duration = Duration::from_secs(24 * 60 * 60);

/// An email in the list of correspondence.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListItem {
    pub id: i64,
    pub title: Option<String>,
    /// When it was sent, e.g. 2026-10-27T17:30:56.1627456.
    pub created_date: Option<String>,
}

impl ListItem {
    /// Whether it is a bill's email, e.g. "Your latest GloBird Energy bill is here", rather than e.g. a welcome letter.
    pub fn is_bill(&self) -> bool {
        self.title
            .as_deref()
            .is_some_and(|title| title.to_lowercase().contains("bill"))
    }

    /// The day it was sent.
    pub fn date(&self) -> Option<Date> {
        let format = time::macros::format_description!("[year]-[month]-[day]");
        Date::parse(self.created_date.as_deref()?.get(..10)?, &format).ok()
    }
}

/// A document attached to an email, e.g. a bill.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub document_id: i64,
    /// e.g. Invoice12361365.
    pub name: Option<String>,
    /// Whether the bill was reversed, i.e. cancelled, as GloBird spells it.
    #[serde(rename = "isReversedInvocie")]
    pub is_reversed: Option<bool>,
}

/// An email, with its documents.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    #[serde(default)]
    pub document_list: Vec<Document>,
}

/// A request for the account, the list, or a bill, see list_request and pdf_request.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// The account, for its ID, which the requests of the correspondence need.
    Account,
    /// A page of the list of emails, from offset.
    List { offset: usize },
    /// A bill's email, for its documents.
    Detail { email_id: i64 },
    /// A bill.
    Document { document_id: i64 },
}

/// What is cached of the correspondence, see cached.
#[derive(Debug, Clone, Default)]
pub struct Cached {
    /// The emails of every cached page, newest first, each once.
    pub items: Vec<ListItem>,
    /// How many there are in all, from the first page, None if it is not cached.
    pub total_count: Option<usize>,
    /// How long ago the first page was fetched.
    pub first_page_age: Option<Duration>,
    /// The details of the emails cached, by email ID.
    pub details: BTreeMap<i64, Detail>,
    /// The bills whose PDFs are cached, by document ID, with their blobs' IDs.
    pub documents: BTreeMap<i64, Id>,
}

impl Cached {
    /// The bills' emails, newest first.
    pub fn bills(&self) -> impl Iterator<Item = &ListItem> {
        self.items.iter().filter(|item| item.is_bill())
    }
}

/// A DataSet of the kind of the account, by the hash of its ID.
fn account_key(store: &Store, kind: &str, account_id: i64) -> anyhow::Result<String> {
    store.hash_key(
        kind,
        &[Part::Id(Id::of(&serde_json::json!({"globird-account": account_id}))?)],
    )
}

/// The sort key of a page of the list's record.
fn list_key(offset: usize) -> String {
    format!("#list/{offset}")
}

/// What is cached, see Cached, of the account, if it has been fetched. The emails are by when they were sent, so newest
/// first is the reverse of their order.
pub fn cached(cache_config: &CacheConfig) -> Cached {
    let mut result = Cached::default();
    let (Ok(store), Some(account_id)) = (
        cache_config.store(),
        super::fetch::account(cache_config).as_ref().and_then(account_id),
    ) else {
        return result;
    };
    if let Ok(hk) = account_key(store, MAIL_KIND, account_id) {
        if let Ok(Some(first)) = store.get(&hk, &list_key(0)) {
            result.total_count = first
                .attributes
                .as_ref()
                .and_then(|page| page.get("totalCount")?.as_u64())
                .map(|total| total as usize);
            result.first_page_age = Micros::now()
                .0
                .checked_sub(first.version.time.0)
                .map(|age| Duration::from_micros(age.max(0) as u64));
        }
        let items = store.query_all(&hk, "0".."\u{10FFFF}").unwrap_or_default();
        result.items = items
            .into_iter()
            .rev()
            .filter_map(|item| serde_json::from_value(item.attributes?).ok())
            .collect();
    }
    if let Ok(hk) = account_key(store, DETAIL_KIND, account_id) {
        for item in store.all(&hk).unwrap_or_default() {
            match (
                item.sk.parse::<i64>(),
                item.attributes.map(serde_json::from_value::<Detail>),
            ) {
                (Ok(email_id), Some(Ok(detail))) => {
                    result.details.insert(email_id, detail);
                }
                _ => crate::log_warning!("Unable to read the GloBird email {}", item.sk),
            }
        }
    }
    if let Ok(hk) = account_key(store, DOCUMENT_KIND, account_id) {
        for item in store.all(&hk).unwrap_or_default() {
            let blob = item
                .attributes
                .as_ref()
                .and_then(|document| Id::parse(document.get("blob")?.as_str()?).ok());
            if let (Ok(document_id), Some(blob)) = (item.sk.parse::<i64>(), blob) {
                result.documents.insert(document_id, blob);
            }
        }
    }
    result
}

/// The PDF of a cached document.
pub fn document(cache_config: &CacheConfig, document_id: i64) -> anyhow::Result<Vec<u8>> {
    let blob = cached(cache_config)
        .documents
        .get(&document_id)
        .cloned()
        .ok_or_else(|| anyhow!("The bill's PDF has not been fetched"))?;
    cache_config
        .store()?
        .get_blob(&blob)?
        .ok_or_else(|| anyhow!("The bill's PDF is missing from the store"))
}

/// The next request to bring the list up to date, None if it is: the account if it is not cached, the first page of
/// the list if it is not cached or is older than LIST_REFRESH, to find new bills, then the next page of older emails.
pub fn list_request(cached: &Cached, account_cached: bool) -> Option<Request> {
    if !account_cached {
        return Some(Request::Account);
    }
    if cached.total_count.is_none() || cached.first_page_age.is_some_and(|age| age > LIST_REFRESH) {
        return Some(Request::List { offset: 0 });
    }
    // The emails cached are the newest, so the next page starts after them.
    let known = cached.items.len();
    (cached.total_count.is_some_and(|total| known < total)).then_some(Request::List { offset: known })
}

/// The next request for the PDF of the bill emailed in email_id, None if it is cached: its email, for its documents,
/// then its first document which is not cached.
pub fn pdf_request(cached: &Cached, email_id: i64) -> Option<Request> {
    let Some(detail) = cached.details.get(&email_id) else {
        return Some(Request::Detail { email_id });
    };
    detail
        .document_list
        .iter()
        .find(|document| !cached.documents.contains_key(&document.document_id))
        .map(|document| Request::Document {
            document_id: document.document_id,
        })
}

/// The account's ID, from the cached account, which the requests of the correspondence need.
pub fn account_id(user: &serde_json::Value) -> Option<i64> {
    user.get("accounts")?.as_array()?.first()?.get("accountId")?.as_i64()
}

/// Saves a page of the list, {"totalCount": ..., "data": [emails]}, each email as GloBird gave it, by when it was sent
/// and its ID, and the page's record, see the module documentation.
fn save_list_page(store: &Store, account_id: i64, offset: usize, page: &serde_json::Value) -> anyhow::Result<()> {
    let hk = account_key(store, MAIL_KIND, account_id)?;
    for item in page.get("data").and_then(|data| data.as_array()).into_iter().flatten() {
        let (Some(id), Some(created)) = (
            item.get("id").and_then(|id| id.as_i64()),
            item.get("createdDate").and_then(|date| date.as_str()),
        ) else {
            continue;
        };
        store.put(&Item::new(
            &hk,
            &format!("{created}#{id}"),
            crate::store::version_now(),
            item.clone(),
        ))?;
    }
    store.put(&Item::new(
        &hk,
        &list_key(offset),
        crate::store::version_now(),
        serde_json::json!({ "totalCount": page.get("totalCount") }),
    ))?;
    Ok(())
}

/// The most requests a refresh or a PDF makes, so that a reply which never says it is done, e.g. a list whose total
/// is more than it gives, cannot ask GloBird again and again.
const MAX_REQUESTS: usize = 10;

/// Brings the list of emails up to date, see list_request, returning how many requests it made: none if it is, one a
/// day, to find new bills, and a few the first time, for the history.
pub async fn refresh(client: &GlobirdClient, cache_config: &CacheConfig) -> anyhow::Result<usize> {
    let mut made = 0;
    while made < MAX_REQUESTS {
        let account_cached = super::fetch::account(cache_config).is_some();
        let Some(request) = list_request(&cached(cache_config), account_cached) else {
            break;
        };
        make(client, cache_config, request).await?;
        made += 1;
    }
    Ok(made)
}

/// Fetches the PDF of the bill emailed in email_id, and its email first if that is not cached, see pdf_request.
pub async fn fetch_pdf(client: &GlobirdClient, cache_config: &CacheConfig, email_id: i64) -> anyhow::Result<()> {
    for _ in 0..MAX_REQUESTS {
        let Some(request) = pdf_request(&cached(cache_config), email_id) else {
            return Ok(());
        };
        make(client, cache_config, request).await?;
    }
    Err(anyhow!("GloBird did not give the bill's PDF"))
}

/// Makes the request, and keeps its reply.
async fn make(client: &GlobirdClient, cache_config: &CacheConfig, request: Request) -> anyhow::Result<()> {
    if cache_config.offline {
        return Err(anyhow!("This profile is offline, so nothing can be fetched"));
    }
    let store = cache_config.store()?;
    if request == Request::Account {
        return super::fetch::save_account(cache_config, &client.current_user().await?);
    }
    let account_id = super::fetch::account(cache_config)
        .as_ref()
        .and_then(account_id)
        .ok_or_else(|| anyhow!("The GloBird account has no account ID"))?;
    let version = crate::store::version_now;
    match request {
        Request::Account => {}
        Request::List { offset } => {
            let page = client.correspondence_list(account_id, offset, PAGE_SIZE).await?;
            save_list_page(store, account_id, offset, &page)?;
        }
        Request::Detail { email_id } => {
            let detail = client.correspondence_detail(account_id, email_id).await?;
            store.put(&Item::new(
                &account_key(store, DETAIL_KIND, account_id)?,
                &email_id.to_string(),
                version(),
                detail,
            ))?;
        }
        Request::Document { document_id } => {
            let pdf = client.document(account_id, document_id).await?;
            let blob = store.put_blob(&pdf)?;
            store.put(&Item::new(
                &account_key(store, DOCUMENT_KIND, account_id)?,
                &document_id.to_string(),
                version(),
                serde_json::json!({ "documentId": document_id, "blob": blob.as_str(), "bytes": pdf.len() }),
            ))?;
        }
    }
    Ok(())
}

/// A charge on a bill, e.g. Peak Usage 5.83 kWh x $0.37180 = $2.17.
#[derive(Debug, Clone, PartialEq)]
pub struct BillLine {
    /// e.g. Peak Usage, Daily Charge, which are the usage's charge types and the daily charge.
    pub description: String,
    pub quantity: f64,
    /// kWh or Days.
    pub unit: String,
    /// In AUD, negative for a credit, e.g. a feed-in.
    pub rate: f64,
    pub amount: f64,
}

impl BillLine {
    /// Whether it is charged by the day, e.g. the daily charge, rather than by the kWh.
    pub fn per_day(&self) -> bool {
        self.unit.to_lowercase().starts_with("day")
    }

    /// Whether it is the daily charge, rather than e.g. a credit of so much a day.
    pub fn is_daily(&self) -> bool {
        self.per_day() && self.description.to_lowercase().contains("daily")
    }
}

/// What a bill says, as read from its text, see parse_bill.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Bill {
    pub invoice: Option<String>,
    pub issued: Option<Date>,
    pub due: Option<Date>,
    /// The first and last days it is for.
    pub period: Option<(Date, Date)>,
    /// The tariff's name, e.g. BOOST.
    pub plan: Option<String>,
    pub lines: Vec<BillLine>,
    /// In AUD, including GST.
    pub total: Option<f64>,
    pub gst: Option<f64>,
}

/// A date as a bill has one, e.g. 28-Sep-2026.
fn bill_date(text: &str) -> Option<Date> {
    let mut parts = text.split('-');
    let day: u8 = parts.next()?.parse().ok()?;
    let month = parts.next()?.to_lowercase();
    let year: i32 = parts.next()?.parse().ok()?;
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let month = time::Month::try_from(months.iter().position(|name| month.starts_with(name))? as u8 + 1).ok()?;
    Date::from_calendar_date(year, month, day).ok()
}

/// An amount as a bill has one, e.g. $1.30900, -$0.03000 or $1,234.56.
fn money(text: &str) -> Option<f64> {
    let negative = text.contains('-');
    let value: f64 = text.replace(['$', '-', ','], "").parse().ok()?;
    Some(if negative { -value } else { value })
}

/// The patterns of a bill's text, made once.
struct Patterns {
    line: Regex,
    period: Regex,
    invoice: Regex,
    issued: Regex,
    due: Regex,
    total: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        const DATE: &str = r"\d{1,2}-[A-Za-z]{3}-\d{4}";
        Patterns {
            line: Regex::new(r"^(.+?) \d{1,2}-[A-Za-z]{3} - \d{1,2}-[A-Za-z]{3} ([\d,.]+) (kWh|Days?) x (-?\$-?[\d,.]+) (-?\$-?[\d,.]+)$").unwrap(),
            period: Regex::new(&format!(r"Billing Period: ({DATE}) to ({DATE})")).unwrap(),
            invoice: Regex::new(r"Invoice number (\S+)").unwrap(),
            issued: Regex::new(&format!(r"Issue Date ({DATE})")).unwrap(),
            due: Regex::new(&format!(r"Due Date[^\d]*({DATE})")).unwrap(),
            total: Regex::new(r"TOTAL \(including (-?\$-?[\d,.]+) GST\) (-?\$-?[\d,.]+)").unwrap(),
        }
    })
}

/// What a bill's text says of electricity. Its charges are listed twice, first alone, e.g. Peak Usage, then with the
/// plan's name, e.g. Peak Usage BOOST; the first list's are kept, as their descriptions are the usage's charge types,
/// and the plan is what the second list adds to them. A bill of gas too has a section of each fuel in both lists, gas
/// first, each with its own billing period and total; the gas sections are left out, see in_gas_section.
pub fn parse_bill(text: &str) -> Bill {
    let patterns = patterns();
    let mut bill = Bill::default();
    let mut first_list_done = false;
    let mut gas = false;
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        gas = in_gas_section(&line, gas);
        if gas {
            continue;
        }
        let first = |pattern: &Regex| {
            pattern
                .captures(&line)
                .and_then(|captures| captures.get(1))
                .map(|found| found.as_str().to_string())
        };
        if let Some(captures) = patterns.line.captures(&line) {
            let description = captures[1].to_string();
            if first_list_done {
                if bill.plan.is_none() {
                    bill.plan = bill.lines.iter().find_map(|known| {
                        description
                            .strip_prefix(&format!("{} ", known.description))
                            .map(str::to_string)
                    });
                }
                continue;
            }
            let (Some(quantity), Some(rate), Some(amount)) =
                (money(&captures[2]), money(&captures[4]), money(&captures[5]))
            else {
                continue;
            };
            bill.lines.push(BillLine {
                description,
                quantity,
                unit: captures[3].to_string(),
                rate,
                amount,
            });
        } else if let Some(captures) = patterns.total.captures(&line) {
            if !first_list_done {
                bill.gst = money(&captures[1]);
                bill.total = money(&captures[2]);
            }
            first_list_done = true;
        } else if let Some(captures) = patterns.period.captures(&line) {
            if bill.period.is_none() {
                bill.period = bill_date(&captures[1]).zip(bill_date(&captures[2]));
            }
        } else if bill.invoice.is_none()
            && let Some(invoice) = first(&patterns.invoice)
        {
            bill.invoice = Some(invoice);
        } else if bill.issued.is_none()
            && let Some(issued) = first(&patterns.issued)
        {
            bill.issued = bill_date(&issued);
        } else if bill.due.is_none()
            && let Some(due) = first(&patterns.due)
        {
            bill.due = bill_date(&due);
        }
    }
    bill
}

/// Whether a bill's line is in a section of gas charges, given whether the line before it was: a section starts with
/// its billing period, e.g. "Billing Period: … ,Total Usage: 2358.16 MJ", or, in the list with the plan's name, its
/// heading, e.g. "Gas Charges for Period …" or "Electricity Charges for Period …".
fn in_gas_section(line: &str, gas: bool) -> bool {
    if line.starts_with("Billing Period:") {
        line.contains(" MJ")
    } else if line.starts_with("Gas Charges for Period") {
        true
    } else if line.starts_with("Electricity Charges for Period") {
        false
    } else {
        gas
    }
}

/// The text of a PDF. pdf-extract panics on some PDFs it cannot read, so that is caught and is an error too.
pub fn pdf_text(bytes: &[u8]) -> anyhow::Result<String> {
    match std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(bytes)) {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(error)) => Err(anyhow!("Unable to read the PDF's text: {error}")),
        Err(_) => Err(anyhow!("Unable to read the PDF's text")),
    }
}

/// A bill, read from its PDF, see parse_bill, or why it could not be.
pub fn read_bill(pdf: &[u8]) -> anyhow::Result<Bill> {
    let text = pdf_text(pdf)?;
    let bill = parse_bill(&text);
    if (bill.lines.is_empty() || bill.total.is_none()) && text.contains("Gas Charges for Period") {
        return Err(anyhow!("It is a bill of gas alone, and the app reads only electricity"));
    }
    if bill.lines.is_empty() || bill.total.is_none() {
        return Err(anyhow!(
            "No charges or total were found in the bill's text, which may be in a form not known yet"
        ));
    }
    Ok(bill)
}

/// Converts the correspondence kept in the module's directory of the cache before the store, `list-<offset>`,
/// `detail-<email>` and `document-<document>.pdf`, see convert.rs, once the account has been, as the DataSets are the
/// account's.
pub fn convert_legacy(cache_config: &CacheConfig, dir: &std::path::Path) -> anyhow::Result<String> {
    let store = cache_config.store()?;
    let Some(account_id) = super::fetch::account(cache_config).as_ref().and_then(account_id) else {
        return Ok(String::from("no bills, as the account was not fetched"));
    };
    let (mut emails, mut bills) = (0, 0);
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let json =
            || -> anyhow::Result<serde_json::Value> { Ok(serde_json::from_str(&std::fs::read_to_string(&path)?)?) };
        if let Some(offset) = name
            .strip_prefix("list-")
            .and_then(|offset| offset.parse::<usize>().ok())
        {
            save_list_page(store, account_id, offset, &json()?)?;
        } else if let Some(email_id) = name.strip_prefix("detail-") {
            store.put(&Item::new(
                &account_key(store, DETAIL_KIND, account_id)?,
                email_id,
                crate::store::version_now(),
                json()?,
            ))?;
            emails += 1;
        } else if let Some(document_id) = name.strip_prefix("document-").and_then(|id| id.strip_suffix(".pdf")) {
            let pdf = std::fs::read(&path)?;
            let blob = store.put_blob(&pdf)?;
            store.put(&Item::new(&account_key(store, DOCUMENT_KIND, account_id)?, document_id, crate::store::version_now(),
                serde_json::json!({ "documentId": document_id.parse::<i64>().ok(), "blob": blob.as_str(), "bytes": pdf.len() })))?;
            bills += 1;
        }
    }
    Ok(format!("{emails} bill emails, {bills} bills"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64, title: &str) -> ListItem {
        ListItem {
            id,
            title: Some(title.to_string()),
            created_date: Some(String::from("2026-01-02T03:04:05.0")),
        }
    }

    fn document(document_id: i64) -> Document {
        Document {
            document_id,
            name: Some(format!("Invoice{document_id}")),
            is_reversed: Some(false),
        }
    }

    #[test]
    fn the_list_is_brought_up_to_date_newest_first_then_once_a_day() {
        let mut cached = Cached::default();
        assert_eq!(list_request(&cached, false), Some(Request::Account));
        assert_eq!(list_request(&cached, true), Some(Request::List { offset: 0 }));

        cached.total_count = Some(4);
        cached.first_page_age = Some(Duration::from_secs(60));
        cached.items = vec![item(3, "Welcome"), item(2, "Your latest bill is here")];
        assert_eq!(list_request(&cached, true), Some(Request::List { offset: 2 }));

        cached.items.push(item(1, "Your latest bill is here"));
        cached.items.push(item(0, "Your latest bill is here"));
        assert_eq!(list_request(&cached, true), None);

        cached.first_page_age = Some(LIST_REFRESH + Duration::from_secs(1));
        assert_eq!(
            list_request(&cached, true),
            Some(Request::List { offset: 0 }),
            "new bills are looked for"
        );
    }

    #[test]
    fn a_bills_pdf_needs_its_email_then_its_document() {
        let mut cached = Cached::default();
        assert_eq!(pdf_request(&cached, 2), Some(Request::Detail { email_id: 2 }));
        cached.details.insert(
            2,
            Detail {
                document_list: vec![document(20)],
            },
        );
        assert_eq!(pdf_request(&cached, 2), Some(Request::Document { document_id: 20 }));
        cached.documents.insert(20, Id::of_bytes(b"x"));
        assert_eq!(pdf_request(&cached, 2), None);
        // An email with no documents has nothing more to fetch.
        cached.details.insert(1, Detail::default());
        assert_eq!(pdf_request(&cached, 1), None);
    }

    #[test]
    fn pages_fetched_again_after_new_emails_keep_each_email_once_newest_first() {
        let dir = std::env::temp_dir().join(format!("globird-bills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let site = Id::of(&serde_json::json!({"site": "bills test"})).unwrap();
        let store = std::sync::Arc::new(Store::open(&format!("file://{}", dir.display()), &site).unwrap());
        let cache_config = CacheConfig::new(dir.join("cache"), false).with_store(store.clone());
        super::super::fetch::save_account(&cache_config, &serde_json::json!({"accounts": [{"accountId": 7}]})).unwrap();
        let page = |ids: &[i64]| {
            serde_json::json!({"totalCount": 4, "data": ids.iter()
            .map(|id| serde_json::json!({"id": id, "title": "bill", "createdDate": format!("2026-0{id}-01T00:00:00")})).collect::<Vec<_>>()})
        };
        // The first page was fetched again after email 4 came, so the second page, fetched before, repeats email 2.
        save_list_page(&store, 7, 2, &page(&[2, 1])).unwrap();
        save_list_page(&store, 7, 0, &page(&[4, 3])).unwrap();
        save_list_page(&store, 7, 1, &page(&[3, 2])).unwrap();
        let cached = cached(&cache_config);
        assert_eq!(
            cached.items.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![4, 3, 2, 1]
        );
        assert_eq!(cached.total_count, Some(4));
        assert!(cached.first_page_age.is_some_and(|age| age < Duration::from_secs(60)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The text of a made-up bill, laid out as pdf-extract gives a GloBird bill's.
    const TEXT: &str = "Invoice number 1000001
Issue Date 3-Feb-2027
Due Date (Direct Debit Date) 17-Feb-2027
Billing Period: 01-Jan-2027 to 31-Jan-2027 (31 days) ,Total Usage: 310.00 kWh ,Total Solar Generation: 50.00 kWh
Daily Charge 01-Jan - 31-Jan 31 Days x  $1.00000 $31.00
Peak Usage 01-Jan - 31-Jan 10.00 kWh x  $0.40000 $4.00
Offpeak Usage 01-Jan - 31-Jan 300.00 kWh x  $0.20000 $60.00
Standard Solar Feed in 01-Jan - 31-Jan 50.00 kWh x -$0.05000 -$2.50
TOTAL (including $8.41 GST) $92.50
Electricity Charges for Period 01-Jan-2027 to 31-Jan-2027
Peak Usage SAVER 01-Jan - 31-Jan 10.00 kWh x  $0.40000 $4.00
Offpeak Usage SAVER 01-Jan - 31-Jan 300.00 kWh x  $0.20000 $60.00
Standard Solar Feed in SAVER 01-Jan - 31-Jan 50.00 kWh x -$0.05000 -$2.50
Daily Charge SAVER 01-Jan - 31-Jan 31 Days x  $1.00000 $31.00
TOTAL (including $8.41 GST) $92.50
";

    #[test]
    fn a_bills_charges_total_period_and_plan_are_read_from_its_text() {
        let bill = parse_bill(TEXT);
        let date = |year, month, day| Date::from_calendar_date(year, month, day).unwrap();
        assert_eq!(bill.invoice.as_deref(), Some("1000001"));
        assert_eq!(bill.issued, Some(date(2027, time::Month::February, 3)));
        assert_eq!(bill.due, Some(date(2027, time::Month::February, 17)));
        assert_eq!(
            bill.period,
            Some((
                date(2027, time::Month::January, 1),
                date(2027, time::Month::January, 31)
            ))
        );
        assert_eq!(bill.plan.as_deref(), Some("SAVER"));
        assert_eq!(bill.total, Some(92.5));
        assert_eq!(bill.gst, Some(8.41));
        assert_eq!(
            bill.lines.len(),
            4,
            "the second list, with the plan's name, is not counted again"
        );
        assert_eq!(
            bill.lines[0],
            BillLine {
                description: String::from("Daily Charge"),
                quantity: 31.0,
                unit: String::from("Days"),
                rate: 1.0,
                amount: 31.0
            }
        );
        assert_eq!(
            bill.lines[3],
            BillLine {
                description: String::from("Standard Solar Feed in"),
                quantity: 50.0,
                unit: String::from("kWh"),
                rate: -0.05,
                amount: -2.5
            }
        );
        assert!(bill.lines[0].is_daily() && !bill.lines[1].is_daily());
    }

    /// The text of a made-up bill of gas and electricity, laid out as pdf-extract gives a GloBird bill's: each list
    /// has the gas section first.
    const GAS_AND_ELECTRICITY: &str = "Invoice number 1000002
Billing Period: 01-Jan-2027 to 28-Jan-2027 (28 days) ,Total Usage: 1000.00 MJ
Gas Daily Charge 01-Jan - 28-Jan 28 Days x $0.50000 $14.00
Gas Usage Summer - Step 1 01-Jan - 28-Jan 1000.00 MJ x $0.03000 $30.00
TOTAL (including $4.00 GST) $44.00
Billing Period: 01-Jan-2027 to 31-Jan-2027 (31 days) ,Total Usage: 310.00 kWh ,Total Solar Generation: 50.00 kWh
Daily Charge 01-Jan - 31-Jan 31 Days x  $1.00000 $31.00
Peak Usage 01-Jan - 31-Jan 10.00 kWh x  $0.40000 $4.00
Offpeak Usage 01-Jan - 31-Jan 300.00 kWh x  $0.20000 $60.00
Standard Solar Feed in 01-Jan - 31-Jan 50.00 kWh x -$0.05000 -$2.50
TOTAL (including $8.41 GST) $92.50
Gas Charges for Period 01-Jan-2027 to 28-Jan-2027
Gas Usage Summer - Step 1 SAVER 01-Jan - 28-Jan 1000.00 MJ x $0.03000 $30.00
Gas Daily Charge SAVER 01-Jan - 28-Jan 28 Days x $0.50000 $14.00
TOTAL (including $4.00 GST) $44.00
Electricity Charges for Period 01-Jan-2027 to 31-Jan-2027
Peak Usage SAVER 01-Jan - 31-Jan 10.00 kWh x  $0.40000 $4.00
Offpeak Usage SAVER 01-Jan - 31-Jan 300.00 kWh x  $0.20000 $60.00
Standard Solar Feed in SAVER 01-Jan - 31-Jan 50.00 kWh x -$0.05000 -$2.50
Daily Charge SAVER 01-Jan - 31-Jan 31 Days x  $1.00000 $31.00
TOTAL (including $8.41 GST) $92.50
";

    #[test]
    fn a_bill_of_gas_too_is_read_for_its_electricity_alone() {
        let bill = parse_bill(GAS_AND_ELECTRICITY);
        let gas_left_out = parse_bill(TEXT);
        assert_eq!(bill.lines, gas_left_out.lines);
        assert_eq!(bill.period, gas_left_out.period);
        assert_eq!(bill.total, Some(92.5));
        assert_eq!(bill.gst, Some(8.41));
        assert_eq!(bill.plan.as_deref(), Some("SAVER"));
    }

    #[test]
    fn text_which_is_not_a_bill_has_nothing() {
        assert_eq!(
            parse_bill("Welcome to GloBird\nYour plan starts today"),
            Bill::default()
        );
    }
}
