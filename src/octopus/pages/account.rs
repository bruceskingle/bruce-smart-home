use dioxus::prelude::*;

use crate::components::error_text::ErrorText;
use crate::octopus::{Profile, graphql::account::viewer::AccountInterface};
use crate::{InitRequested, profile};

#[derive(Props, Clone)]
pub struct AccountRowProps {
    accounts: Vec<AccountInterface>,
    selected_account_id: String,
    profile: Profile,
    profile_name: String,
}

impl PartialEq for AccountRowProps {
    fn eq(&self, other: &Self) -> bool {
        self.accounts
            .iter()
            .map(|account| &account.number_)
            .eq(other.accounts.iter().map(|account| &account.number_))
            && self.selected_account_id == other.selected_account_id
            && self.profile == other.profile
            && self.profile_name == other.profile_name
    }
}

/// The account row of the account page's table: which of the user's accounts the module uses, chosen from a list if
/// there is more than one, see ViewerDataSet.
#[component]
pub fn AccountRow(props: AccountRowProps) -> Element {
    let mut error = use_signal(|| None::<String>);
    let mut init_signal = use_context::<Signal<InitRequested>>();
    let account_ids: Vec<String> = props.accounts.iter().map(|account| account.number_.clone()).collect();
    let valid_account_ids = account_ids.clone();
    let selected_account_id = props.selected_account_id.clone();
    let profile = props.profile.clone();
    let profile_name = props.profile_name.clone();

    rsx! {
        tr {
            th { class: "row-header", "Account" }
            td {
                if props.accounts.len() > 1 {
                    select {
                        class: "account-select",
                        value: "{props.selected_account_id}",
                        onchange: move |event: FormEvent| {
                            let account_number = event.value();
                            if account_number == selected_account_id {
                                return;
                            }
                            if !valid_account_ids.iter().any(|id| id == &account_number) {
                                let message = "The selected Octopus account is not available.".to_string();
                                crate::log_error!("{message}");
                                error.set(Some(message));
                                return;
                            }

                            let updated_profile = Profile {
                                account_number: Some(account_number.clone()),
                                ..profile.clone()
                            };
                            match profile::update_profile(&profile_name, "octopus", &updated_profile) {
                                Ok(()) => {
                                    crate::log_info!("Selected Octopus account {account_number} for profile {profile_name}");
                                    init_signal.set(InitRequested(true));
                                }
                                Err(save_error) => {
                                    let message = format!("Failed to save the selected account: {save_error}");
                                    crate::log_error!("{message}");
                                    error.set(Some(message));
                                }
                            }
                        },
                        for account in &props.accounts {
                            option {
                                value: "{account.number_}",
                                selected: account.number_ == props.selected_account_id,
                                "{account.number_} - {account.billing_name_}"
                            }
                        }
                    }
                    p { class: "note", "Used by all the Octopus pages, and saved with this profile." }
                } else {
                    "{props.selected_account_id}"
                }
                if let Some(message) = error() {
                    p { ErrorText { error: message } }
                }
            }
        }
    }
}
