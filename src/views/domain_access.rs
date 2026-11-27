//! The Devices page's Domain section: founding the domain, adding its devices to it, and joining, by files,
//! see domain_security.rs and docs/CaptureDeviceReplication.md, section 1. What needs the user's authority, e.g. making
//! the domain or admitting someone, asks for the password of their key file, see user_key.rs, which is never kept.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, bail};
use dioxus::prelude::*;

use crate::capture_device::DeviceProfile;
use crate::domain::Settings as DomainSettings;
use crate::domain_security::{self, Invitation, JoinAs, JoinReply, JoinRequest, KeyFile};
use crate::user_key;

/// What the user asked to do which needs their key, see perform.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Action {
    Found,
    Rename,
    AddDevices,
    Admit,
    AskToJoin,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Action::Found => "Found the Domain",
            Action::Rename => "Rename the Domain",
            Action::AddDevices => "Add the Devices",
            Action::Admit => "Let Them Join",
            Action::AskToJoin => "Save the Request",
        }
    }

    /// Whether it makes a principal for the user, who gives their name, in their key file, which may be new.
    fn makes_principal(self) -> bool {
        matches!(self, Action::Found | Action::AskToJoin)
    }

    /// Whether it gives the domain's name, see domain_security::found_domain.
    fn names_domain(self) -> bool {
        matches!(self, Action::Found | Action::Rename)
    }
}

/// What perform needs from the form.
struct Form {
    password: String,
    confirm: String,
    name: String,
    title: String,
    role: JoinAs,
    request: Option<Arc<JoinRequest>>,
    invitation: Option<Arc<Invitation>>,
    devices: Vec<DeviceProfile>,
}

