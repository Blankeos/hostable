//! Non-interactive commands: everything the TUI can do that is useful in scripts,
//! over SSH one-liners, or cron. They talk to the same local daemon API.

use anyhow::{Context, Result, bail};
use clap::{Subcommand, ValueEnum};
use hostable_core::GameKind;
use reqwest::blocking::{Client, RequestBuilder};
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path, time::Duration};

/// `println!` panics on a closed pipe (`hostable list | head`); this just stops quietly.
macro_rules! say {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum GameArg {
    Minecraft,
    Ark,
    Palworld,
}

impl From<GameArg> for GameKind {
    fn from(game: GameArg) -> Self {
        match game {
            GameArg::Minecraft => GameKind::Minecraft,
            GameArg::Ark => GameKind::ArkSurvivalEvolved,
            GameArg::Palworld => GameKind::Palworld,
        }
    }
}

impl GameArg {
    fn api_name(self) -> &'static str {
        match self {
            GameArg::Minecraft => "minecraft",
            GameArg::Ark => "ark_survival_evolved",
            GameArg::Palworld => "palworld",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List servers and whether they are running.
    List,
    /// Start a server.
    Start { server: String },
    /// Stop a server (refuses while players are online unless --force).
    Stop {
        server: String,
        #[arg(long)]
        force: bool,
    },
    /// Restart a server (refuses while players are online unless --force).
    Restart {
        server: String,
        #[arg(long)]
        force: bool,
    },
    /// Take a backup now.
    Backup { server: String },
    /// Back up, then update the game files in place (Steam games).
    Update {
        server: String,
        #[arg(long)]
        force: bool,
    },
    /// Send a console (RCON) command.
    Console {
        server: String,
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,
    },
    /// Show recent logs.
    Logs {
        server: String,
        #[arg(long, default_value_t = 80)]
        lines: u16,
    },
    /// Check a server for known problems.
    Doctor { server: String },
    /// Show what takes disk space; optionally clean something safe.
    Disk {
        /// `journal` or `apt`.
        #[arg(long)]
        clean: Option<String>,
    },
    /// Create a swapfile (protects small VPSes from out-of-memory kills).
    Swap {
        /// Size in MB, e.g. 4096.
        size_mb: u32,
    },
    /// Check whether this machine can run a game.
    Preflight {
        #[arg(value_enum)]
        game: GameArg,
        #[arg(long)]
        memory_mb: Option<u32>,
    },
    /// Install a new server.
    Create {
        #[arg(value_enum)]
        game: GameArg,
        name: String,
        /// vanilla, paper or fabric (Minecraft only).
        #[arg(long, default_value = "vanilla")]
        server_type: String,
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        memory_mb: Option<u32>,
        #[arg(long)]
        join_password: Option<String>,
        /// Accept the game's license/EULA (required).
        #[arg(long)]
        accept_license: bool,
        /// Show the plan without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Adopt a server that is already installed on this machine.
    Import {
        #[arg(value_enum)]
        game: GameArg,
        name: String,
        path: String,
        #[arg(long)]
        dry_run: bool,
    },
}

pub struct Api {
    base: String,
    token: String,
    client: Client,
}

impl Api {
    pub fn new(base: &str, token_path: &Path) -> Result<Self> {
        let token = fs::read_to_string(token_path)
            .map_err(|error| {
                anyhow::anyhow!(
                    "cannot read the operator token at {} ({error}). Run with sudo.",
                    token_path.display()
                )
            })?
            .trim()
            .to_string();
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            token,
            // Installs and updates download gigabytes.
            client: Client::builder()
                .timeout(Duration::from_secs(60 * 60))
                .build()?,
        })
    }

    fn auth(&self, request: RequestBuilder) -> RequestBuilder {
        request.header("x-hostable-operator-token", &self.token)
    }

    fn send(&self, request: RequestBuilder) -> Result<Value> {
        let response = self
            .auth(request)
            .send()
            .context("could not reach hostabled. Is it running? (systemctl status hostabled)")?;
        let status = response.status();
        let body: Value = response.json().unwrap_or(Value::Null);
        if !status.is_success() {
            let message = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("request failed");
            bail!("{message} (HTTP {})", status.as_u16());
        }
        Ok(body)
    }

    fn get(&self, path: &str) -> Result<Value> {
        self.send(self.client.get(format!("{}{path}", self.base)))
    }

    fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.send(self.client.post(format!("{}{path}", self.base)).json(&body))
    }
}

