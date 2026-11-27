//! The guide for AI agents to the app's data, docs/DataForAgents.md, written into the data directory as AGENTS.md when
//! the app starts, with a section about the installation: each domain, its store's directory, its modules and its
//! topology. An agent asked to explain the data can then find it without reading the settings, which hold the
//! household's passwords and API keys.

use std::fmt::Write as _;

/// The guide, as the repository has it.
const GUIDE: &str = include_str!("../docs/DataForAgents.md");

/// The guide's name in the data directory, the name agents look for.
const FILE_NAME: &str = "AGENTS.md";

/// Writes the guide, with the installation's section, see installation, into the data directory, if it differs from
/// what is there, so that it follows the domains as they change, and says why if it can not, e.g. if the domains can
/// not be read, as the app works without it.
pub fn write() {
    if let Err(error) = try_write() {
        crate::log_warning!("Unable to write the guide for AI agents to the data directory, {error:#}");
    }
}

fn try_write() -> anyhow::Result<()> {
    // Without their secrets, which the guide does not give, see installation.
    let domains = crate::domain::read_domain_file_without_secrets()?.unwrap_or_default();
    let contents = format!("{GUIDE}\n{}", installation(&domains));
    let path = crate::data_home::data_dir()?.join(FILE_NAME);
    if std::fs::read_to_string(&path).ok().as_deref() == Some(contents.as_str()) {
        return Ok(());
    }
    std::fs::create_dir_all(crate::data_home::data_dir()?)?;
    std::fs::write(&path, contents)?;
    Ok(())
}

/// The installation's section: the app's version, and each domain's store directory, whether it is an offline copy,
/// its modules, and its topology, as JSON, see power::topology, which is all an analysis needs from the settings. No
/// module's settings are given, as they hold the logins.
fn installation(domains: &[crate::domain::Domain]) -> String {
    let mut section = format!(
        "## This installation\n\nWritten by Bruce's Smart Home {} when it started.\n",
        env!("CARGO_PKG_VERSION")
    );
    if domains.is_empty() {
        section.push_str("\nThere are no domains yet.\n");
    }
    for domain in domains {
        let _ = write!(section, "\n### Domain {}\n\n", domain.name);
        let store = domain
            .id
            .as_deref()
            .and_then(|site| bruce_db::Id::parse(site).ok())
            .map(|site| format!("`store/{}/`", site.hex()))
            .unwrap_or_else(|| String::from("none yet"));
        let _ = writeln!(section, "- Store: {store}");
        if domain.offline {
            section.push_str("- An offline, redacted copy of another domain's data, which fetches nothing.\n");
        }
        let mut modules: Vec<&String> = domain.modules.keys().collect();
        modules.sort();
        let modules: Vec<&str> = modules.iter().map(|module| module.as_str()).collect();
        let _ = writeln!(section, "- Modules: {}", modules.join(", "));
        match &domain.settings.topology {
            Some(topology) => {
                let json = serde_json::to_string_pretty(topology).unwrap_or_default();
                let _ = writeln!(
                    section,
                    "- Topology, from the grid connection, each circuit with what it is for, its channels, the \
                     modules' measurements of it, and the circuits it feeds:\n\n```json\n{json}\n```"
                );
            }
            None => section.push_str("- No topology yet, so the import is not split by use.\n"),
        }
    }
    section
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_domain_is_given_with_its_store_modules_and_topology_but_not_its_settings() {
        let domains: Vec<crate::domain::Domain> = serde_json::from_value(serde_json::json!([
            {
                "name": "Home",
                "modules": {"octopus": {"apiKey": "secret-key"}, "myenergi": {}},
                "settings": {"topology": {"name": "Grid", "use": "grid", "channels": ["octopus:electricity"]}}
            },
            {"name": "Copy", "modules": {}, "offline": true}
        ]))
        .unwrap();
        let section = installation(&domains);
        assert!(section.contains("### Domain Home"), "{section}");
        assert!(section.contains("- Modules: myenergi, octopus"), "{section}");
        assert!(section.contains("\"octopus:electricity\""), "{section}");
        assert!(section.contains("offline, redacted copy"), "{section}");
        assert!(section.contains("No topology yet"), "{section}");
        assert!(!section.contains("secret-key"), "{section}");
    }
}
