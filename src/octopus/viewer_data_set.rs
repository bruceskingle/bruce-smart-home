use std::time::Duration;

use crate::CacheConfig;
use crate::data_set::SingleRecordDataSet;

use super::graphql::account;
use bruce_graphql::GraphQLCaller;

pub struct ViewerDataSet {
    pub data_set: SingleRecordDataSet<account::viewer::Response>,
    pub selected_account_id: String,
}

impl ViewerDataSet {
    pub async fn new(
        config: &CacheConfig,
        request_manager: &impl GraphQLCaller,
        preferred_account_id: Option<&str>,
    ) -> anyhow::Result<Self> {
        let hash_key = format!("#Viewer");
        let data_set = SingleRecordDataSet::new(
            &hash_key,
            Duration::from_hours(20 * 24), // 30 days
            || Ok(account::viewer::Query::new()),
            config,
            request_manager,
        )
        .await?;

        let account_ids: Vec<String> = data_set
            .data
            .viewer_
            .accounts_
            .iter()
            .map(|account| account.number_.clone())
            .collect();
        let selected_account_id = select_account_id(&account_ids, preferred_account_id)?;

        Ok(Self {
            data_set,
            selected_account_id,
        })
    }

    pub fn get_selected_account_id(&self) -> &String {
        &self.selected_account_id
    }
}

fn select_account_id(account_ids: &[String], preferred_account_id: Option<&str>) -> anyhow::Result<String> {
    let preferred = preferred_account_id.and_then(|preferred| {
        let found = account_ids.iter().find(|account_id| account_id.as_str() == preferred);
        if found.is_none() {
            crate::log_warning!("The Octopus account {preferred} in the profile is not one of this user's accounts, {}, so the first is used",
                account_ids.join(", "));
        }
        found
    });
    preferred
        .or_else(|| account_ids.first())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("No accounts found for this Octopus user"))
}

#[cfg(test)]
mod tests {
    use super::select_account_id;

    #[test]
    fn selects_the_saved_account_when_it_is_available() {
        let accounts = vec!["A-1".to_string(), "A-2".to_string()];
        assert_eq!(select_account_id(&accounts, Some("A-2")).unwrap(), "A-2");
    }

    #[test]
    fn falls_back_to_the_first_account_when_the_saved_account_is_missing() {
        let accounts = vec!["A-1".to_string(), "A-2".to_string()];
        assert_eq!(select_account_id(&accounts, Some("A-3")).unwrap(), "A-1");
    }

    #[test]
    fn errors_when_the_user_has_no_accounts() {
        let error = select_account_id(&[], None).unwrap_err();
        assert_eq!(error.to_string(), "No accounts found for this Octopus user");
    }
}
