use anyhow::{Context, Result, bail};
use hostable_core::{HostAction, Protocol};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

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
                let mut args = vec!["install".to_string(), "-y".to_string()];
                args.extend(packages.iter().cloned());
                run_owned("apt-get", args)?;
                Ok(format!("installed apt packages: {}", packages.join(", ")))
            }
            HostAction::EnableAptComponent { components } => Ok(format!(
                "apt components requested: {}",
                components.join(", ")
            )),
            HostAction::AddArchitecture { arch } => {
                run("dpkg", &["--add-architecture", arch])?;
                Ok(format!("added architecture: {arch}"))
            }
            HostAction::CreateUser { user, system } => {
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

fn chown(path: &str, owner: &str) -> Result<()> {
    run("chown", &[owner, path])
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