#[component]
pub fn DomainAccess(domain_name: String) -> Element {
    let settings = use_context::<Signal<DomainSettings>>();
    // Bumped when the domain or the key file changes, so that both are read again.
    let mut generation = use_signal(|| 0u32);
    let new_domain = domain_security::is_new_domain(&domain_name);
    let mut action = use_signal(|| new_domain.then_some(Action::Found));
    if new_domain {
        domain_security::asked_new_domain(&domain_name);
    }
    let mut password = use_signal(String::new);
    let mut confirm = use_signal(String::new);
    let mut name = use_signal(String::new);
    // The domain's name, see domain_security::found_domain, its name in the app until it is founded.
    let mut title = use_signal(|| domain_name.clone());
    let mut role = use_signal(|| JoinAs::Reader);
    let mut request = use_signal(|| None::<Arc<JoinRequest>>);
    let mut invitation = use_signal(|| None::<Arc<Invitation>>);
    let mut reply = use_signal(|| None::<Arc<JoinReply>>);
    let mut joined_name = use_signal(String::new);
    let mut status = use_signal(|| None::<Result<String, String>>);
    let mut busy = use_signal(|| false);

    let _ = generation();
    let site = crate::store::for_domain(&domain_name)
        .ok()
        .and_then(|store| domain_security::ensure(&store).ok().flatten());
    let titled = crate::store::for_domain(&domain_name)
        .ok()
        .and_then(|store| domain_security::title(&store).ok().flatten());
    let key = user_key::remembered();
    let held = key.as_ref().zip(site.as_ref()).and_then(|(path, site)| {
        user_key::principal_for(path, &site.domain)
            .ok()
            .flatten()
            .filter(|held| held.record.id == site.user.as_str())
    });

    let rename_title = titled.clone();
    let mut choose = move |chosen: Action| {
        action.set(Some(chosen));
        password.set(String::new());
        confirm.set(String::new());
        status.set(None);
    };

    let go = {
        let domain_name = domain_name.clone();
        move |_| {
            let Some(chosen) = action() else { return };
            let domain_name = domain_name.clone();
            let form = Form {
                password: password(),
                confirm: confirm(),
                name: name(),
                title: title(),
                role: role(),
                request: request(),
                invitation: invitation(),
                devices: settings.read().capture_devices.clone(),
            };
            spawn(async move {
                busy.set(true);
                status.set(None);
                match perform(&domain_name, chosen, form).await {
                    Ok(message) => {
                        status.set(Some(Ok(message)));
                        action.set(None);
                        request.set(None);
                        invitation.set(None);
                        generation += 1;
                    }
                    Err(error) => status.set(Some(Err(format!("{error:#}")))),
                }
                password.set(String::new());
                confirm.set(String::new());
                busy.set(false);
            });
        }
    };

    let open_key = move |_| {
        spawn(async move {
            let Some(file) = rfd::AsyncFileDialog::new()
                .set_title("Open your key file")
                .add_filter("Key file", &["bruce-key"])
                .pick_file()
                .await
            else {
                return;
            };
            let result = user_key::check(file.path()).and_then(|_| user_key::remember(file.path()));
            match result {
                Ok(()) => status.set(Some(Ok(format!(
                    "This app now uses the key file {}",
                    file.path().display()
                )))),
                Err(error) => status.set(Some(Err(format!("{error:#}")))),
            }
            generation += 1;
        });
    };

    let invite = {
        let domain_name = domain_name.clone();
        move |_| {
            let domain_name = domain_name.clone();
            spawn(async move {
                status.set(None);
                let result = async {
                    let made = domain_security::invitation(&*crate::store::for_domain(&domain_name)?)?;
                    let path = save_file("Save the invitation", &format!("Invitation to {}.json", made.name)).await?;
                    std::fs::write(&path, serde_json::to_vec_pretty(&made)?)?;
                    anyhow::Ok(format!(
                        "Saved the invitation to {}. Send it to whoever is to join, e.g. by email: nothing in it is \
                         secret. They send back a request, to Let Someone Join with",
                        path.display()
                    ))
                }
                .await;
                status.set(Some(result.map_err(|error| format!("{error:#}"))));
            });
        }
    };

    let open_invitation = move |_| {
        spawn(async move {
            status.set(None);
            let Some(file) = rfd::AsyncFileDialog::new()
                .set_title("Open the invitation")
                .add_filter("Invitation", &["json"])
                .pick_file()
                .await
            else {
                return;
            };
            match std::fs::read(file.path())
                .map_err(anyhow::Error::from)
                .and_then(|bytes| domain_security::read_invitation(&bytes))
            {
                Ok(read) => {
                    invitation.set(Some(Arc::new(read)));
                    choose(Action::AskToJoin);
                }
                Err(error) => status.set(Some(Err(format!("{error:#}")))),
            }
        });
    };

    let open_request = move |_| {
        spawn(async move {
            status.set(None);
            let Some(file) = rfd::AsyncFileDialog::new()
                .set_title("Open the request to join")
                .add_filter("Request to join", &["json"])
                .pick_file()
                .await
            else {
                return;
            };
            match std::fs::read(file.path())
                .map_err(anyhow::Error::from)
                .and_then(|bytes| domain_security::read_join_request(&bytes))
            {
                Ok(read) => {
                    request.set(Some(Arc::new(read)));
                    action.set(Some(Action::Admit));
                }
                Err(error) => status.set(Some(Err(format!("{error:#}")))),
            }
        });
    };

    let open_reply = move |_| {
        spawn(async move {
            status.set(None);
            let Some(file) = rfd::AsyncFileDialog::new()
                .set_title("Open the reply to your request to join")
                .add_filter("Reply", &["json"])
                .pick_file()
                .await
            else {
                return;
            };
            match std::fs::read(file.path())
                .map_err(anyhow::Error::from)
                .and_then(|bytes| domain_security::read_join_reply(&bytes))
            {
                Ok(read) => {
                    joined_name.set(read.by.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect());
                    reply.set(Some(Arc::new(read)));
                }
                Err(error) => status.set(Some(Err(format!("{error:#}")))),
            }
        });
    };

    let join = move |_| {
        let result = (|| {
            let read = reply().ok_or_else(|| anyhow!("Open the reply first"))?;
            let path = user_key::remembered()
                .ok_or_else(|| anyhow!("Open your key file first, the one you asked to join with"))?;
            domain_security::join(&read, &path, &joined_name())?;
            anyhow::Ok(format!(
                "Added {}, {}'s domain, to the app. Switch to it to see its devices",
                joined_name().trim(),
                read.by
            ))
        })();
        match result {
            Ok(message) => {
                status.set(Some(Ok(message)));
                reply.set(None);
            }
            Err(error) => status.set(Some(Err(format!("{error:#}")))),
        }
    };

    let makes_principal = action().is_some_and(Action::makes_principal);
    let needs_new_key = key.is_none() && makes_principal;

    rsx! {
        h2 { "Domain" }
        match &site {
            Some(site) => rsx! {
                p { class: "note",
                    {format!(
                        "This domain, {}, has members who say who may use its devices: co-owners, as equals, admins, who manage the devices, and readers. You are its member {}.",
                        titled.clone().unwrap_or_else(|| domain_name.clone()),
                        held.as_ref().map(|held| held.name.clone()).unwrap_or_else(|| site.user.to_string())
                    )}
                }
                div { class: "settings-buttons",
                    button { r#type: "button", disabled: busy(), onclick: move |_| choose(Action::AddDevices), "Add Devices to the Domain" }
                    button { r#type: "button", disabled: busy(), onclick: move |_| { if let Some(current) = rename_title.clone() { title.set(current); } choose(Action::Rename) }, "Rename the Domain…" }
                    button { r#type: "button", disabled: busy(), onclick: invite, "Invite Someone…" }
                    button { r#type: "button", disabled: busy(), onclick: open_request, "Let Someone Join…" }
                }
            },
            None => rsx! {
                p { class: "note",
                    "Until this domain is founded, only this app can use its devices. Found it to share them with others, as co-owners, admins or readers, or join someone else's with their invitation."
                }
                div { class: "settings-buttons",
                    button { r#type: "button", disabled: busy(), onclick: move |_| choose(Action::Found), "Found This Domain" }
                    button { r#type: "button", disabled: busy(), onclick: open_invitation, "Ask to Join a Domain…" }
                    button { r#type: "button", disabled: busy(), onclick: open_reply, "Join a Domain…" }
                }
            },
        }
        if let Some(read) = reply() {
            p { "The reply makes you a {read.role.label()} of {read.by}'s domain, with {read.devices.len()} devices. It is added to the app as a domain, named:" }
            div { class: "settings-buttons",
                input { r#type: "text", placeholder: "Domain name", value: "{joined_name}", oninput: move |event| joined_name.set(event.value()) }
                button { r#type: "button", disabled: busy() || joined_name().trim().is_empty(), onclick: join, "Join" }
                button { r#type: "button", onclick: move |_| reply.set(None), "Cancel" }
            }
        }
        if let Some(chosen) = action() {
            if chosen == Action::Found {
                p { class: "note",
                    "The domain is founded with you as its founder, so that you can share its devices. If it was made before domains, with data already, its data is moved to its new ID, after the app has read everything its devices have, and the devices are moved too, restarting each, so they must be on."
                }
            }
            if chosen == Action::AskToJoin {
                if let Some(read) = invitation() {
                    p { "{read.by} invites you to join {read.name}. The app makes you a principal of it, in your key file, and saves a request to send back to them." }
                }
            }
            if chosen == Action::Admit {
                if let Some(read) = request() {
                    p { "{read.user_name()} asks to join. Let them join as:" }
                    div { class: "settings-buttons",
                        for each in JoinAs::ALL {
                            label {
                                input {
                                    r#type: "radio",
                                    name: "join-as",
                                    checked: role() == each,
                                    onchange: move |_| role.set(each),
                                }
                                " {each.label()}"
                            }
                        }
                    }
                }
            }
            if chosen.names_domain() {
                div { class: "settings-buttons",
                    input { r#type: "text", placeholder: "Domain name, e.g. 7 Sadlers Close", title: "What the domain is called, which its members see, e.g. in an invitation", value: "{title}", oninput: move |event| title.set(event.value()) }
                }
            }
            if makes_principal {
                div { class: "settings-buttons",
                    input { r#type: "text", placeholder: "Your name", value: "{name}", oninput: move |event| name.set(event.value()) }
                }
            }
            if needs_new_key {
                p { class: "note",
                    "Your key file holds your keys, which say it is you, behind a password which is never kept. Keep it safe, e.g. on a USB stick; you are asked where to save it. If you have one already, open it instead."
                }
                div { class: "settings-buttons",
                    button { r#type: "button", disabled: busy(), onclick: open_key, "Open My Key File…" }
                }
            } else if let Some(path) = &key {
                p { class: "note", {format!("The password of your key file, {}:", path.display())} }
            }
            div { class: "settings-buttons",
                input { r#type: "password", placeholder: "Password", value: "{password}", oninput: move |event| password.set(event.value()) }
                if needs_new_key {
                    input { r#type: "password", placeholder: "Password again", value: "{confirm}", oninput: move |event| confirm.set(event.value()) }
                }
                button { r#type: "button", disabled: busy() || password().is_empty(), onclick: go, "{chosen.label()}" }
                button { r#type: "button", disabled: busy(), onclick: move |_| { action.set(None); request.set(None); invitation.set(None); }, "Cancel" }
            }
        }
        match status() {
            Some(Ok(message)) => rsx! { p { class: "ok", "{message}" } },
            Some(Err(message)) => rsx! { p { class: "invalid", "{message}" } },
            None => rsx! {},
        }
    }
}

/// The key file to use for an action which makes the user a principal: the one the app has, or a new one, saved where
/// the user chooses.
async fn key_file_for(name: &str) -> anyhow::Result<KeyFile> {
    if let Some(path) = user_key::remembered() {
        return Ok(KeyFile::Existing(path));
    }
    let path = save_file(
        "Save your key file",
        &format!("{}.bruce-key", if name.trim().is_empty() { "My" } else { name.trim() }),
    )
    .await
    .map_err(|_| anyhow!("No key file was made"))?;
    Ok(KeyFile::New(path))
}

/// The user's secret in the domain, from their key file, unlocked with password.
fn user_secret(domain_name: &str, password: &str) -> anyhow::Result<bruce_db::security::PrincipalSecret> {
    let store = crate::store::for_domain(domain_name)?;
    let site = domain_security::ensure(&store)?.ok_or_else(|| anyhow!("This domain has not been founded"))?;
    let path = user_key::remembered().ok_or_else(|| anyhow!("Open your key file first"))?;
    if user_key::principal_for(&path, &site.domain)?.is_none_or(|held| held.record.id != site.user.as_str()) {
        bail!("The key file has no key of this domain's member; open the one it was made with");
    }
    user_key::unlock(&path, password, &site.user)
}

/// Does what the user chose, with their key, from their key file, unlocked with the form's password, or made, with the
/// form's password, if the app has none yet, returning what to say.
async fn perform(domain_name: &str, action: Action, form: Form) -> anyhow::Result<String> {
    match action {
        Action::Found => {
            if form.name.trim().is_empty() {
                bail!("Give your name");
            }
            let key_file = key_file_for(&form.name).await?;
            domain_security::found_domain(
                domain_name,
                &key_file,
                &form.title,
                &form.name,
                &form.password,
                &form.confirm,
                &form.devices,
            )
            .await
        }
        Action::AskToJoin => {
            let invitation = form.invitation.ok_or_else(|| anyhow!("Open the invitation first"))?;
            if form.name.trim().is_empty() {
                bail!("Give your name");
            }
            let key_file = key_file_for(&form.name).await?;
            let request =
                domain_security::ask_to_join(&invitation, &key_file, &form.name, &form.password, &form.confirm)?;
            let path = save_file(
                "Save the request to join",
                &format!("Join request from {}.json", form.name.trim()),
            )
            .await?;
            std::fs::write(&path, serde_json::to_vec_pretty(&request)?)?;
            Ok(format!(
                "Saved the request to {}. Send it to {}, who sends back a reply to Join a Domain with",
                path.display(),
                invitation.by
            ))
        }
        Action::Rename => {
            let secret = user_secret(domain_name, &form.password)?;
            let store = crate::store::for_domain(domain_name)?;
            domain_security::rename(&store, &secret, &form.title)?;
            let (sent, errors) = domain_security::push(&store, &form.devices).await;
            Ok(format!(
                "Renamed the domain {}, which {sent} of its devices know already{}",
                form.title.trim(),
                domain_security::not_done(&errors)
            ))
        }
        Action::AddDevices => {
            let secret = user_secret(domain_name, &form.password)?;
            let store = crate::store::for_domain(domain_name)?;
            let (admitted, errors) = domain_security::admit_devices(&store, &secret, &form.devices).await?;
            Ok(format!(
                "Added {} devices to the domain{}",
                admitted.len(),
                domain_security::not_done(&errors)
            ))
        }
        Action::Admit => {
            let request = form.request.ok_or_else(|| anyhow!("Open the request to join first"))?;
            let secret = user_secret(domain_name, &form.password)?;
            let store = crate::store::for_domain(domain_name)?;
            let reply = domain_security::admit(&store, &request, form.role, &secret, &form.devices)?;
            let (sent, errors) = domain_security::push(&store, &form.devices).await;
            let name = request.user_name();
            let path = save_file(
                "Save the reply to the request to join",
                &format!("Reply to {name}.json"),
            )
            .await?;
            std::fs::write(&path, serde_json::to_vec_pretty(&reply)?)?;
            Ok(format!(
                "{name} is now a {} of the domain, which {sent} of its devices know already{}. Saved the reply to {}: \
                 send it to them",
                form.role.label(),
                domain_security::not_done(&errors),
                path.display()
            ))
        }
    }
}

/// Where the user chose to save a file, with the system save dialog.
async fn save_file(title: &str, name: &str) -> anyhow::Result<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title(title)
        .set_file_name(name)
        .set_can_create_directories(true)
        .save_file()
        .await
        .map(|file| file.path().to_path_buf())
        .ok_or_else(|| anyhow!("Nothing was saved"))
}
