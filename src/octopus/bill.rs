use bruce_graphql::types::Date;
use dioxus::prelude::*;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;

use crate::data_set::{DataSetAttributes, ListDataSet, OrderedListDataSet};
use crate::octopus::decimal::Decimal;
use crate::octopus::graphql::AccountStatementStatus;
use crate::octopus::graphql::bill::get_statement_transactions::{AbstractTransactionType, Consumption};
use crate::util::as_decimal;
use crate::{CacheConfig, NONE, NULL};

use super::graphql::bill;
use super::line_items::{LineItemDataSet, TariffLineItems};
use super::meter::MeterType;
use bill::get_statement_transactions::{Charge, TransactionType};
use bruce_graphql::{GraphQLCaller, GraphQLQueryBuilder};

// const one_hundred: Decimal = Decimal::new(100, 0);
// const format: time::format_description = time::format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]").unwrap();

pub type BillType = super::graphql::BillTypeEnum;

impl BillType {
    fn as_str(&self) -> &'static str {
        match self {
            BillType::Statement => "Statement",
            BillType::Invoice => "Invoice",
            BillType::CreditNote => "CreditNote",
            BillType::PreKraken => "PreKraken",
            BillType::Collective => "Collective",
        }
    }
}

pub type AbstractBill = crate::octopus::graphql::bill::get_bills::BillInterface;

impl AbstractBill {
    pub fn gui_summary_header() -> Element {
        rsx! {
            tr {

                th { colspan: 5, "" }
                th { class: "header", "Balance" }
                th { class: "header", colspan: 3, "Charges" }
                th { class: "header", colspan: 3, "Credits" }
                th { class: "header", "Balance" }
            }
            tr { class: "header",
                th { "Date" }
                th { "Ref" }
                th { "From" }
                th { "To" }
                th { "Type" }
                th { "b/f" }
                th { "Net" }
                th { "Tax" }
                th { "Gross" }
                th { "Net" }
                th { "Tax" }
                th { "Gross" }
                th { "c/f" }
            }
        }
    }

    /// A row of the bill list, whose id calls on_open with the bill's id when clicked.
    pub fn gui_summary_line(&self, attributes: &DataSetAttributes, on_open: EventHandler<String>) -> Element {
        let abstract_bill = self.as_bill_interface();
        let row_class = if attributes.cached { "cached" } else { "fetched" };

        let detail = match self {
            AbstractBill::StatementType(statement) => {
                rsx! {
                    td { class: "numeric", "{as_decimal(statement.opening_balance_, 2)}" }
                    td { class: "numeric", "{as_decimal(statement.total_charges_.net_total_, 2)}" }
                    td { class: "numeric", "{as_decimal(statement.total_charges_.tax_total_, 2)}" }
                    td { class: "numeric", "{as_decimal(statement.total_charges_.gross_total_, 2)}" }
                    td { class: "numeric", "{as_decimal(statement.total_credits_.net_total_, 2)}" }
                    td { class: "numeric", "{as_decimal(statement.total_credits_.tax_total_, 2)}" }
                    td { class: "numeric", "{as_decimal(statement.total_credits_.gross_total_, 2)}" }
                    td { class: "numeric", "{as_decimal(statement.closing_balance_, 2)}" }
                }
            }
            AbstractBill::PreKrakenBillType(_) => rsx! {},
            AbstractBill::CollectiveBillType(_) => rsx! {},
            AbstractBill::PeriodBasedDocumentType(period_based_document) => {
                rsx! {
                    td {}
                    td { class: "numeric",
                        "{as_decimal(period_based_document.total_charges_.net_total_, 2)}"
                    }
                    td { class: "numeric",
                        "{as_decimal(period_based_document.total_charges_.tax_total_, 2)}"
                    }
                    td { class: "numeric",
                        "{as_decimal(period_based_document.total_charges_.gross_total_, 2)}"
                    }
                    td { class: "numeric",
                        "{as_decimal(period_based_document.total_credits_.net_total_, 2)}"
                    }
                    td { class: "numeric",
                        "{as_decimal(period_based_document.total_credits_.tax_total_, 2)}"
                    }
                    td { class: "numeric",
                        "{as_decimal(period_based_document.total_credits_.gross_total_, 2)}"
                    }
                    td {}
                }
            }
            AbstractBill::InvoiceType(invoice) => {
                rsx! {
                    td {}
                    td {}
                    td {}
                    td {}
                    td {}
                    td {}
                    td { class: "numeric", "{as_decimal(invoice.gross_amount_, 2)}" }
                    td {}
                }
            }
        };

        let id = abstract_bill.id_.clone();
        let issued_date = if let Some(date) = &abstract_bill.issued_date_ {
            date.to_string()
        } else {
            "N/A".to_string()
        };
        rsx! {
            tr { class: row_class,
                td { "{issued_date}" }
                // The bill's window opens to the right of this column, see BillsPage.
                td { class: "bill-ref",
                    div {
                        class: "link",
                        onclick: move |_| on_open.call(id.clone()),
                        "{abstract_bill.id_}"
                    }
                }
                td { "{abstract_bill.from_date_}" }
                td { "{abstract_bill.to_date_}" }
                td { "{abstract_bill.bill_type_.as_str()}" }
                {detail}
            }
        }
    }