fn server_id(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn force_query(force: bool) -> &'static str {
    if force { "?force=true" } else { "" }
}

fn print_warnings(value: &Value) {
    if let Some(warnings) = value.get("warnings").and_then(Value::as_array) {
        for warning in warnings.iter().filter_map(Value::as_str) {
            eprintln!("warning: {warning}");
        }
    }
}

pub fn run(command: Command, api: &Api) -> Result<()> {
    match command {
        Command::List => {
            let servers = api.get("/api/servers")?;
            let list = servers.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                say!(
                    "No servers yet. Try: hostable create palworld \"My Server\" --accept-license"
                );
            }
            for server in list {
                let id = server["id"].as_str().unwrap_or("?");
                let state = api
                    .get(&format!("/api/servers/{id}/status"))
                    .ok()
                    .and_then(|status| status["active"].as_str().map(str::to_string))
                    .unwrap_or_else(|| "unknown".to_string());
                let ports = server["ports"]
                    .as_array()
                    .map(|ports| {
                        ports
                            .iter()
                            .map(|port| {
                                format!(
                                    "{}/{}",
                                    port["port"],
                                    port["protocol"].as_str().unwrap_or("")
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                say!(
                    "{id:<24} {game:<22} {state:<10} {ports}",
                    game = server["game"].as_str().unwrap_or("")
                );
            }
        }
        Command::Start { server } => {
            let reply = api.post(
                &format!("/api/servers/{}/start", server_id(&server)),
                json!({}),
            )?;
            print_warnings(&reply);
            say!("started {server}");
        }
        Command::Stop { server, force } => {
            let reply = api.post(
                &format!(
                    "/api/servers/{}/stop{}",
                    server_id(&server),
                    force_query(force)
                ),
                json!({}),
            )?;
            print_warnings(&reply);
            say!("stopped {server}");
        }
        Command::Restart { server, force } => {
            let reply = api.post(
                &format!(
                    "/api/servers/{}/restart{}",
                    server_id(&server),
                    force_query(force)
                ),
                json!({}),
            )?;
            print_warnings(&reply);
            say!("restarted {server}");
        }
        Command::Backup { server } => {
            let reply = api.post(
                &format!("/api/servers/{}/backups/run", server_id(&server)),
                json!({}),
            )?;
            print_warnings(&reply);
            say!("backup: {}", reply["archive_path"].as_str().unwrap_or("?"));
        }
        Command::Update { server, force } => {
            let reply = api.post(
                &format!(
                    "/api/servers/{}/update{}",
                    server_id(&server),
                    force_query(force)
                ),
                json!({}),
            )?;
            print_warnings(&reply);
            say!(
                "updated {server}: build {} -> {}",
                reply["build_before"].as_str().unwrap_or("unknown"),
                reply["build_after"].as_str().unwrap_or("unknown")
            );
            if let Some(backup) = reply["backup"].as_str() {
                say!("backup taken first: {backup}");
            }
        }
        Command::Console { server, command } => {
            let reply = api.post(
                &format!("/api/servers/{}/console", server_id(&server)),
                json!({ "command": command.join(" ") }),
            )?;
            say!("{}", reply["message"].as_str().unwrap_or(""));
        }
        Command::Logs { server, lines } => {
            let reply = api.get(&format!(
                "/api/servers/{}/logs?lines={lines}",
                server_id(&server)
            ))?;
            let _ = write!(
                std::io::stdout(),
                "{}",
                reply["text"].as_str().unwrap_or("")
            );
        }
        Command::Doctor { server } => {
            let reply = api.get(&format!("/api/servers/{}/doctor", server_id(&server)))?;
            let findings = reply.as_array().cloned().unwrap_or_default();
            if findings.is_empty() {
                say!("no problems found");
            }
            for finding in findings.iter().filter_map(Value::as_str) {
                say!("- {finding}");
            }
        }
        Command::Disk { clean } => {
            if let Some(action) = clean {
                let reply = api.post("/api/system/disk/clean", json!({ "action": action }))?;
                say!("{}", reply["message"].as_str().unwrap_or("done"));
            }
            let report = api.get("/api/system/disk")?;
            say!(
                "disk: {}% used, {} MB free of {} MB",
                report["used_percent"],
                report["free_mb"],
                report["total_mb"]
            );
            for item in report["items"].as_array().cloned().unwrap_or_default() {
                let mb = item["bytes"].as_u64().unwrap_or(0) / (1024 * 1024);
                let clean_hint = item["clean_action"]
                    .as_str()
                    .map(|action| format!("  [hostable disk --clean {action}]"))
                    .unwrap_or_default();
                say!(
                    "{mb:>8} MB  {}{clean_hint}\n             {}",
                    item["label"].as_str().unwrap_or(""),
                    item["note"].as_str().unwrap_or("")
                );
            }
        }
        Command::Swap { size_mb } => {
            let reply = api.post("/api/system/swap", json!({ "size_mb": size_mb }))?;
            say!("{}", reply["message"].as_str().unwrap_or("done"));
        }
        Command::Preflight { game, memory_mb } => {
            let mut path = format!("/api/preflight?game={}", game.api_name());
            if let Some(memory_mb) = memory_mb {
                path.push_str(&format!("&memory_mb={memory_mb}"));
            }
            let report = api.get(&path)?;
            say!(
                "memory: {} MB RAM ({} MB available), {} MB swap | disk free: {} MB",
                report["total_memory_mb"],
                report["available_memory_mb"],
                report["swap_total_mb"],
                report["disk_free_mb"]
            );
            for blocker in report["blockers"].as_array().cloned().unwrap_or_default() {
                say!("BLOCKER: {}", blocker.as_str().unwrap_or(""));
            }
            for warning in report["warnings"].as_array().cloned().unwrap_or_default() {
                say!("warning: {}", warning.as_str().unwrap_or(""));
            }
        }
        Command::Create {
            game,
            name,
            server_type,
            version,
            memory_mb,
            join_password,
            accept_license,
            dry_run,
        } => {
            if !accept_license && !dry_run {
                bail!(
                    "pass --accept-license to confirm you accept the game's license (Minecraft EULA, or the Steam terms for ARK and Palworld)"
                );
            }
            let server_type = match game {
                GameArg::Minecraft => server_type,
                GameArg::Ark => "ark_survival_evolved".to_string(),
                GameArg::Palworld => "palworld".to_string(),
            };
            let mut extra = serde_json::Map::new();
            if let Some(password) = join_password {
                extra.insert("server_password".to_string(), password.into());
            }
            let reply = api.post(
                "/api/servers/create",
                json!({
                    "game": game.api_name(),
                    "dry_run": dry_run,
                    "input": {
                        "name": name,
                        "install_path": format!("/srv/hostable/servers/{}", server_id(&name)),
                        "server_type": server_type,
                        "game_version": version,
                        "memory_mb": memory_mb,
                        "accepted_license": true,
                        "extra": extra,
                    }
                }),
            )?;
            print_warnings(&reply);
            if dry_run {
                say!("plan (nothing changed):");
                for action in reply["plan"]["actions"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                {
                    let label = action
                        .as_object()
                        .and_then(|object| object.keys().next().cloned())
                        .or_else(|| action.as_str().map(str::to_string))
                        .unwrap_or_default();
                    say!("  - {label}");
                }
            } else {
                say!(
                    "created {} -> start it with: hostable start {}",
                    reply["server"]["name"].as_str().unwrap_or(&name),
                    reply["server"]["id"].as_str().unwrap_or("")
                );
            }
        }
        Command::Import {
            game,
            name,
            path,
            dry_run,
        } => {
            let reply = api.post(
                "/api/servers/import",
                json!({
                    "game": game.api_name(),
                    "name": name,
                    "install_path": path,
                    "dry_run": dry_run,
                }),
            )?;
            print_warnings(&reply);
            say!(
                "{} {}",
                if dry_run { "would import" } else { "imported" },
                reply["server"]["name"].as_str().unwrap_or(&name)
            );
        }
    }
    Ok(())
}
