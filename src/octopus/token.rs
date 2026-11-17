use crate::AppContext;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use super::graphql::ObtainJsonWebTokenInput;

use super::graphql::login::obtain_kraken_token::ObtainKrakenJsonWebToken;
use bruce_graphql::{GraphQLCaller, RequestManager, TokenManager};

/*
Implementation of TokenManager for the Octopus API
==================================================
*/

// // Yeah, I know. They declare a GenericScalar in fact its the JWT payload
// #[derive(GraphQLType)]
// #[graphql(params = "NoParams")]
// #[derive(Serialize, Deserialize, Debug, DisplayAsJsonPretty)]
// #[serde(rename_all = "camelCase")]
// struct JWT {
//     sub: String,
//     gty: String,
//     email: String,
//     token_use: String,
//     iss: String,
//     #[graphql(no_params)]
//     #[graphql(scalar)]
//     iat: u32,
//     #[graphql(no_params)]
//     #[graphql(scalar)]
//     exp: u32,
//     #[graphql(no_params)]
//     #[graphql(scalar)]
//     orig_iat: u32
//   }

const GRACE_PERIOD: u32 = 300;
// 60*60*24*10;

#[derive(Serialize, Deserialize)]
struct StoredToken {
    token_expires: u32,
    token: String,
    refresh_expires: u32,
    refresh: String,
}

impl From<&OctopusToken> for StoredToken {
    fn from(from: &OctopusToken) -> StoredToken {
        StoredToken {
            token_expires: from.token_expires,
            token: from.token.as_ref().clone(),
            refresh_expires: from.refresh_expires,
            refresh: from.refresh.as_ref().clone(),
        }
    }
}

struct OctopusToken {
    token_expires: u32,
    token: Arc<String>,
    refresh_expires: u32,
    refresh: Arc<String>,
}

impl From<StoredToken> for OctopusToken {
    fn from(from: StoredToken) -> OctopusToken {
        OctopusToken {
            token_expires: from.token_expires,
            token: Arc::new(from.token),
            refresh_expires: from.refresh_expires,
            refresh: Arc::new(from.refresh),
        }
    }
}

impl From<ObtainKrakenJsonWebToken> for OctopusToken {
    fn from(from: ObtainKrakenJsonWebToken) -> OctopusToken {
        let mut expires = 0;

        if let Some(object) = from.payload_.as_object() {
            if let Some(exp) = object.get("exp") {
                if let Some(exp) = exp.as_u64() {
                    expires = exp as u32;
                }
            }
        }
        OctopusToken {
            token_expires: expires,
            token: Arc::new(from.token_),
            refresh_expires: from.refresh_expires_in_ as u32,
            refresh: Arc::new(from.refresh_token_),
        }
    }
}

pub enum OctopusAuthenticator {
    ApiKey(String),
    EMailPassword { email: String, password: String },
}

impl OctopusAuthenticator {
    pub fn from_api_key(api_key: String) -> OctopusAuthenticator {
        OctopusAuthenticator::ApiKey(api_key)
    }

    pub fn from_email_password(email: String, password: String) -> OctopusAuthenticator {
        OctopusAuthenticator::EMailPassword { email, password }
    }

    pub fn to_obtain_json_web_token_input(&self) -> Result<ObtainJsonWebTokenInput, bruce_graphql::Error> {
        match self {
            OctopusAuthenticator::ApiKey(api_key) => {
                ObtainJsonWebTokenInput::builder().with_apikey(api_key.clone()).build()
            }
            OctopusAuthenticator::EMailPassword { email, password } => ObtainJsonWebTokenInput::builder()
                .with_email(email.clone())
                .with_password(password.clone())
                .build(),
        }
    }
}

pub struct OctopusTokenManager {
    context: Arc<AppContext>,
    request_manager: Arc<RequestManager>,
    authenticator: Mutex<Option<OctopusAuthenticator>>,
    token: Mutex<Option<OctopusToken>>,
    /// Whether the login is the saved one, so that a refused login is reported, see refuse, but not one being tried on
    /// the login page, see set_authenticator.
    saved_login: std::sync::atomic::AtomicBool,
}