    pub fn gui_display(&self, transactions: &Vec<BillTransactionBreakDown>) -> Element {
        println!("\n AbstractBill::gui_display: self={:#?}", self);
        println!("\n AbstractBill::gui_display: transactions={:#?}", transactions);

        let abstract_bill = self.as_bill_interface();
        let mut total_charges = TotalCharges::new();
        let mut parts = Vec::new();

        let transaction_lines = rsx! {
            for txn in transactions {
                {txn.gui_summary_line(&mut total_charges)}
            }
        };

        println!("\n AbstractBill::gui_display: total_charges={:#?}", total_charges);

        let totals = if total_charges.inputs() > 1 {
            let rate = Decimal::from(total_charges.gross_usage()) / total_charges.units();
            rsx! {
                tr {
                    td {}
                    td { "TOTALS" }
                    td { colspan: 10, "" }
                }
                tr { class: "derived",
                    td {}
                    td { "Electricity Import" }
                    td { "" }
                    td { class: "numeric", {{ as_decimal(total_charges.net(), 2) }} }
                    td { colspan: 2, "" }
                    td { class: "numeric", {{ as_decimal(total_charges.gross(), 2) }} }
                    td { colspan: 4, "" }
                    td { class: "numeric", {{ as_decimal(total_charges.gross_usage(), 2) }} }
                    td { class: "numeric", {{ format!("{}", total_charges.units()) }} }
                    td { class: "numeric", {{ format!("{:>10.3}", rate) }} }
                }
            }
        } else {
            rsx! {}
        };

        let issued_date = if let Some(date) = &abstract_bill.issued_date_ {
            date.to_string()
        } else {
            "N/A".to_string()
        };
        // A statement which Octopus has not issued yet, which it does not show in its own app.
        let open = matches!(self, AbstractBill::StatementType(statement) if statement.status_ != AccountStatementStatus::Closed);
        parts.push(rsx!{
            h1 { "Energy Account Statement" }
            if open {
                p { class: "open-bill-note",
                    "This bill is still open. Octopus has not issued it yet, so it is not shown in the Octopus app, and its charges, dates and reference may change until it is issued. It may also be replaced by a different bill, e.g. if its period is closed early."
                }
            }
            table { class: "display",
                tr {
                    th { class: "row-header", "Date:" }
                    td { "{issued_date}" }
                }
                tr {
                    th { class: "row-header", "Ref:" }
                    td { "{abstract_bill.id_}" }
                }
                tr {
                    th { class: "row-header", "From:" }
                    td { "{abstract_bill.from_date_}" }
                }
                tr {
                    th { class: "row-header", "To:" }
                    td { "{abstract_bill.to_date_}" }
                }
            }

            h2 { "Summary of Charges" }

            table {
                {BillTransactionBreakDown::gui_summary_headers()}

                {transaction_lines}
                {totals}
            }

            if total_charges.units().is_positive() {
                h2 { "Detailed Breakdown (excluding VAT)" }
                for transaction in transactions {
                    {transaction.gui_display()}
                }
            }
        });

        rsx! {
            tr {
                for item in parts {
                    {item}
                }
            }
        }
    }
}

