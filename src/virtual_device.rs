//! Virtual capture devices: the capture firmware run on this computer in the board simulator, for people without an
//! ESP32, see capture/README.md. The app starts its launcher, which asks which board to simulate, and the device's
//! window shows its address, which the user copies into the Devices page to adopt it, as a real one.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::anyhow;

/// The launcher's file name, see capture/src/sim_launcher.rs.
fn launcher_name() -> String {
    format!("bruce-smart-home-capture-sim{}", std::env::consts::EXE_SUFFIX)
}

/// Where the launcher is: beside the app's own program, where the installers put it, or, when the app is run from its
/// source, where capture/build-sim.sh builds it.
pub fn launcher() -> Option<PathBuf> {
    let installed = std::env::current_exe()
        .ok()
        .and_then(|program| program.parent().map(|folder| folder.join(launcher_name())));
    let built = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("capture/target-sim/bin")
        .join(launcher_name());
    installed.into_iter().chain([built]).find(|path| path.is_file())
}

/// Whether the device at address is a virtual one: on this computer, with a port, which a real device's address never
/// has, see capture/README.md.
pub fn is_virtual(address: &str) -> bool {
    let Some((host, port)) = address.trim().rsplit_once(':') else {
        return false;
    };
    if port.parse::<u16>().is_err() {
        return false;
    }
    host == "localhost" || host == "127.0.0.1" || this_computer().is_some_and(|ip| ip.to_string() == host)
}

/// This computer's address on its network, as a virtual device finds its own, by asking the system which it would send
/// from to an address on the internet, which sends nothing.
fn this_computer() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// Starts the virtual capture device with the ID id again, e.g. after its window was closed, with the board it was,
/// which the launcher finds from its data, see --device.
pub fn start_device(id: &str) -> anyhow::Result<()> {
    spawn(&["--device", id])
}

/// Starts a virtual capture device: the launcher, which shows the boards to choose from. It runs on its own, after the
/// app has quit too, as a real device would.
pub fn start() -> anyhow::Result<()> {
    spawn(&[])
}

/// Starts the launcher with args, on its own.
fn spawn(args: &[&str]) -> anyhow::Result<()> {
    let launcher = launcher().ok_or_else(|| {
        anyhow!("The virtual capture device is not installed with this app, see the help to build it")
    })?;
    let mut command = Command::new(&launcher);
    command.args(args);
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // Without a console window on Windows, as the device has its own window.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .spawn()
        .map_err(|error| anyhow!("Unable to start {}, {error}", launcher.display()))?;
    crate::log_info!("Started a virtual capture device, {}", launcher.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_virtual_device_is_on_this_computer_with_a_port() {
        assert!(is_virtual("localhost:60496"));
        assert!(is_virtual(" 127.0.0.1:60496 "));
        if let Some(ip) = this_computer() {
            assert!(is_virtual(&format!("{ip}:60496")));
            assert!(!is_virtual(&ip.to_string()));
        }
        // A real device's address, which has no port, or another computer's.
        assert!(!is_virtual("192.0.2.83"));
        assert!(!is_virtual("192.0.2.83:60496"));
        assert!(!is_virtual("localhost:http"));
    }
}