/// The error codes with which Octopus refuses a login: a refresh token which has expired or is not right, an email
/// address and password which are not right, and an API key which is not right.
const REFUSED_CODES: [&str; 4] = ["KT-CT-1134", "KT-CT-1135", "KT-CT-1138", "KT-CT-1139"];

/// Whether Octopus refused the login, rather than e.g. not answering.
fn is_refusal(error: &bruce_graphql::Error) -> bool {
    match error {
        bruce_graphql::Error::GraphQLError(errors) => errors.iter().any(|error| {
            error
                .extensions
                .get("errorCode")
                .and_then(|code| code.as_str())
                .is_some_and(|code| REFUSED_CODES.contains(&code))
        }),
        _ => false,
    }
}

impl OctopusTokenManager {
    pub fn new(
        context: Arc<AppContext>,
        request_manager: Arc<RequestManager>,
        authenticator: Option<OctopusAuthenticator>,
    ) -> OctopusTokenManager {
        let token: Option<OctopusToken> =
            if let Some(json_web_token) = context.read_cache::<StoredToken>(crate::octopus::MODULE_ID) {
                Some(OctopusToken::from(json_web_token))
            } else {
                None
            };

        if context.args.app_args.verbose {
            if token.is_some() {
                println!("Loaded token from cache");
            } else {
                println!("No cached token found");
            }
        }

        OctopusTokenManager {
            context,
            request_manager,
            authenticator: Mutex::new(authenticator),
            token: Mutex::new(token),
            saved_login: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Says that the login is the saved one, see saved_login.
    pub fn with_saved_login(self) -> Self {
        self.saved_login.store(true, std::sync::atomic::Ordering::Relaxed);
        self
    }

    /// The login to try, from the login page, which is not the saved one, see saved_login.
    pub async fn set_authenticator(&self, authenticator: OctopusAuthenticator) {
        self.saved_login.store(false, std::sync::atomic::Ordering::Relaxed);
        *self.authenticator.lock().await = Some(authenticator);
    }

    /// Whether there is a saved login which can be used without asking: a token which can still be refreshed, or an API
    /// key. Checked without a request, so that opening the module does not need the network; a login which turns out
    /// to be refused is reported by the request which finds it, see refuse.
    pub async fn has_login(&self) -> bool {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u32;
        self.authenticator.lock().await.is_some()
            || self
                .token
                .lock()
                .await
                .as_ref()
                .is_some_and(|token| token.refresh_expires - GRACE_PERIOD > now)
    }

    /// Removes the token of the saved login which Octopus refused, which is no use, and reports it, see login.rs, if it
    /// is the saved one. Any API key is kept, so that the login page can show what it was.
    fn refuse(&self, reason: String) {
        if !self.saved_login.swap(false, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        if let Err(error) = self.context.remove_cache(crate::octopus::MODULE_ID) {
            crate::log_error!("Unable to remove the refused Octopus token, {error:#}");
        }
        let _ = crate::login::refused(&self.context, crate::octopus::MODULE_ID, reason);
    }
}

impl TokenManager for OctopusTokenManager {
    /// A token, see obtain, with a refused saved login removed and reported, see refuse.
    async fn get_authenticator(&self, refresh: bool) -> Result<Arc<String>, bruce_graphql::Error> {
        let result = self.obtain(refresh).await;
        match &result {
            Err(error) if is_refusal(error) => self.refuse(format!(
                "Octopus refused the saved login, {}",
                super::describe_login_error(error)
            )),
            Err(bruce_graphql::Error::MissingRequiredValueError(_)) => {
                self.refuse(String::from("The saved Octopus login has expired"))
            }
            _ => {}
        }
        result
    }
}

impl OctopusTokenManager {
    /// The current token, refreshed or obtained again if it has expired, or if refresh is set.
    async fn obtain(&self, refresh: bool) -> Result<Arc<String>, bruce_graphql::Error> {
        let mut locked_token = self.token.lock().await;

        let mut current_token = None;
        let mut refresh_token = None;

        if !refresh {
            if let Some(token) = &*locked_token {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u32;

                if token.token_expires - GRACE_PERIOD > now {
                    current_token = Some(token.token.clone());
                }
                if token.refresh_expires - GRACE_PERIOD > now {
                    refresh_token = Some(token.refresh.clone());
                }
            }
        }

        if let Some(token) = current_token {
            Ok(token)
        } else {
            if let Some(refresh_token) = refresh_token {
                if self.context.args.app_args.verbose {
                    println!("Refreshing Octopus token using refresh token...");
                }

                let input = ObtainJsonWebTokenInput::builder()
                    .with_refresh_token(refresh_token.as_ref().clone())
                    .build()?;

                let mutation = super::graphql::login::obtain_kraken_token::Mutation::new(input);
                let response: crate::octopus::graphql::login::obtain_kraken_token::Response =
                    self.request_manager.call2(&mutation, None).await?;

                let token = OctopusToken::from(response.obtain_kraken_token_);

                if self.context.args.app_args.verbose {
                    println!("Obtained new Octopus token via refresh");
                }

                if let Err(error) = self
                    .context
                    .update_cache(crate::octopus::MODULE_ID, &StoredToken::from(&token))
                {
                    return Err(bruce_graphql::Error::InternalError(format!(
                        "Failed to update cache {}",
                        error
                    )));
                }

                let result = token.token.clone();

                *locked_token = Some(token);

                Ok(result)
            } else {
                let locked_authenticator = self.authenticator.lock().await;
                if let Some(authenticator) = &*locked_authenticator {
                    let input = authenticator.to_obtain_json_web_token_input()?;

                    if self.context.args.app_args.verbose {
                        println!("Obtaining new Octopus token...");
                    }

                    let mutation = super::graphql::login::obtain_kraken_token::Mutation::new(input);
                    let response: crate::octopus::graphql::login::obtain_kraken_token::Response =
                        self.request_manager.call2(&mutation, None).await?;

                    let token = OctopusToken::from(response.obtain_kraken_token_);

                    if let Err(error) = self
                        .context
                        .update_cache(crate::octopus::MODULE_ID, &StoredToken::from(&token))
                    {
                        return Err(bruce_graphql::Error::InternalError(format!(
                            "Failed to update cache {}",
                            error
                        )));
                    }

                    let result = token.token.clone();

                    *locked_token = Some(token);

                    Ok(result)
                } else {
                    Err(bruce_graphql::Error::MissingRequiredValueError(
                        "No authenticator provided",
                    ))
                }
            }
        }
    }
}

// These tests actually hit the API and cause "Too many requests." errors
// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn test_invalid_api_key() {
//         if let Err(octopus_error) = test_api_key("foo".to_string()) {

//             println!("result {:?}", octopus_error);
//             if let Error::GraphQLError(gql_error) = octopus_error {

//                 if let bruce_graphql::error::Error::GraphQLError(json_errors) = &gql_error {
//                     let x = json_errors.get(0).unwrap().extensions.error_code.as_ref().unwrap();
//                     assert_eq!(x, "KT-CT-1139");
//                 }
//                 else {
//                     panic!("Expected GraphQLError KT-CT-1139 got <{:?}>", &gql_error);
//                 }
//             }
//             else {
//                 panic!("Expected GraphQLError KT-CT-1139 got <{:?}>", octopus_error);
//             }
//             // match error {
//             //     Error::GraphQLError(_) => todo!(),
//             //     Error::IOError(_) => todo!(),
//             //     Error::JsonError(_) => todo!(),
//             //     Error::InternalError(_) => todo!(),
//             //     Error::CallerError(_) => todo!(),
//             //     Error::StringError(_) => todo!(),
//             // };
//             // assert_eq!(error., 4);
//         }
//         else {
//             panic!("Expected error, got token");
//         }

//     }

//     fn test_api_key(api_key: String)  -> anyhow::Result<Arc<std::string::String>> {
//         let mut octopus_client = crate::octopus::ClientBuilder::new_test()
//             .with_api_key(api_key)?
//             .do_build(false)?;

//         tokio_test::block_on(octopus_client.token_manager.authenticate())
//     }

//     #[test]
//     fn test_invalid_refresh() {
//         if let Err(octopus_error) = test_refresh_token("invalid_refresh_token".to_string()) {

//             println!("result {:?}", octopus_error);
//             if let Error::GraphQLError(gql_error) = octopus_error {

//                 if let bruce_graphql::error::Error::GraphQLError(json_errors) = &gql_error {
//                     let x = json_errors.get(0).unwrap().extensions.error_code.as_ref().unwrap();
//                     assert_eq!(x, "KT-CT-1135");
//                 }
//                 else {
//                     panic!("Expected GraphQLError KT-CT-1139 got <{:?}>", &gql_error);
//                 }
//             }
//             else {
//                 panic!("Expected GraphQLError KT-CT-1139 got <{:?}>", octopus_error);
//             }
//             // match error {
//             //     Error::GraphQLError(_) => todo!(),
//             //     Error::IOError(_) => todo!(),
//             //     Error::JsonError(_) => todo!(),
//             //     Error::InternalError(_) => todo!(),
//             //     Error::CallerError(_) => todo!(),
//             //     Error::StringError(_) => todo!(),
//             // };
//             // assert_eq!(error., 4);
//         }
//         else {
//             panic!("Expected error, got token");
//         }

//     }

//     #[test]
//     fn test_expired_refresh() {

//         /*

//     Need to fix this once this test refresh token has expired in a week.
//         {
//   "data": {
//     "obtainKrakenToken": {
//       "token": "<JWT>",
//       "possibleErrors": [
//         {
//           "code": "KT-CT-1135",
//           "type": "VALIDATION",
//           "message": "Invalid data.",
//           "description": "Please make sure the refresh token is correct."
//         },
//         {
//           "code": "KT-CT-1134",
//           "type": "VALIDATION",
//           "message": "Invalid data.",
//           "description": "The refresh token has expired."
//         }
//       ]
//     }
//   }
// }
//          */
//         if let Err(octopus_error) = test_refresh_token("1645431dbd92cd2f804fdbba89eaa52e07efe3cabfce41e590ef70a42c96f5ca".to_string()) {

//             println!("result {:?}", octopus_error);
//             if let Error::GraphQLError(gql_error) = octopus_error {

//                 if let bruce_graphql::error::Error::GraphQLError(json_errors) = &gql_error {
//                     let x = json_errors.get(0).unwrap().extensions.error_code.as_ref().unwrap();
//                      assert_eq!(x, "KT-CT-1135");
//                 }
//                 else {
//                     panic!("Expected GraphQLError KT-CT-1139 got <{:?}>", &gql_error);
//                 }
//             }
//             else {
//                 panic!("Expected GraphQLError KT-CT-1139 got <{:?}>", octopus_error);
//             }
//             // match error {
//             //     Error::GraphQLError(_) => todo!(),
//             //     Error::IOError(_) => todo!(),
//             //     Error::JsonError(_) => todo!(),
//             //     Error::InternalError(_) => todo!(),
//             //     Error::CallerError(_) => todo!(),
//             //     Error::StringError(_) => todo!(),
//             // };
//             // assert_eq!(error., 4);
//         }
//         else {
//             panic!("Expected error, got token");
//         }

//     }

//     fn test_refresh_token(refresh_token: String)  -> anyhow::Result<Arc<std::string::String>> {
//         let mut octopus_client = crate::octopus::ClientBuilder::new_test()
//             .with_api_key("dummy_api_key_to_prevent_password_prompt".to_string())?
//             .do_build(false)?;

//     octopus_client.token_manager.token = Some(Token {
//         token_expires: 0,
//         token:          Arc::new("dummy_token".to_string()),
//         refresh_token:  ObtainJSONWebTokenInput {
//             api_key: None,
//             email: None,
//             organization_secret_key: None,
//             password: None,
//             pre_signed_key: None,
//             refresh_token: Some(refresh_token),
//         }
//     });

//         tokio_test::block_on(octopus_client.token_manager.authenticate())
//     }
// }