mod total_charges_module {
    use crate::octopus::decimal::Decimal;

    #[derive(Debug)]
    pub struct TotalCharges {
        gross: i32,
        gross_usage: i32,
        net: i32,
        units: Decimal,
        inputs: i32,
    }

    impl TotalCharges {
        pub fn new() -> Self {
            TotalCharges {
                gross: 0,
                gross_usage: 0,
                net: 0,
                units: Decimal::new(0, 0),
                inputs: 0,
            }
        }

        pub fn gross(&self) -> i32 {
            self.gross
        }

        pub fn gross_usage(&self) -> i32 {
            self.gross_usage
        }

        pub fn net(&self) -> i32 {
            self.net
        }

        pub fn units(&self) -> &Decimal {
            &self.units
        }

        pub fn inputs(&self) -> i32 {
            self.inputs
        }

        pub fn accumulate_line_item(
            &mut self,
            transaction: &crate::octopus::graphql::bill::get_statement_transactions::AbstractTransactionType,
            consumption: &crate::octopus::graphql::bill::get_statement_transactions::Consumption,
            net_factor: f64,
        ) {
            self.gross += *&transaction.amounts_.gross_;
            self.gross_usage += consumption.usage_cost_;

            self.net += (*&transaction.amounts_.gross_ as f64 / net_factor) as i32;

            self.units += consumption.quantity_;
            self.inputs += 1;
        }
    }
}

use total_charges_module::TotalCharges;

impl AbstractTransactionType {}

impl Charge {}

impl TransactionType {
    pub fn gui_break_down_line_headers() -> Element {
        rsx! {
            tr {
                th { colspan: 2, "From" }
                th { colspan: 2, "To" }
                th { "Amount" }
                th { "Units" }
                th { "p/unit" }
                th { "Settlement Unit" }
            }
        }
    }

    pub fn gui_summary_headers() -> Element {
        rsx! {
            tr {
                th { colspan: 11, "" }
                th { colspan: 3, class: "header", "Net" }
                th { colspan: 3, class: "header", "Gross" }
            }
            tr { class: "header",
                th { "id" }
                th { "Description" }
                th { "Posted" }
                th { "Net" }
                th { "Tax %" }
                th { "Tax" }
                th { "Total" }
                th { "Balance" }
                th { "From" }
                th { "To" }
                th { "Units" }
                th { "Supply" }
                th { "Usage" }
                th {
                    // title: "Average cost per unit including standing charge and VAT",
                    "p/unit"
                }
                th { "Supply" }
                th { "Usage" }
                th {
                    // title: "Average cost per unit including standing charge and VAT",
                    "p/unit"
                }
                th { "Note" }
            }
        }
    }
}

///
/// BillTransactionBreakDown represents a BillTransaction and its associated line items.
/// It is an enum that can hold either a Charge and a non-zero length list of line items
/// or an Abstract transaction type.
///
/// Note that a Charge with no line items is represented as a BillTransactionBreakDown::Abstract with the Charge as the transaction type.
#[derive(Debug)]
pub enum BillTransactionBreakDown {
    Charge {
        // transaction: Charge,
        transaction: AbstractTransactionType,
        consumption: Consumption,
        is_export: bool,
        line_item_map: IndexMap<String, TariffLineItems>,
    },
    Abstract {
        transaction: TransactionType,
    },
}

