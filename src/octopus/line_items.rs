use std::sync::Arc;

use bruce_graphql::GraphQLCaller;
use bruce_graphql::types::{Date, DateTime};
use indexmap::IndexMap;

use crate::CacheConfig;

use super::graphql::meter;
use super::meter::{AgreementLineItems, MeterType, Tariff};
use super::meter_agreement::MeterAgreementDataSet;
use super::property::PropertyDataSet;

pub type LineItem = meter::electricity_agreement_line_items::LineItemType;

/// The tariff of one agreement and its line items in a period.
#[derive(Debug)]
pub struct TariffLineItems {
    pub tariff: Tariff,
    pub line_items: Vec<LineItem>,
}

/// The line items for a period, for each agreement in scope during that period.
pub struct LineItemDataSet {
    /// The tariff and line items for each agreement which has line items in the period, indexed by agreement id.
    pub agreements: IndexMap<String, TariffLineItems>,
}

impl LineItemDataSet {
    pub async fn new(
        account_number: &String,
        meter_type: &MeterType,
        is_export: bool,
        start_date: &Date,
        end_date: &Date,
        billing_timezone: &time_tz::Tz,
        cache_config: &Arc<CacheConfig>,
        request_manager: &impl GraphQLCaller,
    ) -> anyhow::Result<Self> {
        let properties = PropertyDataSet::new(account_number, cache_config, request_manager).await?;
        let meter_agreements = MeterAgreementDataSet::new(
            cache_config,
            request_manager,
            &account_number,
            &properties.meter_node_ids,
        )
        .await?;

        let start_date_time = start_date.at_midnight(billing_timezone);
        let end_date_time = end_date.at_next_midnight(billing_timezone);

        let in_scope_agreements =
            meter_agreements.get_in_scope(meter_type, is_export, &start_date_time, &end_date_time);

        let mut agreements = IndexMap::new();

        for agreement in in_scope_agreements {
            let in_scope_items = Self::agreement_line_items(
                cache_config,
                request_manager,
                account_number,
                meter_type,
                &agreement.agreement_id,
                &start_date_time,
                &end_date_time,
                billing_timezone,
            )
            .await?;

            if !in_scope_items.is_empty() {
                agreements.insert(
                    agreement.agreement_id,
                    TariffLineItems {
                        tariff: agreement.tariff,
                        line_items: in_scope_items,
                    },
                );
            }
        }

        Ok(Self { agreements })
    }

    /// The line items for one agreement which start in the given period.
    async fn agreement_line_items(
        cache_config: &CacheConfig,
        request_manager: &impl GraphQLCaller,
        account_number: &String,
        meter_type: &MeterType,
        agreement_id: &String,
        start_date_time: &DateTime,
        end_date_time: &DateTime,
        billing_timezone: &time_tz::Tz,
    ) -> anyhow::Result<Vec<LineItem>> {
        let mut in_scope_items = Vec::new();
        let mut bucket_date = start_date_time.to_date();
        loop {
            let line_items = AgreementLineItems::new(
                cache_config,
                request_manager,
                account_number.clone(),
                meter_type,
                agreement_id.clone(),
                &bucket_date,
                billing_timezone,
            )
            .await?;

            for (_cursor, (_, item)) in line_items.line_items {
                if &item.start_at_ >= end_date_time {
                    // thats it

                    return Ok(in_scope_items);
                }
                if &item.start_at_ >= start_date_time {
                    in_scope_items.push(item);
                }
            }

            bucket_date = line_items.end_date;

            if *bucket_date > end_date_time.date() {
                return Ok(in_scope_items);
            }
        }
    }
}
