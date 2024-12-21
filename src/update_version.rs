use anyhow::{anyhow, Context, Error};
use libloading::{Library, Symbol};
use semver::Version;
use std::process::{self, Command};

pub fn update_available() -> Result<bool, Error> {
    let release = minreq::get("https://api.github.com/repos/hankthetank27/edl-gen/releases/latest")
        .with_header("User-Agent", "EDLgen")
        .send()?
        .json::<serde_json::Value>()?;
    let latest_version = release["tag_name"]
        .as_str()
        .context("'tag_name' property does not exist for latest release")?;
    bump_is_greater(env!("CARGO_PKG_VERSION"), latest_version)
}

pub fn update() -> Result<(), Error> {
    #[cfg(all(target_os = "macos", not(debug_assertions)))]
    mac_conveyor_sparkle_check_update()?;
    #[cfg(all(target_os = "windows", not(debug_assertions)))]
    windows_update_and_quit();
    Ok(())
}

fn bump_is_greater(current: &str, latest: &str) -> Result<bool, Error> {
    Ok(Version::parse(latest)? > Version::parse(current)?)
}

pub fn mac_conveyor_sparkle_check_update() -> Result<(), Error> {
    unsafe {
        let lib = Library::new("../Frameworks/libconveyor.dylib")
            .map_err(|e| anyhow!("Failed to load Conveyor library: {}", e))?;
        let update: Symbol<unsafe extern "C" fn() -> i32> = lib
            .get(b"conveyor_check_for_updates")
            .map_err(|e| anyhow!("Failed to find updater symbol: {}", e))?;
        update();
    }
    Ok(())
}

pub fn windows_update_and_quit() {
    if Command::new("updatecheck.exe")
        .args(["--update-check"])
        .spawn()
        .is_ok()
    {
        process::exit(0);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_bump_greater() {
        assert!(bump_is_greater("1.2.0", "1.2.3").unwrap());
        assert!(bump_is_greater("0.2.0", "1.2.3").unwrap());
        assert!(bump_is_greater("0.2.0", "0.2.3").unwrap());
    }
}