impl BillTransactionBreakDown {
    /// The transactions of a statement, with the line items making up each charge.
    pub async fn for_statement(
        account_number: String,
        statement_id: String,
        billing_timezone: &time_tz::Tz,
        cache_config: &Arc<CacheConfig>,
        request_manager: &impl GraphQLCaller,
    ) -> anyhow::Result<Vec<BillTransactionBreakDown>> {
        let mut result = Vec::new();
        let transactions =
            StatementTransactionDataSet::new(account_number.clone(), statement_id, cache_config, request_manager)
                .await?;

        for (_key, transaction) in transactions.data_set.data {
            if let TransactionType::Charge(charge_ref) = &transaction
                && charge_ref.consumption_.is_some()
            {
                // print the line items making up this charge
                //println!("Get line items {:?} - {:?}",  &consumption.start_date_, &consumption.end_date_);

                if let TransactionType::Charge(charge) = transaction {
                    if let Some(consumption) = &charge.consumption_ {
                        let meter_type = match charge.as_transaction_type().title_.as_str() {
                            "Gas" => MeterType::Gas,
                            "Electricity" => MeterType::Electricity,
                            _ => panic!("Unknown consumption type"),
                        };

                        let line_items = LineItemDataSet::new(
                            &account_number,
                            &meter_type,
                            charge.is_export_,
                            &consumption.start_date_,
                            &consumption.end_date_,
                            billing_timezone,
                            cache_config,
                            request_manager,
                        )
                        .await?;

                        result.push(BillTransactionBreakDown::from_charge(charge, line_items.agreements));
                    }
                }
            } else {
                result.push(BillTransactionBreakDown::from_abstract(transaction));
            }
        }

        Ok(result)
    }

    pub fn from_charge(charge: Charge, line_item_map: IndexMap<String, TariffLineItems>) -> Self {
        if !line_item_map.is_empty()
            && let Some(consumption) = charge.consumption_
        {
            return BillTransactionBreakDown::Charge {
                transaction: charge.transaction_type_,
                consumption,
                is_export: charge.is_export_,
                line_item_map,
            };
        }
        return BillTransactionBreakDown::Abstract {
            transaction: TransactionType::Charge(charge),
        };
    }

    pub fn from_abstract(transaction: TransactionType) -> Self {
        BillTransactionBreakDown::Abstract { transaction }
    }

