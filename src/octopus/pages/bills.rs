use std::sync::Arc;

use anyhow::anyhow;
use dioxus::desktop::LogicalPosition;
use dioxus::prelude::*;
use time_tz::Tz;

use crate::components::app::MAIN_CSS;
use crate::components::child_windows;
use crate::octopus::bill::{AbstractBill, BillDataSet, BillTransactionBreakDown};
use crate::octopus::context::OctopusContext;
use crate::views::help_panel::HelpPanel;
use dioxus_free_icons::{Icon, icons::ld_icons::LdCircleHelp};

/// Where the bills' windows open in the main window, just right of the column of bill references, so that the others
/// can still be clicked, and below the navigation bar: the right of the column and the top of the page area.
const ANCHOR: &str = "
    const rights = [...document.querySelectorAll('.bill-ref')].map((cell) => cell.getBoundingClientRect().right);
    const page = document.querySelector('.app-page');
    return [rights.length ? Math.max(...rights) : 0, page ? page.getBoundingClientRect().top : 0];
";
/// The space between the column of bill references and a bill's window.
const ANCHOR_GAP: f64 = 8.0;

#[derive(Props, Clone, PartialEq)]
pub struct BillsPageProps {
    path: Vec<String>,
}

/// The list of bills. Each bill opens in a window of its own, see child_windows, so that the list stays where it was
/// behind it, and bills can be compared side by side.
#[component]
pub fn BillsPage(props: BillsPageProps) -> Element {
    let _ = props;
    let context = use_context::<Arc<OctopusContext>>();

    // Fetch the list of all bills, this runs once when the component is mounted.
    // The bills are shared with the bill windows, so they are held in an Arc.
    let bills_resource = use_resource({
        let context = context.clone();
        move || {
            let context = context.clone();
            async move {
                BillDataSet::new(&context.account_id, &context.cache_config, &*context.request_manager)
                    .await
                    .map(Arc::new)
            }
        }
    });

    let open_bill = use_callback({
        let context = context.clone();
        move |bill_id: String| {
            let Some(Ok(bills)) = &*bills_resource.peek() else {
                return;
            };
            let title = match find_bill(&bill_id, bills) {
                Some(bill) => {
                    let bill = bill.as_bill_interface();
                    format!("Bill {} ({} to {})", bill.id_, bill.from_date_, bill.to_date_)
                }
                None => format!("Bill {bill_id}"),
            };
            let (context, bills) = (context.clone(), bills.clone());
            spawn(async move {
                let anchor = document::eval(ANCHOR)
                    .join::<(f64, f64)>()
                    .await
                    .ok()
                    .map(|(x, y)| LogicalPosition::new(x + ANCHOR_GAP, y));
                child_windows::open(&bill_id, title, anchor, || {
                    VirtualDom::new_with_props(
                        BillWindow,
                        BillWindowProps {
                            context,
                            bill_id: bill_id.clone(),
                            bills,
                        },
                    )
                });
            });
        }
    });

    match &*bills_resource.read() {
        None => rsx! {
            div { "Loading Bills for account {context.account_id}..." }
        },
        Some(Err(error)) => Err(anyhow!("Failed to load bills: {error:#}"))?,
        Some(Ok(bills)) => {
            rsx! {
                table {
                    {AbstractBill::gui_summary_header()?}
                    // The most recent first.
                    for (_id , (attributes , bill)) in bills.data_set.data.iter().rev() {
                        {bill.gui_summary_line(attributes, open_bill)?}
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone)]
struct BillWindowProps {
    context: Arc<OctopusContext>,
    bill_id: String,
    bills: Arc<BillDataSet>,
}

impl PartialEq for BillWindowProps {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.context, &other.context)
            && self.bill_id == other.bill_id
            && Arc::ptr_eq(&self.bills, &other.bills)
    }
}

/// The root of a bill's window, which has its own VirtualDom, so it provides the OctopusContext and the stylesheet
/// itself. Escape closes it.
#[component]
fn BillWindow(props: BillWindowProps) -> Element {
    use_context_provider(|| props.context.clone());
    child_windows::use_child_window();
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
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        crate::components::tooltips::Tooltips {}
        ErrorBoundary {
            handle_error: |errors: ErrorContext| rsx! {
                div { class: "error", "Failed to show the bill" }
                pre { "{errors:?}" }
            },
            div { class: "bill-window-layout",
                div { class: "bill-window",
                    if !show_help() {
                        button {
                            r#type: "button",
                            class: "nav-icon-button help-reopen",
                            title: "Show the help",
                            onclick: move |_| {
                                crate::help::set_shown(true);
                                show_help.set(true);
                            },
                            Icon { icon: LdCircleHelp, width: 16, height: 16 }
                        }
                    }
                    BillDetail {
                        account_id: props.context.account_id.clone(),
                        bill_id: props.bill_id.clone(),
                        bills: props.bills.clone(),
                        billing_timezone: props.context.billing_timezone,
                    }
                }
                if show_help() {
                    HelpPanel {
                        topic: String::from("octopus/bill.md"),
                        on_close: move |_| {
                            crate::help::set_shown(false);
                            show_help.set(false);
                        },
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone)]
struct BillDetailProps {
    account_id: String,
    bill_id: String,
    bills: Arc<BillDataSet>,
    billing_timezone: &'static Tz,
}

impl PartialEq for BillDetailProps {
    fn eq(&self, other: &Self) -> bool {
        self.account_id == other.account_id
            && self.bill_id == other.bill_id
            && Arc::ptr_eq(&self.bills, &other.bills)
            && std::ptr::eq(self.billing_timezone, other.billing_timezone)
    }
}

#[component]
fn BillDetail(props: BillDetailProps) -> Element {
    let context = use_context::<Arc<OctopusContext>>();

    // Fetch all transactions for this bill.
    let transactions_resource = use_resource({
        let props = props.clone();
        let context = context.clone();
        move || {
            let props = props.clone();
            let context = context.clone();
            async move {
                BillTransactionBreakDown::for_statement(
                    props.account_id,
                    props.bill_id,
                    props.billing_timezone,
                    &context.cache_config,
                    &*context.request_manager,
                )
                .await
            }
        }
    });

    let Some(bill) = find_bill(&props.bill_id, &props.bills) else {
        return rsx! {
            {format!("No such bill {}", props.bill_id)}
        };
    };

    match &*transactions_resource.read() {
        None => rsx! {
            {format!("Loading transactions for bill {}...", props.bill_id)}
        },
        Some(Err(error)) => Err(anyhow!(
            "Failed to load transactions for bill {}: {error:#}",
            props.bill_id
        ))?,
        Some(Ok(bill_transactions)) => bill.gui_display(bill_transactions),
    }
}

fn find_bill<'a>(bill_id: &String, bills: &'a BillDataSet) -> Option<&'a AbstractBill> {
    bills.data_set.data.get(bill_id).map(|(_attributes, bill)| bill)
}
