use anyhow::{Context, Result, bail};
use hostable_core::{HostAction, Protocol};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy)]
pub struct ActionExecutor {
    dry_run: bool,
}

impl ActionExecutor {
    pub fn new(dry_run: bool) -> Self {
        Self { dry_run }
    }

    pub fn execute(&self, actions: &[HostAction]) -> Result<Vec<String>> {
        let mut log = Vec::new();
        for action in actions {
            log.push(self.execute_one(action)?);
        }
        Ok(log)
    }

    fn execute_one(&self, action: &HostAction) -> Result<String> {
        if self.dry_run {
            return Ok(format!("dry-run: {action:?}"));
        }

        match action {
            HostAction::AptInstall { packages } => {
                run("apt-get", &["update"])?;
                preseed_steamcmd_license(packages)?;
                let mut args = vec!["install".to_string(), "-y".to_string()];
                args.extend(packages.iter().cloned());
                run_owned("apt-get", args)?;
                Ok(format!("installed apt packages: {}", packages.join(", ")))
            }
            HostAction::EnableAptComponent { components } => {
                let changed = enable_apt_components(components)?;
                if changed {
                    run("apt-get", &["update"])?;
                    Ok(format!("enabled apt components: {}", components.join(", ")))
                } else {
                    Ok(format!(
                        "apt components already enabled: {}",
                        components.join(", ")
                    ))
                }
            }
            HostAction::AddArchitecture { arch } => {
                run("dpkg", &["--add-architecture", arch])?;
                Ok(format!("added architecture: {arch}"))
            }
            HostAction::CreateUser { user, system } => {
                if Command::new("id")
                    .arg(user)
                    .output()
                    .map(|output| output.status.success())
                    .unwrap_or(false)
                {
                    return Ok(format!("user already exists: {user}"));
                }
                let mut args = Vec::new();
                if *system {
                    args.push("--system");
                }
                args.extend(["--create-home", "--shell", "/usr/sbin/nologin", user]);
                run("useradd", &args)?;
                Ok(format!("created user: {user}"))
            }
            HostAction::CreateDirectory { path, owner } => {
                fs::create_dir_all(path)
                    .with_context(|| format!("failed to create {}", path.display()))?;
                chown(path.to_string_lossy().as_ref(), owner)?;
                Ok(format!("created directory: {}", path.display()))
            }
            HostAction::Download { url, dest, .. } => {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                run_owned(
                    "curl",
                    vec![
                        "-fL".to_string(),
                        url.clone(),
                        "-o".to_string(),
                        dest.to_string_lossy().into_owned(),
                    ],
                )?;
                Ok(format!("downloaded {url} to {}", dest.display()))
            }
            HostAction::RunCommand {
                program,
                args,
                user,
                cwd,
            } => {
                let mut command = if let Some(user) = user {
                    let mut command = Command::new("runuser");
                    command.arg("-u").arg(user).arg("--").arg(program);
                    command
                } else {
                    Command::new(program)
                };
                command.args(args);
                if let Some(cwd) = cwd {
                    command.current_dir(cwd);
                }
                run_command(&mut command)?;
                Ok(format!("ran command: {program} {}", args.join(" ")))
            }
            HostAction::WriteFile {
                path,
                contents,
                owner,
                mode,
            } => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, contents)
                    .with_context(|| format!("failed to write {}", path.display()))?;
                fs::set_permissions(path, fs::Permissions::from_mode(*mode))?;
                chown(path.to_string_lossy().as_ref(), owner)?;
                Ok(format!("wrote file: {}", path.display()))
            }
            HostAction::RenderSystemdUnit { name, contents } => {
                let path = format!("/etc/systemd/system/{name}");
                fs::write(&path, contents).with_context(|| format!("failed to write {path}"))?;
                Ok(format!("rendered systemd unit: {name}"))
            }
            HostAction::SystemdDaemonReload => {
                run("systemctl", &["daemon-reload"])?;
                Ok("reloaded systemd".to_string())
            }
            HostAction::SystemdEnable { unit } => {
                run("systemctl", &["enable", unit])?;
                Ok(format!("enabled systemd unit: {unit}"))
            }
            HostAction::SystemdStart { unit } => {
                run("systemctl", &["start", unit])?;
                Ok(format!("started systemd unit: {unit}"))
            }
            HostAction::UfwAllow { port, protocol } => {
                let rule = match protocol {
                    Protocol::Tcp => format!("{port}/tcp"),
                    Protocol::Udp => format!("{port}/udp"),
                };
                run("ufw", &["allow", &rule])?;
                Ok(format!("allowed firewall rule: {rule}"))
            }
        }
    }
}

pub fn systemctl(unit: &str, verb: &str) -> Result<()> {
    run("systemctl", &[verb, unit])
}