    pub fn gui_summary_line(&self, total_charges: &mut TotalCharges) -> Element {
        match self {
            BillTransactionBreakDown::Charge {
                transaction,
                consumption,
                is_export,
                line_item_map: _,
            } => {
                // transaction.gui_summary_line(total_charges)

                let mut parts = Vec::new();

                parts.push(rsx!(
                    td { class: "link", "{transaction.id_.as_str()}" }

                )?);
                parts.push(if *is_export {
                    rsx!(
                        td {
                            {transaction.title_.as_str()}
                            " Export"
                        }
                    )
                } else {
                    rsx!(
                        td { {transaction.title_.as_str()} }
                    )
                }?);
                parts.push(rsx!(
                    td { "{transaction.posted_date_}" }
                )?);

                let vat_rate = if transaction.amounts_.tax_ > 0 {
                    10000 * transaction.amounts_.tax_ / transaction.amounts_.net_
                } else {
                    0
                };
                let net_factor = 1.0 + transaction.amounts_.tax_ as f64 / transaction.amounts_.net_ as f64;

                parts.push(rsx!(
                    td { class: "numeric", {as_decimal(transaction.amounts_.net_, 2)} }
                    td { class: "numeric derived", {as_decimal(vat_rate, 2)} }
                    td { class: "numeric", {as_decimal(transaction.amounts_.tax_, 2)} }
                    td { class: "numeric", {as_decimal(transaction.amounts_.gross_, 2)} }
                    td { class: "numeric", {as_decimal(transaction.balance_carried_forward_, 2)} }
                )?);

                let net_supply_charge = (consumption.supply_charge_ as f64 / net_factor) as i32;
                let net_usage_cost = (consumption.usage_cost_ as f64 / net_factor) as i32;

                let (gross_unit_cost, net_unit_cost) = if consumption.quantity_.is_non_zero() {
                    (
                        format!(
                            "{:>12.4}",
                            Decimal::from(transaction.amounts_.gross_ - consumption.supply_charge_)
                                / consumption.quantity_
                        ),
                        format!(
                            "{:>12.4}",
                            Decimal::from(transaction.amounts_.net_ - net_supply_charge) / consumption.quantity_
                        ),
                    )
                } else {
                    (String::new(), String::new())
                };

                parts.push(rsx!(
                    td { {format!("{}", consumption.start_date_)} }
                    td { {format!("{}", consumption.end_date_)} }
                    td { class: "numeric", {format!("{:>12.4}", consumption.quantity_)} }

                    td { class: "numeric derived", {as_decimal(net_supply_charge, 2)} }
                    td { class: "numeric derived", {as_decimal(net_usage_cost, 2)} }
                    td { class: "numeric derived", {net_unit_cost} }

                    td { class: "numeric", {as_decimal(consumption.supply_charge_, 2)} }
                    td { class: "numeric", {as_decimal(consumption.usage_cost_, 2)} }
                    td { class: "numeric derived", {gross_unit_cost} }
                )?);

                if *is_export {
                } else {
                    if transaction.title_.eq("Electricity") {
                        total_charges.accumulate_line_item(transaction, consumption, net_factor);
                        // total_charges.gross += *&transaction.amounts_.gross_;
                        // total_charges.gross_supply = consumption.supply_charge_;
                        // total_charges.gross_usage += consumption.usage_cost_;

                        // total_charges.net += (*&transaction.amounts_.gross_ as f64 / net_factor) as i32;
                        // total_charges.net_supply = (consumption.supply_charge_ as f64 / net_factor) as i32;
                        // total_charges.net_usage += (consumption.usage_cost_ as f64 / net_factor) as i32;

                        // total_charges.units += consumption.quantity_;
                    }
                }

                if let Some(note) = &transaction.note_ {
                    let note = note.trim();
                    parts.push(rsx!(
                        td { {note} }
                    )?);
                } else {
                    parts.push(rsx!(
                        td { "" }
                    )?);
                }
                rsx! {
                    tr {
                        for item in parts {
                            {item}
                        }
                    }
                }
            }
            BillTransactionBreakDown::Abstract { transaction } => {
                // transaction.gui_summary_line(total_charges)
                let abstract_txn = transaction.as_transaction_type();
                let mut parts = Vec::new();

                parts.push(rsx!(
                    td { class: "link", "{abstract_txn.id_.as_str()}" }

                )?);
                parts.push(rsx!(
                    td { {abstract_txn.title_.as_str()} }
                )?);
                parts.push(rsx!(
                    td { "{abstract_txn.posted_date_}" }
                )?);

                let vat_rate = if abstract_txn.amounts_.tax_ > 0 {
                    10000 * abstract_txn.amounts_.tax_ / abstract_txn.amounts_.net_
                } else {
                    0
                };

                parts.push(rsx!(
                    td { class: "numeric", {as_decimal(-abstract_txn.amounts_.net_, 2)} }
                    td { class: "numeric derived", {as_decimal(vat_rate, 2)} }
                    td { class: "numeric", {as_decimal(-abstract_txn.amounts_.tax_, 2)} }
                    td { class: "numeric", {as_decimal(-abstract_txn.amounts_.gross_, 2)} }
                    td { class: "numeric",
                        {as_decimal(abstract_txn.balance_carried_forward_, 2)}
                    }
                )?);
                parts.push(rsx!(
                    td { "" }
                    td { "" }
                    td { "" }
                    td { "" }
                    td { "" }
                )?);

                if let Some(note) = &abstract_txn.note_ {
                    let note = note.trim();
                    parts.push(rsx!(
                        td { {note} }
                    )?);
                } else {
                    parts.push(rsx!(
                        td { "" }
                    )?);
                }
                rsx! {
                    tr {
                        for item in parts {
                            {item}
                        }
                    }
                }
            }
        }
        // self.transaction.gui_summary_line(total_charges)
    }

    pub fn gui_summary_headers() -> Element {
        TransactionType::gui_summary_headers()
    }

