use std::io::Write;
use std::process::{Command, Stdio};
use std::{env, error::Error, fs::File, path::Path, process};

// fn get_current_working_dir() -> String {
//     let res = std::env::current_dir();
//     match res {
//         Ok(path) => path.into_os_string().into_string().unwrap(),
//         Err(_) => "FAILED".to_string()
//     }
// }
fn main() {
    if let Err(_) = build() {
        process::exit(1);
        // panic!("Build failed {}", error);
    }
}

fn build() -> Result<(), Box<dyn Error>> {
    if let Some(dir) = env::var_os("OUT_DIR") {
        let format = time::format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]").unwrap();
        let now = time::OffsetDateTime::now_utc();

        let dest_path = Path::new(&dir).join("crate_info.rs");
        // let dest_path_string = format!("{}", dest_path.to_string_lossy());

        let mut file = File::create(dest_path)?;

        writeln!(
            file,
            r#"
mod create_info {{
    #![allow(dead_code)]
    pub const PACKAGE_NAME: &'static str = "{}";
    pub const PACKAGE_VERSION: &'static str = "{}";
    pub const USER_AGENT: &'static str = "{}-{}";
    pub const BUILD_TIMESTAMP: &'static str = "{}";
"#,
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            now.format(&format).unwrap()
        )?;

        git_status(&mut file)?;
        writeln!(file, "}}")?;
    }

    println!("cargo::rerun-if-changed=build.rs");
    bruce_graphql_builder::builder("graphql")
        .with_type("Date", "bruce_graphql::types::Date")
        .with_type("DateTime", "bruce_graphql::types::DateTime")
        .with_type("Decimal", "crate::octopus::decimal::Decimal")
        .with_schema("graphql/octopus/octopus-schema.graphql")
        .with_query("graphql/octopus/Login.graphql", "login")
        .with_query("graphql/octopus/account.graphql", "account")
        .with_query("graphql/octopus/meter.graphql", "meter")
        .with_query("graphql/octopus/bill.graphql", "bill")
        .with_query("graphql/octopus/flex.graphql", "flex")
        // .with_query("force_error", "force_error")
        // .with_print(true)
        .build()?;

    // panic!("Panic test!");
    Ok(())
}

/*
git --no-optional-locks status --porcelain=v2 --branch --show-stash --ignore-submodules -uno
*/

fn git_status(file: &mut File) -> Result<(), std::io::Error> {
    let mut is_git = false;
    let mut tag_name = String::new();
    let mut branch_name = String::new();
    let mut is_dirty = false;
    let mut is_staged = false;
    let mut has_stash = false;
    let mut upstream: Option<i32> = None;

    let output = Command::new("git")
        .arg("name-rev")
        .arg("--name-only")
        .arg("--tags")
        .arg("HEAD")
        .stderr(Stdio::null())
        .output()
        .expect("Failed to execute command");

    let output_str = String::from_utf8_lossy(&output.stdout);

    for line in output_str.lines() {
        tag_name = line.trim().to_string();
    }

    let output = Command::new("git")
        .arg("--no-optional-locks")
        .arg("status")
        .arg("--porcelain=v2")
        .arg("--branch")
        .arg("--show-stash")
        .arg("--ignore-submodules")
        .arg("-uno")
        .stderr(Stdio::null())
        .output()
        .expect("Failed to execute command");

    let output_str = String::from_utf8_lossy(&output.stdout);

    for line in output_str.lines() {
        is_git = true;

        let line = line.trim();
        if line.starts_with('#') {
            if line.starts_with("# branch.head") {
                branch_name = line[14..].to_string();
            } else if line.starts_with("# stash") {
                has_stash = true;
            } else if line.starts_with("# branch.ab") {
                let remote_differences = line[12..].replace(['+', '-'], "");
                if remote_differences == "0 0" {
                    upstream = Some(0);
                } else if remote_differences.starts_with("0 ") {
                    upstream = Some(-1);
                } else if remote_differences.ends_with(" 0") {
                    upstream = Some(1);
                } else {
                    upstream = Some(2);
                }
            }
        } else if &line[2..3] != "." {
            is_staged = true;
            if &line[3..4] != "." {
                is_dirty = true;
            }
        } else {
            is_dirty = true;
        }
        if is_staged && is_dirty {
            // Early exit, no need to check more entries since both dirty and
            // staged are in effect.
            break;
        }
    }

    writeln!(file, "\tpub const GIT_REPOSITORY : bool = {};", is_git)?;
    if is_git {
        // pub const USER_AGENT: &'static str = "{}-{};";
        writeln!(file, "\tpub const GIT_BRANCH : &'static str = \"{}\";", branch_name)?;
        writeln!(file, "\tpub const GIT_TAG : &'static str = \"{}\";", tag_name)?;
    }
    writeln!(file, "\tpub const GIT_DIRTY : bool = {};", is_dirty)?;
    writeln!(file, "\tpub const GIT_STAGED : bool = {};", is_staged)?;
    if upstream.is_some() {
        writeln!(
            file,
            "\tpub const GIT_UPSTREAM : &'static str = \"{}\";",
            upstream.unwrap()
        )?;
    }
    writeln!(file, "\tpub const GIT_STASH : bool = {};", has_stash)
}
