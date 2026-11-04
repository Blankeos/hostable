//! `hostable upgrade`: reinstall the release in place with the installer.
//!
//! The installer is the one place that knows how a release is laid out (both
//! binaries, the web admin, the systemd unit), so an upgrade runs it again for the
//! target version instead of duplicating that here. Game servers run as their own
//! systemd units, so restarting the daemon does not interrupt a game.

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const REPO: &str = "Blankeos/hostable";
const INSTALLER_URL: &str = "https://raw.githubusercontent.com/Blankeos/hostable/main/install.sh";

macro_rules! say {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
}

#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// Already on the requested (or latest) version.
    Current(String),
    Install(String),
}

pub fn run(version: Option<&str>, check: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let client = Client::builder().timeout(Duration::from_secs(15)).build()?;

    let requested = version.filter(|v| !v.eq_ignore_ascii_case("latest"));
    let latest = match requested {
        Some(_) => None,
        None => Some(latest_version(&client)?),
    };
    let plan = plan(current, requested, latest.as_deref())?;

    if check {
        match plan {
            Plan::Current(version) => say!("hostable {version} is up to date"),
            Plan::Install(target) => {
                say!("hostable {current} -> {target} available. Run: hostable upgrade")
            }
        }
        return Ok(());
    }

    let target = match plan {
        Plan::Current(version) => {
            say!("hostable {version} is up to date");
            return Ok(());
        }
        Plan::Install(target) => target,
    };

    let exe = install_path()?;
    let install_dir = exe
        .parent()
        .context("cannot tell where hostable is installed")?;
    say!("hostable {current} -> {target}");

    // Overridable for forks and for testing against a local server.
    let installer_url =
        env::var("HOSTABLE_INSTALLER_URL").unwrap_or_else(|_| INSTALLER_URL.to_string());
    let script = client
        .get(installer_url)
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.text())
        .context("could not download the installer")?;
    if !script.starts_with("#!") {
        bail!("the installer download looks wrong; try again in a minute");
    }

    let file = write_private(&script)?;
    let status = Command::new("sh")
        .arg(&file)
        .env("HOSTABLE_VERSION", format!("v{target}"))
        .env("HOSTABLE_INSTALL_DIR", install_dir)
        // The packages are already there. Skipping apt keeps an upgrade to seconds.
        .env("HOSTABLE_SKIP_DEPS", "1")
        .status()
        .context("could not run the installer")?;
    let _ = fs::remove_file(&file);
    if !status.success() {
        bail!(
            "the installer failed (exit {})",
            status.code().unwrap_or(-1)
        );
    }

    let installed = Command::new(&exe)
        .arg("--version")
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default();
    say!(
        "upgraded to {}. Game servers kept running.",
        if installed.is_empty() {
            target
        } else {
            installed
        }
    );
    Ok(())
}

fn latest_version(client: &Client) -> Result<String> {
    let release: Release = client
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .header(
            "User-Agent",
            concat!("hostable/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.json())
        .context("could not look up the latest release on GitHub")?;
    Ok(normalize(&release.tag_name))
}

fn normalize(version: &str) -> String {
    version.trim().trim_start_matches('v').to_string()
}

/// `1.2.3` to numbers; anything else (a pre-release tag, a typo) is rejected.
fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let parsed = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(parsed)
}

fn plan(current: &str, requested: Option<&str>, latest: Option<&str>) -> Result<Plan> {
    let current = normalize(current);
    if let Some(requested) = requested {
        let target = normalize(requested);
        if parse(&target).is_none() {
            bail!("'{requested}' is not a version like 0.0.2");
        }
        return Ok(if target == current {
            Plan::Current(current)
        } else {
            Plan::Install(target)
        });
    }
    let latest = normalize(latest.context("no release found")?);
    match (parse(&current), parse(&latest)) {
        (Some(have), Some(newest)) if newest > have => Ok(Plan::Install(latest)),
        (Some(_), Some(_)) => Ok(Plan::Current(current)),
        _ => bail!("cannot compare {current} with {latest}"),
    }
}

/// Where the running binary lives, refusing a cargo build: the installer would put
/// a release next to it and the next `cargo build` would quietly replace it again.
fn install_path() -> Result<PathBuf> {
    if !cfg!(target_os = "linux") {
        bail!("upgrade works on the VPS (Linux). Run it there.");
    }
    let exe = env::current_exe().context("cannot resolve the hostable binary")?;
    let exe = exe.canonicalize().unwrap_or(exe);
    if is_build_output(&exe) {
        bail!(
            "{} is a cargo build, not an installed release. Install with the install script instead.",
            exe.display()
        );
    }
    Ok(exe)
}

fn is_build_output(path: &Path) -> bool {
    path.components().any(|part| part.as_os_str() == "target")
        && path
            .components()
            .any(|part| part.as_os_str() == "debug" || part.as_os_str() == "release")
}

fn write_private(script: &str) -> Result<PathBuf> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = env::temp_dir().join(format!("hostable-install-{}.sh", std::process::id()));
    let _ = fs::remove_file(&path);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .context("could not write the installer to a temp file")?;
    file.write_all(script.as_bytes())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_release_is_installed() {
        assert_eq!(
            plan("0.0.1", None, Some("v0.0.2")).unwrap(),
            Plan::Install("0.0.2".into())
        );
    }

    #[test]
    fn same_or_older_latest_is_current() {
        assert_eq!(
            plan("0.0.2", None, Some("v0.0.2")).unwrap(),
            Plan::Current("0.0.2".into())
        );
        assert_eq!(
            plan("0.1.0", None, Some("v0.0.9")).unwrap(),
            Plan::Current("0.1.0".into())
        );
    }

    #[test]
    fn compares_numbers_not_text() {
        assert_eq!(
            plan("0.0.9", None, Some("0.0.10")).unwrap(),
            Plan::Install("0.0.10".into())
        );
    }

    #[test]
    fn an_explicit_version_may_go_back() {
        assert_eq!(
            plan("0.0.3", Some("v0.0.1"), None).unwrap(),
            Plan::Install("0.0.1".into())
        );
        assert_eq!(
            plan("0.0.1", Some("0.0.1"), None).unwrap(),
            Plan::Current("0.0.1".into())
        );
    }

    #[test]
    fn rejects_things_that_are_not_versions() {
        assert!(plan("0.0.1", Some("main"), None).is_err());
        assert!(plan("0.0.1", Some("1.2"), None).is_err());
        assert!(plan("0.0.1", Some("1.2.3.4"), None).is_err());
    }

    #[test]
    fn cargo_builds_are_not_installs() {
        assert!(is_build_output(Path::new(
            "/home/me/hostable/target/debug/hostable"
        )));
        assert!(is_build_output(Path::new("/x/target/release/hostable")));
        assert!(!is_build_output(Path::new("/usr/local/bin/hostable")));
    }
}