    pub fn gui_display(&self) -> Element {
        let one_hundred = Decimal::new(100, 0);
        // let format = time::format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]").unwrap();
        let date_format = time::format_description::parse("[year]-[month]-[day]").unwrap();
        let time_format = time::format_description::parse("           [hour]:[minute]:[second]").unwrap();

        let mut parts = Vec::new();

        println!("\nBillTransactionBreakDown::gui_display: transaction={:#?}", self);

        match self {
            BillTransactionBreakDown::Charge {
                transaction,
                consumption,
                is_export: _,
                line_item_map,
            } => {
                for (_agreement_id, TariffLineItems { tariff, line_items }) in line_item_map {
                    let mut amount_map = BTreeMap::new();
                    let mut total_amount = Decimal::new(0, 0);
                    let mut total_units = Decimal::new(0, 0);

                    let txn = transaction.as_transaction_type();

                    println!();
                    parts.push(rsx! {
                        {tariff.gui_display()}
                    }?);

                    let mut sub_parts = Vec::new();

                    sub_parts.push(TransactionType::gui_break_down_line_headers()?);
                    let mut prev = None;
                    for item in line_items {
                        let amount = item.net_amount_ / one_hundred;

                        total_amount += amount;
                        total_units += item.number_of_units_;

                        let unit_cost = (if item.number_of_units_.is_non_zero() {
                            item.net_amount_ / item.number_of_units_
                        } else {
                            item.net_amount_
                        })
                        .round_dp(2);

                        let (from_date, from_time) = if let Some(prev) = prev {
                            if prev == item.start_at_.date() {
                                ("".to_string(), item.start_at_.format(&time_format).unwrap())
                            } else {
                                (
                                    item.start_at_.format(&date_format).unwrap(),
                                    item.start_at_.format(&time_format).unwrap(),
                                )
                            }
                        } else {
                            (
                                item.start_at_.format(&date_format).unwrap(),
                                item.start_at_.format(&time_format).unwrap(),
                            )
                        };

                        let (to_date, to_time) = if item.end_at_.date() == item.start_at_.date() {
                            ("".to_string(), item.end_at_.format(&time_format).unwrap())
                        } else {
                            (
                                item.end_at_.format(&date_format).unwrap(),
                                item.end_at_.format(&time_format).unwrap(),
                            )
                        };

                        if item.number_of_units_.is_positive() {
                            let key = unit_cost.round_dp(2);
                            let formatted_key = key.to_string();
                            if let Some((_, total_amount, total_units)) = amount_map.get(&key) {
                                amount_map.insert(
                                    key,
                                    (
                                        formatted_key,
                                        amount + *total_amount,
                                        item.number_of_units_ + *total_units,
                                    ),
                                );
                            } else {
                                amount_map.insert(key, (formatted_key, amount, item.number_of_units_));
                            }
                        }

                        prev = Some(item.start_at_.date());

                        sub_parts.push(rsx! {
                            tr {
                                td { {from_date} }
                                td { {from_time} }
                                td { {to_date} }
                                td { {to_time} }
                                td { class: "numeric", {amount.round_dp(3).to_string()} }
                                td { class: "numeric",
                                    {item.number_of_units_.round_dp(4).to_string()}
                                }
                                td { class: "numeric derived", {unit_cost.round_dp(2).to_string()} }
                                td { {format!("{:.4}", item.settlement_unit_)} }
                            }
                        }?);
                    }

                    sub_parts.push(rsx! {
                        tr {
                            td { colspan: 4, "Total" }
                            td { class: "numeric derived", {format!("{:.3}", total_amount)} }
                            td { class: "numeric derived", {format!("{:.4}", total_units)} }
                        }
                    }?);

                    let net_factor = 1.0 + transaction.amounts_.tax_ as f64 / transaction.amounts_.net_ as f64;
                    let net_supply_charge = (consumption.supply_charge_ as f64 / net_factor) as i32;

                    sub_parts.push(rsx! {
                        tr {
                            td { colspan: 4, "Total from Statement" }
                            td { class: "numeric derived",
                                {as_decimal(txn.amounts_.net_ - net_supply_charge, 2)}
                            }
                            td { class: "numeric", {format!("{:.4}", consumption.quantity_)} }
                        }
                    }?);

                    let mut consumption_analysis = None;

                    if line_items.len() > 0 {
                        let start_date = line_items.get(0).unwrap().start_at_.date();
                        let end_date = line_items.get(line_items.len() - 1).unwrap().end_at_.date();
                        let days = end_date.to_julian_day() - start_date.to_julian_day();

                        if let Some(standing_charge_rate) = tariff.standing_charge() {
                            let standing_charge =
                                Decimal::new((standing_charge_rate * (10000 * days) as f64) as i64, 6);
                            sub_parts.push(rsx! {
                                tr {
                                    td { colspan: 4,
                                        {
                                            format!(
                                                "Standing charge from Tariff ({} days @ {:.3})",
                                                days,
                                                standing_charge_rate,
                                            )
                                        }
                                    }
                                    td { class: "numeric derived",
                                        {format!("{:.3}", standing_charge)}
                                    }
                                }
                                // tr {

                                //     td { colspan: 4, "Total" }
                                //     td { class: "numeric derived",
                                //         {format!("{:.3}", total_amount + standing_charge)}
                                //     }
                                // }
                            }?);
                        } else {
                            sub_parts.push(rsx! {
                                tr {
                                    td { colspan: 4,
                                        {format!("Standing charge from Tariff ({} days @ ", days)}
                                        span { class: "invalid", {NULL} }
                                        ")"
                                    }
                                    td { class: "numeric invalid", {NONE} }
                                }
                            }?);
                        }

                        if !amount_map.is_empty() {
                            let standing_charge = Decimal::new(net_supply_charge as i64, 2);
                            consumption_analysis = Some(rsx! {
                                h4 { "Consumption Analysis" }
                                table { class: "display",
                                    tr {
                                        th { "Unit Rate" }
                                        th { "Cost" }
                                        th { "Units" }
                                        th { "% Cost" }
                                        th { "% Units" }
                                        th { "% Bill" }
                                    }

                                    for (_key , (formatted_key , amount , units)) in amount_map {
                                        tr {
                                            td { class: "numeric", "{formatted_key}" }
                                            td { class: "numeric", {format!("{:.2}", amount)} }
                                            td { class: "numeric derived", {format!("{:.2}", units)} }
                                            td { class: "numeric derived",
                                                {format!("{:.2}", one_hundred * amount / total_amount)}
                                            }
                                            td { class: "numeric derived",
                                                {format!("{:.2}", one_hundred * units / total_units)}
                                            }
                                            td { class: "numeric derived",
                                                {format!("{:.2}", one_hundred * amount / (standing_charge + total_amount))}
                                            }
                                        }
                                    }
                                    tr {
                                        th { class: "row-header", "Standing Charge" }
                                        td { class: "numeric derived",
                                            {format!("{:.2}", standing_charge)}
                                        }
                                        th { colspan: 3, "" }
                                        td { class: "numeric derived",
                                            {
                                                format!(
                                                    "{:.2}",
                                                    one_hundred * standing_charge / (standing_charge + total_amount),
                                                )
                                            }
                                        }
                                    }
                                }
                            }?);
                        }
                    }

                    sub_parts.push(rsx! {
                        tr {
                            td { colspan: 4, "Standing Charge from Statement" }
                            td { class: "numeric derived", {as_decimal(net_supply_charge, 2)} }
                        }
                    }?);

                    parts.push(rsx! {
                        table {
                            for item in sub_parts {
                                {item}
                            }
                        }
                    }?);

                    if let Some(consumption_analysis) = consumption_analysis {
                        parts.push(consumption_analysis);
                    }
                }

                rsx! {
                    tr {
                        for item in parts {
                            {item}
                        }
                    }
                }
            }
            BillTransactionBreakDown::Abstract { transaction: _ } => {
                rsx! {
                    tr {}
                }
            }
        }
    }
}