pub fn systemctl_is_active(unit: &str) -> Result<String> {
    let output = Command::new("systemctl")
        .args(["is-active", unit])
        .output()
        .context("failed to run systemctl is-active")?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn journalctl_logs(unit: &str, lines: u16) -> Result<String> {
    let line_count = lines.clamp(1, 500).to_string();
    let output = Command::new("journalctl")
        .args([
            "-u",
            unit,
            "-n",
            &line_count,
            "--no-pager",
            "--output",
            "short-iso",
        ])
        .output()
        .context("failed to run journalctl")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("journalctl failed: {}", stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn chown(path: &str, owner: &str) -> Result<()> {
    run("chown", &[owner, path])
}

fn preseed_steamcmd_license(packages: &[String]) -> Result<()> {
    if !packages.iter().any(|package| package == "steamcmd") {
        return Ok(());
    }

    let selections = b"steamcmd steam/question select I AGREE\nsteamcmd steam/license note \n";
    let mut command = Command::new("debconf-set-selections")
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to spawn debconf-set-selections")?;
    command
        .stdin
        .as_mut()
        .context("failed to open debconf-set-selections stdin")?
        .write_all(selections)?;
    let status = command.wait()?;
    if !status.success() {
        bail!("debconf-set-selections failed for steamcmd license");
    }
    Ok(())
}

fn enable_apt_components(components: &[String]) -> Result<bool> {
    if components.is_empty() {
        return Ok(false);
    }

    let mut changed = false;
    for path in apt_source_files()? {
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        let updated = if path.extension().and_then(|ext| ext.to_str()) == Some("sources") {
            update_deb822_sources(&contents, components)
        } else {
            update_sources_list(&contents, components)
        };

        if updated != contents {
            let backup = path.with_extension(format!(
                "{}hostable.bak",
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .map(|extension| format!("{extension}."))
                    .unwrap_or_default()
            ));
            if !backup.exists() {
                fs::write(&backup, &contents)
                    .with_context(|| format!("failed to write {}", backup.display()))?;
            }
            fs::write(&path, updated)
                .with_context(|| format!("failed to update {}", path.display()))?;
            changed = true;
        }
    }
    Ok(changed)
}

fn apt_source_files() -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let sources_list = PathBuf::from("/etc/apt/sources.list");
    if sources_list.exists() {
        files.push(sources_list);
    }
    let source_dirs = [
        Path::new("/etc/apt/sources.list.d"),
        Path::new("/etc/apt/sources.sources.d"),
    ];
    for dir in source_dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let extension = path.extension().and_then(|extension| extension.to_str());
            if matches!(extension, Some("list" | "sources")) {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn update_sources_list(contents: &str, components: &[String]) -> String {
    contents
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if !trimmed.starts_with("deb ") {
                return line.to_string();
            }

            let mut parts = line
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>();
            let component_start = if parts.get(1).map(|value| value.starts_with('[')) == Some(true)
            {
                4
            } else {
                3
            };
            if parts.len() < component_start {
                return line.to_string();
            }
            for component in components {
                if !parts[component_start..].contains(component) {
                    parts.push(component.clone());
                }
            }
            parts.join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if contents.ends_with('\n') { "\n" } else { "" }
}

fn update_deb822_sources(contents: &str, components: &[String]) -> String {
    contents
        .lines()
        .map(|line| {
            let Some(existing) = line.strip_prefix("Components:") else {
                return line.to_string();
            };
            let mut values = existing
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>();
            for component in components {
                if !values.contains(component) {
                    values.push(component.clone());
                }
            }
            format!("Components: {}", values.join(" "))
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if contents.ends_with('\n') { "\n" } else { "" }
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let mut command = Command::new(program);
    command.args(args);
    run_command(&mut command)
}

fn run_owned(program: &str, args: Vec<String>) -> Result<()> {
    let mut command = Command::new(program);
    command.args(args);
    run_command(&mut command)
}

fn run_command(command: &mut Command) -> Result<()> {
    let output = command
        .output()
        .with_context(|| format!("failed to spawn {:?}", command))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("command {:?} failed: {}", command, stderr.trim());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_components_to_classic_sources_list() {
        let input = "deb http://deb.debian.org/debian bookworm main\n";
        let output = update_sources_list(
            input,
            &[
                "contrib".to_string(),
                "non-free".to_string(),
                "non-free-firmware".to_string(),
            ],
        );
        assert_eq!(
            output,
            "deb http://deb.debian.org/debian bookworm main contrib non-free non-free-firmware\n"
        );
    }

    #[test]
    fn preserves_existing_components_in_classic_sources_list() {
        let input =
            "deb [signed-by=/key.gpg] http://archive.ubuntu.com/ubuntu noble main universe\n";
        let output =
            update_sources_list(input, &["universe".to_string(), "multiverse".to_string()]);
        assert_eq!(
            output,
            "deb [signed-by=/key.gpg] http://archive.ubuntu.com/ubuntu noble main universe multiverse\n"
        );
    }

    #[test]
    fn adds_components_to_deb822_sources() {
        let input =
            "Types: deb\nURIs: http://deb.debian.org/debian\nSuites: bookworm\nComponents: main\n";
        let output = update_deb822_sources(input, &["contrib".to_string(), "non-free".to_string()]);
        assert_eq!(
            output,
            "Types: deb\nURIs: http://deb.debian.org/debian\nSuites: bookworm\nComponents: main contrib non-free\n"
        );
    }
}
