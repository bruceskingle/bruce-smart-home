//! The virtual capture device's launcher: `bruce-smart-home-capture-sim --board <board> [options]`, or without
//! --board, after the user chooses one in a window, starts the capture
//! firmware built for that board for the simulator, `bruce-smart-home-capture-<board>`, from the folder this is in,
//! giving it the rest of the options, e.g. --data or --screenshot, see bruce-embedded-std's docs/Simulator.md. The
//! firmware is built once per board, as for the ESP32s, see build-sim.sh, and this makes them one command.

use std::process::{Command, ExitCode};

/// The boards there is a build of, by the name --board takes, and their makers' names.
const BOARDS: &[(&str, &str)] = &[
    ("rlcd42", "Waveshare ESP32-S3-RLCD-4.2, black and white, with the clock"),
    ("guitron", "Guition JC3248W535C, colour, with the graph"),
];

fn usage() -> String {
    let boards: Vec<String> = BOARDS
        .iter()
        .map(|(name, title)| format!("  {name:10} {title}"))
        .collect();
    format!(
        "Usage: bruce-smart-home-capture-sim [--board <board> | --device <id>] [--data <folder>] [--screenshot <file> --after <seconds> \
         [--headless]]\nThe boards:\n{}",
        boards.join("\n")
    )
}

/// The board of the virtual device with the ID id, kept in its board's usual folder, see bruce-esp-idf's sim/mod.rs,
/// where its NVS's device namespace holds its ID, see its identity.rs, or None if no board's has it.
fn board_of(id: &str) -> Option<&'static str> {
    let simulator = dirs::data_dir()?.join("Bruce's Embedded Simulator");
    // Its folder, or, until it first starts and moves it, where the simulator kept it before 16 November 2026.
    let folders = [
        simulator.join("bruce-smart-home-capture"),
        simulator.join("bruce-esp-idf"),
    ];
    BOARDS.iter().map(|(name, _)| *name).find(|name| {
        folders.iter().any(|folder| {
            std::fs::read(folder.join(name).join("nvs/device.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .is_some_and(|device| device["sim_id"]["str"] == id)
        })
    })
}

/// The board the user chooses in a window, see bruce_embedded_sim::picker, by the name --board takes, or None if they
/// close it.
fn choose() -> anyhow::Result<Option<String>> {
    let boards: Vec<(&'static bruce_embedded_sim::Board, &str)> = BOARDS
        .iter()
        .filter_map(|(name, about)| bruce_embedded_sim::Board::named(name).map(|board| (board, *about)))
        .collect();
    Ok(bruce_embedded_sim::picker::choose(&boards)?.map(|board| board.id.to_string()))
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // --device <id> starts the virtual device with that ID, whichever board it is, e.g. from the app's Restart.
    if let Some(position) = args.iter().position(|arg| arg == "--device") {
        let Some(id) = args.get(position + 1).cloned() else {
            eprintln!("{}", usage());
            return ExitCode::from(2);
        };
        args.drain(position..=position + 1);
        match board_of(&id) {
            Some(board) => args.extend(["--board".to_string(), board.to_string()]),
            None => {
                eprintln!("There is no virtual device {id} in the usual folders");
                return ExitCode::FAILURE;
            }
        }
    }
    let board = match args.iter().position(|arg| arg == "--board") {
        Some(position) => {
            let Some(board) = args.get(position + 1).cloned() else {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            };
            args.drain(position..=position + 1);
            board
        }
        // Without one, e.g. opened from the desktop, the user chooses in a window.
        None => match choose() {
            Ok(Some(board)) => board,
            Ok(None) => return ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Unable to show the boards to choose from, {error:#}\n{}", usage());
                return ExitCode::FAILURE;
            }
        },
    };
    let Some((name, _)) = BOARDS.iter().find(|(name, _)| name.eq_ignore_ascii_case(&board)) else {
        eprintln!("There is no board {board}\n{}", usage());
        return ExitCode::from(2);
    };
    let folder = match std::env::current_exe() {
        Ok(program) => program.parent().map(|folder| folder.to_path_buf()).unwrap_or_default(),
        Err(error) => {
            eprintln!("Unable to find where this program is, {error}");
            return ExitCode::FAILURE;
        }
    };
    let firmware = folder.join(format!(
        "bruce-smart-home-capture-{name}{}",
        std::env::consts::EXE_SUFFIX
    ));
    match Command::new(&firmware).args(&args).status() {
        Ok(status) => status
            .code()
            .map_or(ExitCode::FAILURE, |code| ExitCode::from(code as u8)),
        Err(error) => {
            eprintln!("Unable to start {}, {error}", firmware.display());
            ExitCode::FAILURE
        }
    }
}