pub struct BillDataSet {
    pub data_set: OrderedListDataSet<String, Date, AbstractBill>,
}

impl BillDataSet {
    pub async fn new(
        account_number: &String,
        config: &Arc<CacheConfig>,
        request_manager: &impl GraphQLCaller,
    ) -> anyhow::Result<Self> {
        let hash_key = format!("{}#Bills", account_number);

        let initial_query_provider = |opt_last_record: Option<&AbstractBill>| {
            let mut builder = super::graphql::bill::get_bills::Query::builder()
                .with_account_number(account_number.clone())
                .with_include_bills_without_pdf(true)
                .with_include_open_statements(true)
                .with_last(20);

            if let Some(last_record) = opt_last_record {
                // If we ever found ourselves in the position that there are two bills on the same issue date
                // and we fetch one of them as the last item in a query we would never see the second one.
                // By stepping back one day we usually read one bill we already have but we avoid that gap.
                let start_date = last_record.as_bill_interface().from_date_.clone(); //.previous_day();
                builder = builder.with_from_date(start_date);
            }

            builder.build()
        };

        let continuation_query_provider = |last_response: &super::graphql::bill::get_bills::Response| {
            if last_response.account_.bills_.page_info.has_previous_page
                && let Some(start_cursor) = &last_response.account_.bills_.page_info.start_cursor
            {
                Some(
                    super::graphql::bill::get_bills::Query::builder()
                        .with_account_number(account_number.clone())
                        .with_include_bills_without_pdf(true)
                        .with_include_open_statements(true)
                        .with_last(20)
                        .with_before(start_cursor.clone())
                        .build(),
                )
            } else {
                None
            }
        };
        let indexer = |bill: &AbstractBill| {
            let is_final = if let AbstractBill::StatementType(statement) = bill {
                statement.status_ == AccountStatementStatus::Closed
            } else {
                true
            };

            let bill = bill.as_bill_interface();
            (bill.id_.clone(), bill.from_date_.clone(), is_final)
        };
        let response_iterator = |response: super::graphql::bill::get_bills::Response| {
            let it = response.account_.bills_.edges.into_iter().map(|edge| edge.node);
            it
        };

        let data_set = OrderedListDataSet::new(
            &hash_key,
            Duration::from_hours(24),
            initial_query_provider,
            continuation_query_provider,
            response_iterator,
            indexer,
            // An open statement can be replaced by another, e.g. when its period is closed early, and the first is then
            // no longer returned.
            true,
            config,
            request_manager,
        )
        .await?;

        Ok(Self { data_set })
    }
}

pub struct StatementTransactionDataSet {
    pub data_set: ListDataSet<String, TransactionType>,
}

impl StatementTransactionDataSet {
    pub async fn new(
        account_number: String,
        statement_id: String,
        config: &Arc<CacheConfig>,
        request_manager: &impl GraphQLCaller,
    ) -> anyhow::Result<Self> {
        let hash_key = format!("{}#{}#StatementTransactions", account_number, statement_id);

        let initial_query_provider = || {
            super::graphql::bill::get_statement_transactions::Query::builder()
                .with_account_number(account_number.clone())
                .with_statement_id(statement_id.clone())
                .with_transactions_last(20)
                .build()
        };

        let continuation_query_provider =
            |last_response: &super::graphql::bill::get_statement_transactions::Response| {
                if let bill::get_statement_transactions::BillInterface::StatementType(statement) =
                    &last_response.account_.bill_
                {
                    if statement.transactions_.page_info.has_previous_page
                        && let Some(start_cursor) = &statement.transactions_.page_info.start_cursor
                    {
                        Some(
                            super::graphql::bill::get_statement_transactions::Query::builder()
                                .with_account_number(account_number.clone())
                                .with_statement_id(statement_id.clone())
                                .with_transactions_last(20)
                                .with_transactions_before(start_cursor.clone())
                                .build(),
                        )
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
        let indexer = |transaction: &TransactionType| transaction.as_transaction_type().id_.clone();

        let response_iterator = |response: super::graphql::bill::get_statement_transactions::Response| {
            if let bill::get_statement_transactions::BillInterface::StatementType(statement) = response.account_.bill_ {
                let it = statement.transactions_.edges.into_iter().map(|edge| edge.node);
                it
            } else {
                panic!("Bill {} is not a statement", statement_id)
            }
            // response.account_.bills_.edges.into_iter().map(|edge| edge.node)
        };

        let data_set = ListDataSet::new(
            &hash_key,
            None,
            initial_query_provider,
            continuation_query_provider,
            response_iterator,
            indexer,
            config,
            request_manager,
        )
        .await?;

        Ok(Self { data_set })
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn test_bill_deserialize() {
//         let all_json = r#"{
//   "account": {
//     "bills": {
//       "edges": [
//         {
//           "node": {
//             "__typename": "StatementType",
//             "billType": "STATEMENT",
//             "closingBalance": 30711,
//             "fromDate": "2025-01-10",
//             "heldStatus": {
//               "isHeld": false,
//               "reason": null
//             },
//         "pageInfo": {
//           "endCursor": "YXJyYXljb25uZWN0aW9uOjE5",
//           "hasNextPage": true
//         }
//       }
//     }
//   }"#;
//         let response: super::super::graphql::bill::get_bills_and_transactions::Response = serde_json::from_str(all_json).unwrap();

//         serde_json::to_string_pretty(&response);
//     }
// }
