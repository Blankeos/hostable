use crate::theme::{Level, theme};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use reqwest::{
    blocking::{Client, RequestBuilder},
    header::HeaderValue,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

mod create;
mod link;
mod manage;
mod ui;
use manage::ManageTab;
use ui::{action_spans, block};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Server,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    ServerDetail,
    CreateServer,
    ConnectServer,
    EditS3,
    EditAdminWeb,
    EditAdminPassword,
    Manage,
    PickModrinth,
    PickChoice,
    EditConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreateField {
    Game,
    Type,
    Name,
    Path,
    Version,
    Memory,
    Motd,
    Content,
    License,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModrinthPickKind {
    Modpack,
    Mod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum S3Field {
    LocalKeepLast,
    S3KeepLast,
    Bucket,
    Region,
    Endpoint,
    Prefix,
    AccessKeyEnv,
    SecretKeyEnv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdminPasswordField {
    NewPassword,
    ConfirmPassword,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectField {
    Game,
    Name,
    Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChoiceTarget {
    CreateGame,
    CreateType,
    ConnectGame,
}

#[derive(Debug, Clone)]
enum ChoiceValue {
    Game(hostable_core::GameKind),
    ServerType(hostable_core::ServerType),
}

#[derive(Debug, Clone)]
struct ChoiceOption {
    label: &'static str,
    value: ChoiceValue,
}

#[derive(Debug, Clone)]
struct CreateDraft {
    game: hostable_core::GameKind,
    server_type: hostable_core::ServerType,
    field: CreateField,
    cursor: usize,
    name: String,
    path: String,
    version: String,
    memory_mb: String,
    motd: String,
    modrinth: ModrinthDraft,
    /// Highlighted row of the Modrinth list (the last row is "+ add").
    content_row: usize,
    /// The folder stops following the name once it has been edited by hand.
    path_edited: bool,
    accepted_license: bool,
    dry_run_reviewed: bool,
}

#[derive(Debug, Clone, Default)]
struct ModrinthDraft {
    modpack: Option<SelectedModrinthProject>,
    mods: Vec<SelectedModrinthProject>,
}

#[derive(Debug, Clone)]
struct SelectedModrinthProject {
    title: String,
    selection: hostable_core::ModrinthInstallSelection,
}

#[derive(Debug, Clone)]
struct ModrinthPicker {
    kind: ModrinthPickKind,
    query: String,
    cursor: usize,
    results: Vec<hostable_core::ModrinthSearchHit>,
    selected: usize,
    dirty: bool,
}

#[derive(Debug, Clone)]
struct ConnectDraft {
    field: ConnectField,
    cursor: usize,
    game: hostable_core::GameKind,
    name: String,
    path: String,
    dry_run_reviewed: bool,
}

#[derive(Debug, Clone)]
struct S3Draft {
    field: S3Field,
    cursor: usize,
    local_keep_last: String,
    s3_keep_last: String,
    bucket: String,
    region: String,
    endpoint_url: String,
    prefix: String,
    access_key_env: String,
    secret_key_env: String,
}

#[derive(Debug, Clone, Default)]
struct AdminWebDraft {
    enabled: bool,
    domain: String,
    cursor: usize,
    public_ip: Option<String>,
}

#[derive(Debug, Clone)]
struct AdminPasswordDraft {
    field: AdminPasswordField,
    cursor: usize,
    new_password: String,
    confirm_password: String,
}

#[derive(Debug, Clone)]
struct ChoicePicker {
    target: ChoiceTarget,
    query: String,
    cursor: usize,
    selected: usize,
}

#[derive(Debug, Clone, Default)]
struct ConfigEditor {
    server_id: String,
    document: Option<hostable_core::ConfigDocument>,
    selected_file: usize,
    buffer: String,
}

pub struct App {
    active_screen: Screen,
    mode: Mode,
    create: CreateDraft,
    connect: ConnectDraft,
    s3: S3Draft,
    admin_web: AdminWebDraft,
    admin_password: AdminPasswordDraft,
    choice_picker: ChoicePicker,
    modrinth_picker: ModrinthPicker,
    config_editor: ConfigEditor,
    manage: manage::Manage,
    selected_server: usize,
    daemon_url: String,
    operator_token: Option<String>,
    client: Client,
    servers: Vec<hostable_core::ServerRecord>,
    server_statuses: HashMap<String, String>,
    status: String,
    /// Result of the last attempt to reach the daemon. `None` until the first one.
    connected: Option<bool>,
    started: Instant,
    last_status_poll: Instant,
    last_list_poll: Instant,
    toast_seen: String,
    toast_at: Instant,
    toast_level: Option<Level>,
    help: bool,
    confirm: Option<Confirm>,
}

/// Asked when the daemon refuses a stop or restart because players are online.
#[derive(Debug, Clone)]
struct Confirm {
    action: String,
    ok_message: String,
    server_id: String,
    server_name: String,
    players: u32,
    /// Focus starts on Cancel so an accidental Enter is safe.
    focus_confirm: bool,
}

enum ActionOutcome {
    Done(Vec<String>),
    NeedsConfirm(u32),
    Failed(String),
}

impl App {
    pub fn new(daemon_url: String, operator_token_path: PathBuf) -> Self {
        let operator_token = fs::read_to_string(&operator_token_path)
            .ok()
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty());
        let status = if operator_token.is_some() {
            "Not connected".to_string()
        } else {
            format!(
                "Not connected | cannot read {} (run `sudo hostable`)",
                operator_token_path.display()
            )
        };
        Self {
            active_screen: Screen::Server,
            mode: Mode::Normal,
            create: CreateDraft::default(),
            connect: ConnectDraft::default(),
            s3: S3Draft::default(),
            admin_web: AdminWebDraft::default(),
            admin_password: AdminPasswordDraft::default(),
            choice_picker: ChoicePicker::default(),
            modrinth_picker: ModrinthPicker::default(),
            config_editor: ConfigEditor::default(),
            manage: manage::Manage::default(),
            selected_server: 0,
            daemon_url,
            operator_token,
            client: Client::builder()
                .connect_timeout(Duration::from_secs(3))
                // Installs and backups are slow; short calls set their own timeout.
                .timeout(Duration::from_secs(3600))
                .build()
                .unwrap_or_else(|_| Client::new()),
            servers: Vec::new(),
            server_statuses: HashMap::new(),
            status,
            connected: None,
            started: Instant::now(),
            last_status_poll: Instant::now(),
            last_list_poll: Instant::now(),
            toast_seen: String::new(),
            toast_at: Instant::now(),
            toast_level: None,
            help: false,
            confirm: None,
        }
    }

    /// Entry point for terminal key events. Returns true when the app should quit.
    pub fn handle_event(&mut self, key: KeyEvent) -> bool {
        // Windows reports releases and repeats too; act on presses only.
        if key.kind != KeyEventKind::Press {
            return false;
        }
        // Raw mode turns Ctrl-C into a plain key event, so quit explicitly.
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if key.code == KeyCode::Char('o') && self.mode == Mode::Manage {
                self.open_mod_link();
            }
            return key.code == KeyCode::Char('c');
        }
        self.handle_key(key.code)
    }

    pub fn handle_key(&mut self, code: KeyCode) -> bool {
        if self.confirm.is_some() {
            self.handle_confirm_key(code);
            return false;
        }
        if self.help {
            self.help = false;
            return false;
        }
        if self.mode == Mode::ServerDetail {
            self.handle_server_detail_key(code);
            return false;
        }
        if self.mode == Mode::CreateServer {
            self.handle_create_key(code);
            return false;
        }
        if self.mode == Mode::ConnectServer {
            self.handle_connect_key(code);
            return false;
        }
        if self.mode == Mode::EditS3 {
            self.handle_s3_key(code);
            return false;
        }
        if self.mode == Mode::EditAdminWeb {
            self.handle_admin_web_key(code);
            return false;
        }
        if self.mode == Mode::EditAdminPassword {
            self.handle_admin_password_key(code);
            return false;
        }
        if self.mode == Mode::Manage {
            self.handle_manage_key(code);
            return false;
        }
        if self.mode == Mode::PickModrinth {
            self.handle_modrinth_key(code);
            return false;
        }
        if self.mode == Mode::PickChoice {
            self.handle_choice_key(code);
            return false;
        }
        if self.mode == Mode::EditConfig {
            self.handle_config_key(code);
            return false;
        }

        match code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Esc => {
                if self.active_screen == Screen::Settings {
                    self.active_screen = Screen::Server;
                }
            }
            KeyCode::Char('1') => {
                self.active_screen = Screen::Server;
            }
            KeyCode::Char('2') => {
                self.active_screen = Screen::Settings;
                self.refresh_settings();
                self.refresh_public_ip();
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => self.next_tab(),
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => self.previous_tab(),
            KeyCode::Char('r') => {
                if self.active_screen == Screen::Server {
                    self.refresh_servers();
                } else {
                    self.refresh_settings();
                    self.refresh_public_ip();
                }
            }
            _ => match self.active_screen {
                Screen::Server => self.handle_server_key(code),
                Screen::Settings => self.handle_settings_key(code),
            },
        }
        false
    }

    fn handle_server_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Down | KeyCode::Char('j') => self.next_server(),
            KeyCode::Up | KeyCode::Char('k') => self.previous_server(),
            KeyCode::Enter => self.open_server_detail(),
            KeyCode::Char('n') => self.open_create_server(),
            KeyCode::Char('c') => self.open_connect_server(),
            _ => {}
        }
    }

    fn handle_server_detail_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Char('r') => self.refresh_servers(),
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('y') => self.copy_join_address(),
            KeyCode::Char('s') => self.start_selected(),
            KeyCode::Char('x') => self.stop_selected(),
            KeyCode::Char('R') => self.restart_selected(),
            KeyCode::Char('b') => self.backup_selected(),
            KeyCode::Char('c') | KeyCode::Char('L') => self.open_manage(ManageTab::Console),
            KeyCode::Char('p') => self.open_manage(ManageTab::Players),
            KeyCode::Char('m') => self.open_manage(ManageTab::Mods),
            KeyCode::Char('f') => self.open_config_editor(),
            _ => {}
        }
    }

    fn handle_settings_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('e') => self.open_s3_settings(),
            KeyCode::Char('t') => self.test_s3_settings(),
            KeyCode::Char('w') => self.toggle_admin_web(),
            KeyCode::Char('d') => self.open_admin_web_domain(),
            KeyCode::Char('p') => self.open_admin_password(),
            _ => {}
        }
    }

    fn with_operator_token(&self, request: RequestBuilder) -> RequestBuilder {
        let Some(token) = &self.operator_token else {
            return request;
        };
        let Ok(header) = HeaderValue::from_str(token) else {
            return request;
        };
        request.header("x-hostable-operator-token", header)
    }

    pub fn refresh_servers(&mut self) {
        self.last_list_poll = Instant::now();
        let url = format!("{}/api/servers", self.daemon_url.trim_end_matches('/'));
        let request = self
            .with_operator_token(self.client.get(url))
            .timeout(Duration::from_secs(5));
        let result = match request.send() {
            Ok(response) if response.status().is_success() => response
                .json::<Vec<hostable_core::ServerRecord>>()
                .map_err(|error| error.to_string()),
            Ok(response) => Err(error_text(response)),
            Err(error) => Err(error.to_string()),
        };
        match result {
            Ok(servers) => {
                self.connected = Some(true);
                self.servers = servers;
                if self.selected_server >= self.servers.len() {
                    self.selected_server = self.servers.len().saturating_sub(1);
                }
                self.refresh_server_statuses();
            }
            Err(error) => {
                self.connected = Some(false);
                self.status = format!("hostabled unavailable: {error}");
                self.servers.clear();
                self.server_statuses.clear();
                self.selected_server = 0;
            }
        }
    }

    fn refresh_server_statuses(&mut self) {
        self.last_status_poll = Instant::now();
        let ids = self
            .servers
            .iter()
            .map(|server| server.id.0.clone())
            .collect::<Vec<_>>();
        // Build the new map first so statuses never flash back to "unknown".
        let mut statuses = HashMap::new();
        for server_id in ids {
            let url = format!(
                "{}/api/servers/{}/status",
                self.daemon_url.trim_end_matches('/'),
                server_id
            );
            let active = self
                .with_operator_token(self.client.get(url))
                .timeout(Duration::from_secs(4))
                .send()
                .and_then(|response| response.error_for_status())
                .and_then(|response| response.json::<ServiceStatusResponse>())
                .map(|response| response.active)
                .unwrap_or_else(|_| "unknown".to_string());
            statuses.insert(server_id, active);
        }
        self.server_statuses = statuses;
    }

    /// Called on every idle tick of the event loop. Keeps statuses fresh.
    pub fn tick(&mut self) {
        if self.confirm.is_some() || self.active_screen != Screen::Server {
            return;
        }
        if self.mode == Mode::Manage {
            self.manage_tick();
            return;
        }
        if !matches!(self.mode, Mode::Normal | Mode::ServerDetail) {
            return;
        }
        if self.connected == Some(false) {
            if self.last_list_poll.elapsed() >= Duration::from_secs(5) {
                self.refresh_servers();
            }
        } else if self.last_list_poll.elapsed() >= Duration::from_secs(30) {
            self.refresh_servers();
        } else if self.last_status_poll.elapsed() >= Duration::from_secs(3) {
            self.refresh_server_statuses();
        }
    }

    fn update_toast(&mut self) {
        if self.status != self.toast_seen {
            self.toast_seen = self.status.clone();
            self.toast_at = Instant::now();
            self.toast_level = classify_status(&self.status);
        }
    }

    fn active_toast(&self) -> Option<(Level, &str)> {
        let level = self.toast_level?;
        let seconds = match level {
            Level::Info | Level::Ok => 4,
            Level::Warn => 8,
            Level::Error => 10,
        };
        (self.toast_at.elapsed() < Duration::from_secs(seconds))
            .then_some((level, self.toast_seen.as_str()))
    }

    fn copy_join_address(&mut self) {
        let Some(server) = self.servers.get(self.selected_server) else {
            return;
        };
        let Some(port) = server.ports.first() else {
            self.status = "This server has no public port to copy".to_string();
            return;
        };
        let Some(ip) = self.admin_web.public_ip.clone() else {
            self.status = "Public IP not known yet, try again in a moment".to_string();
            self.refresh_public_ip();
            return;
        };
        let address = format!("{ip}:{}", port.port);
        copy_to_clipboard(&address);
        self.status = format!("Copied {address} (needs a terminal that supports OSC 52)");
    }

    fn handle_confirm_key(&mut self, code: KeyCode) {
        let Some(confirm) = self.confirm.as_mut() else {
            return;
        };
        match code {
            KeyCode::Char('n') | KeyCode::Esc => {
                self.confirm = None;
                self.status = "Cancelled, players were not interrupted".to_string();
            }
            KeyCode::Tab | KeyCode::Left | KeyCode::Right | KeyCode::BackTab => {
                confirm.focus_confirm = !confirm.focus_confirm;
            }
            KeyCode::Char('y') => self.confirm_force(),
            KeyCode::Enter => {
                if confirm.focus_confirm {
                    self.confirm_force();
                } else {
                    self.confirm = None;
                    self.status = "Cancelled, players were not interrupted".to_string();
                }
            }
            _ => {}
        }
    }

    fn confirm_force(&mut self) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        match self.call_action(&confirm.server_id, &confirm.action, true) {
            ActionOutcome::Done(warnings) => {
                self.status = action_status(&confirm.server_name, &confirm.ok_message, &warnings)
            }
            ActionOutcome::NeedsConfirm(_) => {}
            ActionOutcome::Failed(error) => {
                self.status = format!("{} failed: {error}", confirm.server_name)
            }
        }
        self.refresh_server_statuses();
    }

    fn call_action(&self, server_id: &str, action: &str, force: bool) -> ActionOutcome {
        let mut url = format!(
            "{}/api/servers/{}/{}",
            self.daemon_url.trim_end_matches('/'),
            server_id,
            action
        );
        if force {
            url.push_str("?force=true");
        }
        let response = match self.with_operator_token(self.client.post(url)).send() {
            Ok(response) => response,
            Err(error) => return ActionOutcome::Failed(error.to_string()),
        };
        let status = response.status();
        let body = response.text().unwrap_or_default();
        let json = serde_json::from_str::<serde_json::Value>(&body).ok();
        if status.is_success() {
            let warnings = json
                .as_ref()
                .and_then(|value| value.get("warnings"))
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            return ActionOutcome::Done(warnings);
        }
        if let Some(value) = &json
            && value.get("code").and_then(|code| code.as_str()) == Some("players_online")
        {
            let players = value.get("players").and_then(|p| p.as_u64()).unwrap_or(1);
            return ActionOutcome::NeedsConfirm(players as u32);
        }
        let message = json
            .as_ref()
            .and_then(|value| value.get("error"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("HTTP {status}"));
        ActionOutcome::Failed(message)
    }

    fn server_status(&self, server_id: &str) -> &str {
        self.server_statuses
            .get(server_id)
            .map(String::as_str)
            .unwrap_or("unknown")
    }

    pub fn refresh_settings(&mut self) {
        let url = format!("{}/api/settings", self.daemon_url.trim_end_matches('/'));
        let request = self.with_operator_token(self.client.get(url));

        match request
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json::<HostableSettings>())
        {
            Ok(settings) => {
                if let Some(s3) = settings.s3 {
                    self.s3 = S3Draft::from_config(s3);
                }
                self.s3.local_keep_last = settings.backup.local_keep_last.to_string();
                self.s3.s3_keep_last = settings.backup.s3_keep_last.to_string();
                self.admin_web.enabled = settings.admin_web.enabled;
                self.admin_web.domain = settings.admin_web.domain.unwrap_or_default();
                self.status = "Settings loaded".to_string();
            }
            Err(error) => {
                if self.active_screen == Screen::Settings {
                    self.status = format!("Failed to load settings: {error}");
                }
            }
        }
    }

    pub fn refresh_public_ip(&mut self) {
        let url = format!(
            "{}/api/network/public-ip",
            self.daemon_url.trim_end_matches('/')
        );
        let request = self
            .with_operator_token(self.client.get(url))
            .timeout(Duration::from_secs(5));
        if let Ok(response) = request
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json::<PublicIpResponse>())
        {
            self.admin_web.public_ip = Some(response.ip);
        }
    }

    pub fn next_tab(&mut self) {
        self.active_screen = match self.active_screen {
            Screen::Server => Screen::Settings,
            Screen::Settings => Screen::Server,
        };
        if self.active_screen == Screen::Settings {
            self.refresh_settings();
            self.refresh_public_ip();
        }
    }

    pub fn previous_tab(&mut self) {
        self.next_tab();
    }

    pub fn next_server(&mut self) {
        if self.mode == Mode::CreateServer {
            self.create.next_field();
            return;
        }
        if self.active_screen != Screen::Server || self.servers.is_empty() {
            return;
        }
        self.selected_server = (self.selected_server + 1) % self.servers.len();
    }

    pub fn previous_server(&mut self) {
        if self.mode == Mode::CreateServer {
            self.create.previous_field();
            return;
        }
        if self.active_screen != Screen::Server || self.servers.is_empty() {
            return;
        }
        self.selected_server = if self.selected_server == 0 {
            self.servers.len() - 1
        } else {
            self.selected_server - 1
        };
    }

    pub fn start_selected(&mut self) {
        self.run_server_action("start", "started");
    }

    pub fn stop_selected(&mut self) {
        self.run_server_action("stop", "stopped");
    }

    pub fn restart_selected(&mut self) {
        self.run_server_action("restart", "restarted");
    }

    pub fn backup_selected(&mut self) {
        self.run_server_action("backups/run", "backup finished");
    }

    pub fn open_server_detail(&mut self) {
        if self.active_screen != Screen::Server {
            return;
        }
        let Some(server) = self.servers.get(self.selected_server) else {
            self.status = "No server selected".to_string();
            return;
        };
        self.mode = Mode::ServerDetail;
        self.status = format!("Opened {}", server.name);
    }

    pub fn open_create_server(&mut self) {
        self.active_screen = Screen::Server;
        self.mode = Mode::CreateServer;
        self.create = CreateDraft::default();
        self.status.clear();
    }

    fn open_choice_picker(&mut self, target: ChoiceTarget) {
        let options = self.choice_options_for(target);
        let selected = options
            .iter()
            .position(|option| self.choice_is_current(target, option))
            .unwrap_or(0);
        self.choice_picker = ChoicePicker {
            target,
            query: String::new(),
            cursor: 0,
            selected,
        };
        self.mode = Mode::PickChoice;
        self.status = "Filter choices, use up/down, Enter selects".to_string();
    }

    fn close_choice_picker(&mut self, selected: bool) {
        self.mode = match self.choice_picker.target {
            ChoiceTarget::CreateGame | ChoiceTarget::CreateType => Mode::CreateServer,
            ChoiceTarget::ConnectGame => Mode::ConnectServer,
        };
        if !selected {
            self.status = "Choice cancelled".to_string();
        }
    }

    fn choice_title(&self) -> &'static str {
        match self.choice_picker.target {
            ChoiceTarget::CreateGame => "Choose Game",
            ChoiceTarget::CreateType => "Choose Server Type",
            ChoiceTarget::ConnectGame => "Choose Game",
        }
    }

    fn choice_options_for(&self, target: ChoiceTarget) -> Vec<ChoiceOption> {
        match target {
            ChoiceTarget::CreateGame | ChoiceTarget::ConnectGame => vec![
                ChoiceOption {
                    label: "Minecraft",
                    value: ChoiceValue::Game(hostable_core::GameKind::Minecraft),
                },
                ChoiceOption {
                    label: "ARK: Survival Evolved",
                    value: ChoiceValue::Game(hostable_core::GameKind::ArkSurvivalEvolved),
                },
                ChoiceOption {
                    label: "Palworld",
                    value: ChoiceValue::Game(hostable_core::GameKind::Palworld),
                },
            ],
            ChoiceTarget::CreateType => {
                if self.create.game == hostable_core::GameKind::ArkSurvivalEvolved {
                    vec![ChoiceOption {
                        label: "ARK",
                        value: ChoiceValue::ServerType(
                            hostable_core::ServerType::ArkSurvivalEvolved,
                        ),
                    }]
                } else if self.create.game == hostable_core::GameKind::Palworld {
                    vec![ChoiceOption {
                        label: "Palworld",
                        value: ChoiceValue::ServerType(hostable_core::ServerType::Palworld),
                    }]
                } else {
                    vec![
                        ChoiceOption {
                            label: "Vanilla",
                            value: ChoiceValue::ServerType(hostable_core::ServerType::Vanilla),
                        },
                        ChoiceOption {
                            label: "Paper",
                            value: ChoiceValue::ServerType(hostable_core::ServerType::Paper),
                        },
                        ChoiceOption {
                            label: "Fabric",
                            value: ChoiceValue::ServerType(hostable_core::ServerType::Fabric),
                        },
                    ]
                }
            }
        }
    }

    fn filtered_choice_options(&self) -> Vec<ChoiceOption> {
        let query = self.choice_picker.query.trim().to_ascii_lowercase();
        self.choice_options_for(self.choice_picker.target)
            .into_iter()
            .filter(|option| {
                query.is_empty() || option.label.to_ascii_lowercase().contains(query.as_str())
            })
            .collect()
    }

    fn choice_is_current(&self, target: ChoiceTarget, option: &ChoiceOption) -> bool {
        match (target, &option.value) {
            (ChoiceTarget::CreateGame, ChoiceValue::Game(game)) => self.create.game == *game,
            (ChoiceTarget::ConnectGame, ChoiceValue::Game(game)) => self.connect.game == *game,
            (ChoiceTarget::CreateType, ChoiceValue::ServerType(server_type)) => {
                self.create.server_type == *server_type
            }
            _ => false,
        }
    }

    fn next_choice(&mut self) {
        let options = self.filtered_choice_options();
        if !options.is_empty() {
            self.choice_picker.selected = (self.choice_picker.selected + 1) % options.len();
        }
    }

    fn previous_choice(&mut self) {
        let options = self.filtered_choice_options();
        if options.is_empty() {
            return;
        }
        self.choice_picker.selected = if self.choice_picker.selected == 0 {
            options.len() - 1
        } else {
            self.choice_picker.selected - 1
        };
    }

    fn clamp_choice_selection(&mut self) {
        let len = self.filtered_choice_options().len();
        if len == 0 {
            self.choice_picker.selected = 0;
        } else if self.choice_picker.selected >= len {
            self.choice_picker.selected = len - 1;
        }
    }

    fn apply_choice(&mut self) {
        let Some(option) = self
            .filtered_choice_options()
            .get(self.choice_picker.selected)
            .cloned()
        else {
            self.status = "No choice selected".to_string();
            return;
        };
        match (self.choice_picker.target, option.value) {
            (ChoiceTarget::CreateGame, ChoiceValue::Game(game)) => {
                self.create.set_game(game);
                self.create.dry_run_reviewed = false;
            }
            (ChoiceTarget::ConnectGame, ChoiceValue::Game(game)) => {
                self.connect.set_game(game);
            }
            (ChoiceTarget::CreateType, ChoiceValue::ServerType(server_type)) => {
                self.create.server_type = server_type;
                self.create.dry_run_reviewed = false;
            }
            _ => {}
        }
        self.close_choice_picker(true);
        self.status = format!("Selected {}", option.label);
    }

    pub fn open_connect_server(&mut self) {
        self.active_screen = Screen::Server;
        self.mode = Mode::ConnectServer;
        self.connect = ConnectDraft::default();
        self.status = "Connect existing folder".to_string();
    }

    pub fn open_s3_settings(&mut self) {
        if self.active_screen != Screen::Settings {
            return;
        }
        self.mode = Mode::EditS3;
        self.s3.reset_cursor();
        self.status = "Editing S3 settings".to_string();
    }

    pub fn open_admin_web_domain(&mut self) {
        if self.active_screen != Screen::Settings {
            return;
        }
        self.mode = Mode::EditAdminWeb;
        self.admin_web.cursor = self.admin_web.domain.chars().count();
        self.status = "Editing admin web domain".to_string();
    }

    pub fn open_admin_password(&mut self) {
        if self.active_screen != Screen::Settings {
            return;
        }
        self.mode = Mode::EditAdminPassword;
        self.admin_password = AdminPasswordDraft::default();
        self.status = "Set web admin password".to_string();
    }

    pub fn toggle_admin_web(&mut self) {
        if self.active_screen != Screen::Settings {
            return;
        }
        self.admin_web.enabled = !self.admin_web.enabled;
        self.save_admin_web_settings();
    }

    pub fn open_config_editor(&mut self) {
        if self.active_screen != Screen::Server {
            return;
        }
        let Some(server) = self.servers.get(self.selected_server) else {
            self.status = "No selected server".to_string();
            return;
        };
        let url = format!(
            "{}/api/servers/{}/config",
            self.daemon_url.trim_end_matches('/'),
            server.id.0
        );
        let request = self.with_operator_token(self.client.get(url));

        match request
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json::<hostable_core::ConfigDocument>())
        {
            Ok(document) => {
                self.config_editor = ConfigEditor {
                    server_id: server.id.0.clone(),
                    document: Some(document),
                    selected_file: 0,
                    buffer: String::new(),
                };
                self.config_editor.load_selected_buffer();
                self.mode = Mode::EditConfig;
                self.status = "Config loaded".to_string();
            }
            Err(error) => {
                self.status = format!("Failed to load config: {error}");
            }
        }
    }

    pub fn test_s3_settings(&mut self) {
        if self.active_screen != Screen::Settings {
            return;
        }
        let url = format!(
            "{}/api/settings/s3/test",
            self.daemon_url.trim_end_matches('/')
        );
        let request = self.with_operator_token(self.client.post(url));

        match request
            .send()
            .and_then(|response| response.error_for_status())
        {
            Ok(_) => {
                self.status = "S3 connection test wrote a small object".to_string();
            }
            Err(error) => {
                self.status = format!("S3 test failed: {error}");
            }
        }
    }

    fn render_servers(&self, frame: &mut Frame<'_>, area: Rect) {
        match self.mode {
            Mode::Normal => self.render_server_table(frame, area),
            Mode::ServerDetail => self.render_server_detail(frame, area),
            Mode::CreateServer => self.render_create_server(frame, area),
            Mode::ConnectServer => self.render_connect_server(frame, area),
            Mode::PickChoice => self.render_choice_picker(frame, area),
            Mode::PickModrinth => self.render_modrinth_picker(frame, area),
            Mode::Manage => self.render_manage(frame, area),
            Mode::EditConfig => self.render_config_editor(frame, area),
            Mode::EditS3 | Mode::EditAdminWeb | Mode::EditAdminPassword => {
                self.render_server_table(frame, area)
            }
        }
    }

    fn render_choice_picker(&self, frame: &mut Frame<'_>, area: Rect) {
        frame.render_widget(Clear, area);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(4),
            ])
            .split(area);
        frame.render_widget(
            Paragraph::new(Text::from(vec![Line::from(Span::styled(
                self.choice_title(),
                Style::default()
                    .fg(theme().text)
                    .add_modifier(Modifier::BOLD),
            ))])),
            rows[0],
        );
        render_input_field(
            frame,
            rows[1],
            "Filter",
            &self.choice_picker.query,
            true,
            self.choice_picker.cursor,
        );
        let options = self.filtered_choice_options();
        let items = if options.is_empty() {
            vec![ListItem::new(Span::styled(
                "No matches",
                Style::default().fg(theme().muted),
            ))]
        } else {
            options
                .iter()
                .map(|option| ListItem::new(option.label))
                .collect()
        };
        let mut state = ListState::default();
        if !options.is_empty() {
            state.select(Some(self.choice_picker.selected));
        }
        let list = List::new(items).highlight_symbol("> ").highlight_style(
            Style::default()
                .fg(theme().accent)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_stateful_widget(list, rows[2], &mut state);
    }

    fn render_connect_server(&self, frame: &mut Frame<'_>, area: Rect) {
        let game = game_label(self.connect.game);
        let submit = if self.connect.dry_run_reviewed {
            "Enter connect"
        } else {
            "Enter dry-run"
        };
        let viewport = form_viewport(area, 3, self.connect.field.index());
        frame.render_widget(
            Paragraph::new(Text::from(vec![Line::from(vec![
                Span::styled(
                    "Connect Folder",
                    Style::default()
                        .fg(theme().text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {submit}"), Style::default().fg(theme().muted)),
                Span::styled(
                    format!("  {}", viewport.range_label()),
                    Style::default().fg(theme().muted),
                ),
            ])])),
            viewport.header,
        );
        for (index, row) in viewport.rows {
            match index {
                0 => render_select_field(
                    frame,
                    row,
                    "Game",
                    game,
                    self.connect.field == ConnectField::Game,
                    "Enter opens searchable list",
                ),
                1 => render_input_field(
                    frame,
                    row,
                    "Name",
                    &self.connect.name,
                    self.connect.field == ConnectField::Name,
                    self.connect.cursor,
                ),
                2 => render_input_field(
                    frame,
                    row,
                    "Folder",
                    &self.connect.path,
                    self.connect.field == ConnectField::Path,
                    self.connect.cursor,
                ),
                _ => {}
            }
        }
    }

    fn render_config_editor(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(document) = &self.config_editor.document else {
            frame.render_widget(Paragraph::new("No config loaded"), area);
            return;
        };
        let selected = document.files.get(self.config_editor.selected_file);
        let files = document
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                let marker = if index == self.config_editor.selected_file {
                    ">"
                } else {
                    " "
                };
                format!("{marker} {}", file.path.display())
            })
            .collect::<Vec<_>>()
            .join("\n");
        let title = selected
            .map(|file| file.path.display().to_string())
            .unwrap_or_else(|| "No editable files".to_string());
        let preview = self
            .config_editor
            .buffer
            .lines()
            .take(16)
            .collect::<Vec<_>>()
            .join("\n");
        let text = format!(
            "Config editor\n{}\n\nFiles\n{}\n\nEditing\n{}\n\n{}\n\nF2 save   Tab next file   Esc close",
            document.title, files, title, preview
        );
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), area);
    }

    fn render_settings(&self, frame: &mut Frame<'_>, area: Rect) {
        if self.mode == Mode::EditS3 {
            let viewport = form_viewport(area, 8, self.s3.field.index());
            frame.render_widget(
                Paragraph::new(Text::from(vec![
                    Line::from(vec![
                        Span::styled(
                            "Backup Settings",
                            Style::default()
                                .fg(theme().text)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("  {}", viewport.range_label()),
                            Style::default().fg(theme().muted),
                        ),
                    ]),
                    Line::from(action_spans(&[
                        ("tab", "next field"),
                        ("enter", "save"),
                        ("esc", "cancel"),
                    ])),
                ])),
                viewport.header,
            );
            for (index, row) in viewport.rows {
                match index {
                    0 => render_input_field(
                        frame,
                        row,
                        "Local backups to keep",
                        &self.s3.local_keep_last,
                        self.s3.field == S3Field::LocalKeepLast,
                        self.s3.cursor,
                    ),
                    1 => render_input_field(
                        frame,
                        row,
                        "S3 backups to keep",
                        &self.s3.s3_keep_last,
                        self.s3.field == S3Field::S3KeepLast,
                        self.s3.cursor,
                    ),
                    2 => render_input_field(
                        frame,
                        row,
                        "Bucket",
                        &self.s3.bucket,
                        self.s3.field == S3Field::Bucket,
                        self.s3.cursor,
                    ),
                    3 => render_input_field(
                        frame,
                        row,
                        "Region",
                        &self.s3.region,
                        self.s3.field == S3Field::Region,
                        self.s3.cursor,
                    ),
                    4 => render_input_field(
                        frame,
                        row,
                        "Endpoint",
                        &self.s3.endpoint_url,
                        self.s3.field == S3Field::Endpoint,
                        self.s3.cursor,
                    ),
                    5 => render_input_field(
                        frame,
                        row,
                        "Prefix",
                        &self.s3.prefix,
                        self.s3.field == S3Field::Prefix,
                        self.s3.cursor,
                    ),
                    6 => render_input_field(
                        frame,
                        row,
                        "Access env",
                        &self.s3.access_key_env,
                        self.s3.field == S3Field::AccessKeyEnv,
                        self.s3.cursor,
                    ),
                    7 => render_input_field(
                        frame,
                        row,
                        "Secret env",
                        &self.s3.secret_key_env,
                        self.s3.field == S3Field::SecretKeyEnv,
                        self.s3.cursor,
                    ),
                    _ => {}
                }
            }
            return;
        }

        if self.mode == Mode::EditAdminWeb {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(2),
                ])
                .split(area);
            frame.render_widget(
                Paragraph::new(Text::from(vec![
                    Line::from(Span::styled(
                        "Admin Web",
                        Style::default()
                            .fg(theme().text)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(action_spans(&[("enter", "save"), ("esc", "cancel")])),
                ])),
                rows[0],
            );
            render_input_field(
                frame,
                rows[1],
                "Domain",
                &self.admin_web.domain,
                true,
                self.admin_web.cursor,
            );
            frame.render_widget(
                Paragraph::new(format!(
                    "Point your domain provider at {}.",
                    self.admin_web.public_ip_label()
                ))
                .style(Style::default().fg(theme().muted))
                .wrap(Wrap { trim: true }),
                rows[2],
            );
            return;
        }

        if self.mode == Mode::EditAdminPassword {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(2),
                ])
                .split(area);
            frame.render_widget(
                Paragraph::new(Text::from(vec![
                    Line::from(Span::styled(
                        "Admin Web Password",
                        Style::default()
                            .fg(theme().text)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(action_spans(&[
                        ("tab", "next field"),
                        ("enter", "save"),
                        ("esc", "cancel"),
                    ])),
                ])),
                rows[0],
            );
            render_secret_input_field(
                frame,
                rows[1],
                "New password",
                &self.admin_password.new_password,
                self.admin_password.field == AdminPasswordField::NewPassword,
                self.admin_password.cursor,
            );
            render_secret_input_field(
                frame,
                rows[2],
                "Confirm password",
                &self.admin_password.confirm_password,
                self.admin_password.field == AdminPasswordField::ConfirmPassword,
                self.admin_password.cursor,
            );
            frame.render_widget(
                Paragraph::new("Must be at least 10 characters. This sets or resets the password used by the browser admin UI.")
                    .style(Style::default().fg(theme().muted))
                    .wrap(Wrap { trim: true }),
                rows[3],
            );
            return;
        }

        let admin_state = if self.admin_web.enabled { "on" } else { "off" };
        let admin_target = self.admin_web.display_target();
        let text = Text::from(vec![
            Line::from(Span::styled(
                "Settings",
                Style::default()
                    .fg(theme().text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            section_heading("S3 backups"),
            muted_kv("local backup copies", self.s3.local_keep_last.clone()),
            muted_kv("S3 backup copies", self.s3.s3_keep_last.clone()),
            Line::from(Span::styled(
                "Retention count: keeps the newest N backups and prunes older ones.",
                Style::default().fg(theme().muted),
            )),
            muted_kv("bucket", empty_label(&self.s3.bucket).to_string()),
            muted_kv("region", empty_label(&self.s3.region).to_string()),
            muted_kv("endpoint", empty_label(&self.s3.endpoint_url).to_string()),
            muted_kv("prefix", empty_label(&self.s3.prefix).to_string()),
            muted_kv(
                "access env",
                empty_label(&self.s3.access_key_env).to_string(),
            ),
            muted_kv(
                "secret env",
                empty_label(&self.s3.secret_key_env).to_string(),
            ),
            Line::from(action_spans(&[("e", "edit"), ("t", "test connection")])),
            Line::from(""),
            section_heading("Admin web"),
            muted_kv("state", admin_state.to_string()),
            muted_kv("target", admin_target),
            muted_kv("VPS IP", self.admin_web.public_ip_label()),
            Line::from(action_spans(&[
                ("w", "toggle"),
                ("d", "domain"),
                ("p", "password"),
            ])),
            Line::from(Span::styled(
                "Use this IP at your domain provider.",
                Style::default().fg(theme().muted),
            )),
        ]);
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), area);
    }

    fn run_server_action(&mut self, action: &str, ok_message: &str) {
        if self.active_screen != Screen::Server {
            return;
        }
        let Some(server) = self.servers.get(self.selected_server) else {
            self.status = "No server selected".to_string();
            return;
        };
        let server_id = server.id.0.clone();
        let server_name = server.name.clone();
        match self.call_action(&server_id, action, false) {
            ActionOutcome::Done(warnings) => {
                self.status = action_status(&server_name, ok_message, &warnings);
            }
            ActionOutcome::NeedsConfirm(players) => {
                self.confirm = Some(Confirm {
                    action: action.to_string(),
                    ok_message: ok_message.to_string(),
                    server_id,
                    server_name,
                    players,
                    focus_confirm: false,
                });
                // Nothing changed yet, so there is nothing to refresh.
                return;
            }
            ActionOutcome::Failed(error) => {
                self.status = format!("{server_name} failed: {error}");
            }
        }
        if !matches!(action, "backups/run") {
            self.refresh_server_statuses();
        }
    }

    fn handle_choice_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => self.close_choice_picker(false),
            KeyCode::Down => self.next_choice(),
            KeyCode::Up => self.previous_choice(),
            KeyCode::Left => {
                move_cursor_left(&self.choice_picker.query, &mut self.choice_picker.cursor);
            }
            KeyCode::Right => {
                move_cursor_right(&self.choice_picker.query, &mut self.choice_picker.cursor);
            }
            KeyCode::Home => self.choice_picker.cursor = 0,
            KeyCode::End => self.choice_picker.cursor = self.choice_picker.query.chars().count(),
            KeyCode::Backspace => {
                backspace_at(
                    &mut self.choice_picker.query,
                    &mut self.choice_picker.cursor,
                );
                self.clamp_choice_selection();
            }
            KeyCode::Delete => {
                delete_at(
                    &mut self.choice_picker.query,
                    &mut self.choice_picker.cursor,
                );
                self.clamp_choice_selection();
            }
            KeyCode::Enter => self.apply_choice(),
            KeyCode::Char(ch) => {
                insert_at(
                    &mut self.choice_picker.query,
                    &mut self.choice_picker.cursor,
                    ch,
                );
                self.clamp_choice_selection();
            }
            _ => {}
        }
    }

    fn handle_connect_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status = "Connect folder cancelled".to_string();
            }
            KeyCode::Tab | KeyCode::Down => self.connect.next_field(),
            KeyCode::BackTab | KeyCode::Up => self.connect.previous_field(),
            KeyCode::Left => self.connect.move_cursor_left(),
            KeyCode::Right => self.connect.move_cursor_right(),
            KeyCode::Home => self.connect.move_cursor_start(),
            KeyCode::End => self.connect.move_cursor_end(),
            KeyCode::Backspace => self.connect.backspace(),
            KeyCode::Delete => self.connect.delete_char(),
            KeyCode::Enter => {
                if self.connect.field == ConnectField::Game {
                    self.open_choice_picker(ChoiceTarget::ConnectGame);
                } else {
                    self.submit_connect_server();
                }
            }
            KeyCode::Char(ch) => self.connect.insert_char(ch),
            _ => {}
        }
    }

    fn submit_connect_server(&mut self) {
        let dry_run = !self.connect.dry_run_reviewed;
        let request = ImportServerRequest {
            game: self.connect.game,
            name: self.connect.name.trim().to_string(),
            install_path: PathBuf::from(self.connect.path.trim()),
            dry_run,
        };
        if request.name.is_empty() {
            self.status = "Name is required".to_string();
            return;
        }
        let url = format!(
            "{}/api/servers/import",
            self.daemon_url.trim_end_matches('/')
        );
        let http = self.with_operator_token(self.client.post(url).json(&request));

        match http.send().and_then(|response| response.error_for_status()) {
            Ok(_) if dry_run => {
                self.connect.dry_run_reviewed = true;
                self.status = "Import dry-run succeeded. Press Enter again to connect.".to_string();
            }
            Ok(_) => {
                self.mode = Mode::Normal;
                self.status = "Server connected".to_string();
                self.refresh_servers();
            }
            Err(error) => {
                self.status = format!("Connect folder failed: {error}");
            }
        }
    }

    fn handle_s3_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status = "S3 settings edit cancelled".to_string();
            }
            KeyCode::Tab | KeyCode::Down => self.s3.next_field(),
            KeyCode::BackTab | KeyCode::Up => self.s3.previous_field(),
            KeyCode::Left => self.s3.move_cursor_left(),
            KeyCode::Right => self.s3.move_cursor_right(),
            KeyCode::Home => self.s3.move_cursor_start(),
            KeyCode::End => self.s3.move_cursor_end(),
            KeyCode::Backspace => self.s3.backspace(),
            KeyCode::Delete => self.s3.delete_char(),
            KeyCode::Enter => self.save_s3_settings(),
            KeyCode::Char(ch) => self.s3.insert_char(ch),
            _ => {}
        }
    }

    fn handle_admin_web_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status = "Admin web edit cancelled".to_string();
            }
            KeyCode::Left => move_cursor_left(&self.admin_web.domain, &mut self.admin_web.cursor),
            KeyCode::Right => {
                move_cursor_right(&self.admin_web.domain, &mut self.admin_web.cursor);
            }
            KeyCode::Home => self.admin_web.cursor = 0,
            KeyCode::End => self.admin_web.cursor = self.admin_web.domain.chars().count(),
            KeyCode::Backspace => {
                backspace_at(&mut self.admin_web.domain, &mut self.admin_web.cursor);
            }
            KeyCode::Delete => {
                delete_at(&mut self.admin_web.domain, &mut self.admin_web.cursor);
            }
            KeyCode::Enter => self.save_admin_web_settings(),
            KeyCode::Char(ch) => {
                insert_at(&mut self.admin_web.domain, &mut self.admin_web.cursor, ch);
            }
            _ => {}
        }
    }

    fn handle_admin_password_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status = "Admin password edit cancelled".to_string();
            }
            KeyCode::Tab | KeyCode::Down => self.admin_password.next_field(),
            KeyCode::BackTab | KeyCode::Up => self.admin_password.previous_field(),
            KeyCode::Left => self.admin_password.move_cursor_left(),
            KeyCode::Right => self.admin_password.move_cursor_right(),
            KeyCode::Home => self.admin_password.move_cursor_start(),
            KeyCode::End => self.admin_password.move_cursor_end(),
            KeyCode::Backspace => self.admin_password.backspace(),
            KeyCode::Delete => self.admin_password.delete_char(),
            KeyCode::Enter => self.save_admin_password(),
            KeyCode::Char(ch) => self.admin_password.insert_char(ch),
            _ => {}
        }
    }

    fn search_modrinth(&mut self) {
        if self.modrinth_picker.query.trim().is_empty() {
            self.status = "Type a search first".to_string();
            return;
        }

        let project_type = self.modrinth_picker.kind.as_query_value();
        let modpack = self.modrinth_picker.kind == ModrinthPickKind::Modpack;
        let game_version = non_empty(self.create.version.clone());
        if !modpack && game_version.is_none() {
            self.status = "Set a version first".to_string();
            return;
        }
        let mut request = self
            .client
            .get(format!(
                "{}/api/modrinth/search",
                self.daemon_url.trim_end_matches('/')
            ))
            .query(&[
                ("query", self.modrinth_picker.query.trim()),
                ("project_type", project_type),
            ]);
        // A modpack brings its own Minecraft version, so only single mods are
        // narrowed to the version typed above.
        if let (false, Some(version)) = (modpack, game_version.as_deref()) {
            request = request.query(&[("game_version", version)]);
        }
        if let Some(loader) = picker_loader(modpack, &self.create.server_type) {
            request = request.query(&[("loader", loader)]);
        }
        let request = self.with_operator_token(request);

        match request
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json::<hostable_core::ModrinthSearchResponse>())
        {
            Ok(response) => {
                self.modrinth_picker.results = response.hits;
                self.modrinth_picker.selected = 0;
                self.modrinth_picker.dirty = false;
                self.status.clear();
            }
            Err(error) => {
                self.status = if error.is_connect() {
                    "Search needs hostabled".to_string()
                } else {
                    format!("Search failed: {error}")
                };
            }
        }
    }

    fn select_modrinth_result(&mut self) {
        let Some(hit) = self
            .modrinth_picker
            .results
            .get(self.modrinth_picker.selected)
            .cloned()
        else {
            return;
        };
        if self.modrinth_picker.kind == ModrinthPickKind::Mod
            && self.create.modrinth.contains(&hit.project_id)
        {
            self.status = "Already added".to_string();
            return;
        }
        let modpack = self.modrinth_picker.kind == ModrinthPickKind::Modpack;
        let game_version = non_empty(self.create.version.clone());
        if !modpack && game_version.is_none() {
            self.status = "Set a version first".to_string();
            return;
        }
        let project_type = self.modrinth_picker.kind.as_query_value();
        let mut request = self
            .client
            .get(format!(
                "{}/api/modrinth/projects/{}/install",
                self.daemon_url.trim_end_matches('/'),
                hit.project_id
            ))
            .query(&[("project_type", project_type)]);
        if let (false, Some(version)) = (modpack, game_version.as_deref()) {
            request = request.query(&[("game_version", version)]);
        }
        if let Some(loader) = picker_loader(modpack, &self.create.server_type) {
            request = request.query(&[("loader", loader)]);
        }
        let request = self.with_operator_token(request);

        match request
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json::<hostable_core::ModrinthInstallSelection>())
        {
            Ok(selection) => {
                let selected = SelectedModrinthProject {
                    title: hit.title.clone(),
                    selection,
                };
                self.create.dry_run_reviewed = false;
                match self.modrinth_picker.kind {
                    ModrinthPickKind::Modpack => {
                        self.status = self.apply_pack(selected);
                        self.mode = Mode::CreateServer;
                    }
                    ModrinthPickKind::Mod => {
                        // Stay in the picker so several mods can be added in a row.
                        self.create.modrinth.mods.push(selected);
                        self.status = format!("Added {}", hit.title);
                    }
                }
            }
            Err(error) => {
                self.status = format!("Add failed: {error}");
            }
        }
    }

    fn handle_config_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::ServerDetail;
                self.status = "Closed config editor".to_string();
            }
            KeyCode::Tab => {
                self.config_editor.next_file();
                self.status = "Switched config file".to_string();
            }
            KeyCode::F(2) => self.save_config_editor(),
            KeyCode::Enter => {
                self.config_editor.buffer.push('\n');
            }
            KeyCode::Backspace => {
                self.config_editor.buffer.pop();
            }
            KeyCode::Char(ch) => {
                self.config_editor.buffer.push(ch);
            }
            _ => {}
        }
    }

    fn save_config_editor(&mut self) {
        let Some(document) = &mut self.config_editor.document else {
            self.status = "No config loaded".to_string();
            return;
        };
        let Some(file) = document.files.get_mut(self.config_editor.selected_file) else {
            self.status = "No config file selected".to_string();
            return;
        };
        file.contents = self.config_editor.buffer.clone();
        let patch = hostable_core::ConfigPatch {
            files: vec![file.clone()],
        };
        let url = format!(
            "{}/api/servers/{}/config",
            self.daemon_url.trim_end_matches('/'),
            self.config_editor.server_id
        );
        let request = self.with_operator_token(self.client.put(url).json(&patch));

        match request
            .send()
            .and_then(|response| response.error_for_status())
        {
            Ok(_) => {
                self.status = "Config saved".to_string();
            }
            Err(error) => {
                self.status = format!("Failed to save config: {error}");
            }
        }
    }

    fn save_s3_settings(&mut self) {
        let backup_settings = match self.s3.to_backup_settings() {
            Ok(settings) => settings,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        let backup_url = format!(
            "{}/api/settings/backups",
            self.daemon_url.trim_end_matches('/')
        );
        let backup_request =
            self.with_operator_token(self.client.put(backup_url).json(&backup_settings));
        if let Err(error) = backup_request
            .send()
            .and_then(|response| response.error_for_status())
        {
            self.status = format!("Failed to save backup settings: {error}");
            return;
        }
        if self.s3.bucket.trim().is_empty() {
            self.mode = Mode::Normal;
            self.status = "Backup retention saved".to_string();
            return;
        }

        let config = match self.s3.to_config() {
            Ok(config) => config,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        let url = format!("{}/api/settings/s3", self.daemon_url.trim_end_matches('/'));
        let request = self.with_operator_token(self.client.put(url).json(&config));

        match request
            .send()
            .and_then(|response| response.error_for_status())
        {
            Ok(_) => {
                self.mode = Mode::Normal;
                self.status = "Backup settings saved".to_string();
            }
            Err(error) => {
                self.status = format!("Failed to save S3 settings: {error}");
            }
        }
    }

    fn save_admin_web_settings(&mut self) {
        let request = AdminWebSettingsRequest {
            enabled: self.admin_web.enabled,
            domain: non_empty(self.admin_web.domain.clone()),
        };
        let url = format!(
            "{}/api/settings/admin-web",
            self.daemon_url.trim_end_matches('/')
        );
        let http = self.with_operator_token(self.client.put(url).json(&request));

        match http.send().and_then(|response| response.error_for_status()) {
            Ok(_) => {
                self.mode = Mode::Normal;
                self.status = if self.admin_web.enabled {
                    format!(
                        "Admin web enabled. Point your domain at {}",
                        self.admin_web.public_ip_label()
                    )
                } else {
                    "Admin web disabled".to_string()
                };
            }
            Err(error) => {
                self.status = format!("Failed to save admin web settings: {error}");
            }
        }
    }

    fn save_admin_password(&mut self) {
        let password = &self.admin_password.new_password;
        if password.len() < 10 {
            self.status = "Admin password must be at least 10 characters".to_string();
            return;
        }
        if password != &self.admin_password.confirm_password {
            self.status = "Admin password confirmation does not match".to_string();
            return;
        }

        let request = AdminPasswordRequest {
            new_password: password.clone(),
        };
        let url = format!(
            "{}/api/admin/password",
            self.daemon_url.trim_end_matches('/')
        );
        let http = self.with_operator_token(self.client.put(url).json(&request));

        match http.send().and_then(|response| response.error_for_status()) {
            Ok(_) => {
                self.mode = Mode::Normal;
                self.admin_password = AdminPasswordDraft::default();
                self.status = "Admin web password updated".to_string();
            }
            Err(error) => {
                self.status = format!("Failed to update admin web password: {error}");
            }
        }
    }

    fn submit_create_server(&mut self) {
        if !self.create.accepted_license {
            self.status = "Accept the license first".to_string();
            return;
        }
        let memory_mb = match self.create.memory_mb.parse::<u32>() {
            Ok(memory_mb) => memory_mb,
            Err(_) => {
                self.status = "Memory must be a number".to_string();
                return;
            }
        };
        let dry_run = !self.create.dry_run_reviewed;
        let extra = self.create.extra_json();
        let request = CreateServerRequest {
            game: self.create.game,
            input: hostable_core::CreateServerInput {
                name: self.create.name.clone(),
                install_path: PathBuf::from(&self.create.path),
                server_type: self.create.server_type.clone(),
                game_version: non_empty(self.create.version.clone()),
                loader_version: None,
                build_version: None,
                memory_mb: Some(memory_mb),
                accepted_license: self.create.accepted_license,
                extra,
            },
            dry_run,
        };
        let url = format!(
            "{}/api/servers/create",
            self.daemon_url.trim_end_matches('/')
        );
        let http = self.with_operator_token(self.client.post(url).json(&request));

        match http.send() {
            Ok(response) if response.status().is_success() => {
                if dry_run {
                    self.create.dry_run_reviewed = true;
                    self.status = "Dry run passed".to_string();
                } else {
                    self.mode = Mode::Normal;
                    self.status = format!("Created {}", self.create.name);
                    self.refresh_servers();
                }
            }
            Ok(response) => self.status = format!("Failed: {}", error_text(response)),
            Err(error) => {
                self.status = if error.is_connect() {
                    "Needs hostabled".to_string()
                } else {
                    format!("Failed: {error}")
                };
            }
        }
    }
}

impl Default for CreateDraft {
    fn default() -> Self {
        Self {
            game: hostable_core::GameKind::Minecraft,
            server_type: hostable_core::ServerType::Fabric,
            field: CreateField::Name,
            cursor: "friends".chars().count(),
            name: "friends".to_string(),
            path: "/srv/hostable/servers/friends".to_string(),
            version: "1.20.4".to_string(),
            memory_mb: "4096".to_string(),
            motd: "A Hostable Minecraft server".to_string(),
            modrinth: ModrinthDraft::default(),
            content_row: 0,
            path_edited: false,
            accepted_license: false,
            dry_run_reviewed: false,
        }
    }
}

impl CreateField {
    fn index(self) -> usize {
        match self {
            CreateField::Game => 0,
            CreateField::Type => 1,
            CreateField::Name => 2,
            CreateField::Path => 3,
            CreateField::Version => 4,
            CreateField::Memory => 5,
            CreateField::Motd => 6,
            CreateField::Content => 7,
            CreateField::License => 8,
        }
    }
}

impl CreateDraft {
    const ORDER: [CreateField; 9] = [
        CreateField::Game,
        CreateField::Type,
        CreateField::Name,
        CreateField::Path,
        CreateField::Version,
        CreateField::Memory,
        CreateField::Motd,
        CreateField::Content,
        CreateField::License,
    ];

    fn step(&mut self, forward: bool) {
        let len = Self::ORDER.len();
        let mut index = self.field.index();
        for _ in 0..len {
            index = if forward {
                (index + 1) % len
            } else {
                (index + len - 1) % len
            };
            if !self.skipped(Self::ORDER[index]) {
                break;
            }
        }
        self.field = Self::ORDER[index];
        self.reset_cursor();
    }

    fn next_field(&mut self) {
        self.step(true);
    }

    fn previous_field(&mut self) {
        self.step(false);
    }

    fn input(&self) -> Option<&String> {
        match self.field {
            CreateField::Name => Some(&self.name),
            CreateField::Path => Some(&self.path),
            CreateField::Version => Some(&self.version),
            CreateField::Memory => Some(&self.memory_mb),
            CreateField::Motd => Some(&self.motd),
            CreateField::Game | CreateField::Type | CreateField::Content | CreateField::License => {
                None
            }
        }
    }

    fn reset_cursor(&mut self) {
        self.cursor = self.input().map(|value| value.chars().count()).unwrap_or(0);
    }

    fn insert_char(&mut self, ch: char) {
        match self.field {
            CreateField::Name => insert_at(&mut self.name, &mut self.cursor, ch),
            CreateField::Path => insert_at(&mut self.path, &mut self.cursor, ch),
            CreateField::Version => insert_at(&mut self.version, &mut self.cursor, ch),
            CreateField::Memory => insert_at(&mut self.memory_mb, &mut self.cursor, ch),
            CreateField::Motd => insert_at(&mut self.motd, &mut self.cursor, ch),
            _ => {}
        }
        self.after_edit();
        self.dry_run_reviewed = false;
    }

    fn backspace(&mut self) {
        match self.field {
            CreateField::Name => backspace_at(&mut self.name, &mut self.cursor),
            CreateField::Path => backspace_at(&mut self.path, &mut self.cursor),
            CreateField::Version => backspace_at(&mut self.version, &mut self.cursor),
            CreateField::Memory => backspace_at(&mut self.memory_mb, &mut self.cursor),
            CreateField::Motd => backspace_at(&mut self.motd, &mut self.cursor),
            _ => {}
        }
        self.after_edit();
        self.dry_run_reviewed = false;
    }

    fn delete_char(&mut self) {
        match self.field {
            CreateField::Name => delete_at(&mut self.name, &mut self.cursor),
            CreateField::Path => delete_at(&mut self.path, &mut self.cursor),
            CreateField::Version => delete_at(&mut self.version, &mut self.cursor),
            CreateField::Memory => delete_at(&mut self.memory_mb, &mut self.cursor),
            CreateField::Motd => delete_at(&mut self.motd, &mut self.cursor),
            _ => {}
        }
        self.after_edit();
        self.dry_run_reviewed = false;
    }

    fn after_edit(&mut self) {
        match self.field {
            CreateField::Name => self.sync_path(),
            CreateField::Path => self.path_edited = true,
            _ => {}
        }
    }

    fn move_cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_cursor_right(&mut self) {
        let len = self.input().map(|value| value.chars().count()).unwrap_or(0);
        self.cursor = self.cursor.saturating_add(1).min(len);
    }

    fn move_cursor_start(&mut self) {
        self.cursor = 0;
    }

    fn move_cursor_end(&mut self) {
        self.reset_cursor();
    }

    fn toggle_license(&mut self) {
        self.accepted_license = !self.accepted_license;
        self.dry_run_reviewed = false;
    }

    fn set_game(&mut self, game: hostable_core::GameKind) {
        self.game = match game {
            hostable_core::GameKind::Minecraft => {
                self.server_type = hostable_core::ServerType::Fabric;
                self.content_row = 0;
                self.version = "1.20.4".to_string();
                self.motd = "A Hostable Minecraft server".to_string();
                hostable_core::GameKind::Minecraft
            }
            hostable_core::GameKind::ArkSurvivalEvolved => {
                self.server_type = hostable_core::ServerType::ArkSurvivalEvolved;
                self.version.clear();
                self.motd = "A Hostable ARK server".to_string();
                self.modrinth = ModrinthDraft::default();
                hostable_core::GameKind::ArkSurvivalEvolved
            }
            hostable_core::GameKind::Palworld => {
                self.server_type = hostable_core::ServerType::Palworld;
                self.version.clear();
                self.motd = "A Hostable Palworld server".to_string();
                self.modrinth = ModrinthDraft::default();
                hostable_core::GameKind::Palworld
            }
        };
        self.dry_run_reviewed = false;
        self.reset_cursor();
    }

    fn extra_json(&self) -> serde_json::Value {
        let mut extra = serde_json::json!({
            "motd": self.motd,
        });
        if let Some(modrinth) = self.modrinth.to_json() {
            extra["modrinth"] = modrinth;
        }
        extra
    }
}

impl ModrinthDraft {
    fn to_json(&self) -> Option<serde_json::Value> {
        if self.modpack.is_none() && self.mods.is_empty() {
            return None;
        }

        let mut mods = Vec::new();
        for selected in &self.mods {
            push_install_file(
                &mut mods,
                &selected.title,
                "selected",
                &selected.selection.file,
            );
            for dependency in &selected.selection.dependencies {
                push_install_file(
                    &mut mods,
                    &selected.title,
                    "required_dependency",
                    dependency,
                );
            }
        }

        let modpack = self.modpack.as_ref().map(|selected| {
            serde_json::json!({
                "title": selected.title,
                "project_id": selected.selection.project_id,
                "version_id": selected.selection.version_id,
                "version_number": selected.selection.version_number,
                "filename": selected.selection.file.filename,
                "download_url": selected.selection.file.url,
            })
        });

        Some(serde_json::json!({
            "source": "modrinth",
            "modpack": modpack,
            "modpack_url": self.modpack.as_ref().map(|selected| selected.selection.file.url.clone()),
            "mods": mods,
        }))
    }
}

fn push_install_file(
    mods: &mut Vec<serde_json::Value>,
    title: &str,
    install_type: &str,
    file: &hostable_core::ModrinthInstallFile,
) {
    if mods
        .iter()
        .any(|item| item.get("download_url").and_then(|value| value.as_str()) == Some(&file.url))
    {
        return;
    }
    mods.push(serde_json::json!({
        "title": title,
        "install_type": install_type,
        "project_id": file.project_id,
        "version_id": file.version_id,
        "filename": file.filename,
        "download_url": file.url,
        "sha512": file.sha512,
        "size": file.size,
    }));
}

impl Default for ChoicePicker {
    fn default() -> Self {
        Self {
            target: ChoiceTarget::CreateGame,
            query: String::new(),
            cursor: 0,
            selected: 0,
        }
    }
}

impl Default for ModrinthPicker {
    fn default() -> Self {
        Self {
            kind: ModrinthPickKind::Modpack,
            query: String::new(),
            cursor: 0,
            results: Vec::new(),
            selected: 0,
            dirty: true,
        }
    }
}

impl ModrinthPicker {
    fn cycle_kind(&mut self) {
        self.kind = match self.kind {
            ModrinthPickKind::Modpack => ModrinthPickKind::Mod,
            ModrinthPickKind::Mod => ModrinthPickKind::Modpack,
        };
        self.mark_dirty();
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.results.clear();
        self.selected = 0;
    }

    fn next_result(&mut self) {
        if !self.results.is_empty() {
            self.selected = (self.selected + 1) % self.results.len();
        }
    }

    fn previous_result(&mut self) {
        if self.results.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.results.len() - 1
        } else {
            self.selected - 1
        };
    }

    fn insert_char(&mut self, ch: char) {
        insert_at(&mut self.query, &mut self.cursor, ch);
        self.mark_dirty();
    }

    fn backspace(&mut self) {
        backspace_at(&mut self.query, &mut self.cursor);
        self.mark_dirty();
    }

    fn delete_char(&mut self) {
        delete_at(&mut self.query, &mut self.cursor);
        self.mark_dirty();
    }

    fn move_cursor_left(&mut self) {
        move_cursor_left(&self.query, &mut self.cursor);
    }

    fn move_cursor_right(&mut self) {
        move_cursor_right(&self.query, &mut self.cursor);
    }

    fn move_cursor_start(&mut self) {
        self.cursor = 0;
    }

    fn move_cursor_end(&mut self) {
        self.cursor = self.query.chars().count();
    }
}

impl ModrinthPickKind {
    fn as_query_value(self) -> &'static str {
        match self {
            ModrinthPickKind::Modpack => "modpack",
            ModrinthPickKind::Mod => "mod",
        }
    }
}

impl Default for ConnectDraft {
    fn default() -> Self {
        Self {
            field: ConnectField::Game,
            cursor: 0,
            game: hostable_core::GameKind::Minecraft,
            name: "friends".to_string(),
            path: "/srv/hostable/servers/friends".to_string(),
            dry_run_reviewed: false,
        }
    }
}

impl ConnectField {
    fn index(self) -> usize {
        match self {
            ConnectField::Game => 0,
            ConnectField::Name => 1,
            ConnectField::Path => 2,
        }
    }
}

impl ConnectDraft {
    fn next_field(&mut self) {
        self.field = match self.field {
            ConnectField::Game => ConnectField::Name,
            ConnectField::Name => ConnectField::Path,
            ConnectField::Path => ConnectField::Game,
        };
        self.reset_cursor();
    }

    fn previous_field(&mut self) {
        self.field = match self.field {
            ConnectField::Game => ConnectField::Path,
            ConnectField::Name => ConnectField::Game,
            ConnectField::Path => ConnectField::Name,
        };
        self.reset_cursor();
    }

    fn input(&self) -> Option<&String> {
        match self.field {
            ConnectField::Name => Some(&self.name),
            ConnectField::Path => Some(&self.path),
            ConnectField::Game => None,
        }
    }

    fn reset_cursor(&mut self) {
        self.cursor = self.input().map(|value| value.chars().count()).unwrap_or(0);
    }

    fn insert_char(&mut self, ch: char) {
        match self.field {
            ConnectField::Name => insert_at(&mut self.name, &mut self.cursor, ch),
            ConnectField::Path => insert_at(&mut self.path, &mut self.cursor, ch),
            ConnectField::Game => {}
        }
        self.dry_run_reviewed = false;
    }

    fn backspace(&mut self) {
        match self.field {
            ConnectField::Name => backspace_at(&mut self.name, &mut self.cursor),
            ConnectField::Path => backspace_at(&mut self.path, &mut self.cursor),
            ConnectField::Game => {}
        }
        self.dry_run_reviewed = false;
    }

    fn delete_char(&mut self) {
        match self.field {
            ConnectField::Name => delete_at(&mut self.name, &mut self.cursor),
            ConnectField::Path => delete_at(&mut self.path, &mut self.cursor),
            ConnectField::Game => {}
        }
        self.dry_run_reviewed = false;
    }

    fn move_cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_cursor_right(&mut self) {
        let len = self.input().map(|value| value.chars().count()).unwrap_or(0);
        self.cursor = self.cursor.saturating_add(1).min(len);
    }

    fn move_cursor_start(&mut self) {
        self.cursor = 0;
    }

    fn move_cursor_end(&mut self) {
        self.reset_cursor();
    }

    fn set_game(&mut self, game: hostable_core::GameKind) {
        self.game = game;
        self.dry_run_reviewed = false;
    }
}

impl Default for S3Draft {
    fn default() -> Self {
        Self {
            field: S3Field::LocalKeepLast,
            cursor: "5".chars().count(),
            local_keep_last: "5".to_string(),
            s3_keep_last: "5".to_string(),
            bucket: String::new(),
            region: "us-east-1".to_string(),
            endpoint_url: String::new(),
            prefix: "hostable".to_string(),
            access_key_env: "AWS_ACCESS_KEY_ID".to_string(),
            secret_key_env: "AWS_SECRET_ACCESS_KEY".to_string(),
        }
    }
}

impl ConfigEditor {
    fn load_selected_buffer(&mut self) {
        self.buffer = self
            .document
            .as_ref()
            .and_then(|document| document.files.get(self.selected_file))
            .map(|file| file.contents.clone())
            .unwrap_or_default();
    }

    fn next_file(&mut self) {
        let Some(document) = &self.document else {
            return;
        };
        if document.files.is_empty() {
            return;
        }
        self.selected_file = (self.selected_file + 1) % document.files.len();
        self.load_selected_buffer();
    }
}

impl S3Draft {
    fn from_config(config: hostable_core::S3Config) -> Self {
        Self {
            field: S3Field::LocalKeepLast,
            cursor: "5".chars().count(),
            local_keep_last: "5".to_string(),
            s3_keep_last: "5".to_string(),
            bucket: config.bucket,
            region: config.region.unwrap_or_default(),
            endpoint_url: config.endpoint_url.unwrap_or_default(),
            prefix: config.prefix,
            access_key_env: config.access_key_env,
            secret_key_env: config.secret_key_env,
        }
    }

    fn to_config(&self) -> Result<hostable_core::S3Config, String> {
        if self.bucket.trim().is_empty() {
            return Err("Bucket is required".to_string());
        }
        if self.access_key_env.trim().is_empty() || self.secret_key_env.trim().is_empty() {
            return Err("Credential env var names are required".to_string());
        }

        Ok(hostable_core::S3Config {
            bucket: self.bucket.trim().to_string(),
            region: non_empty(self.region.clone()),
            endpoint_url: non_empty(self.endpoint_url.clone()),
            prefix: self.prefix.trim().trim_matches('/').to_string(),
            access_key_env: self.access_key_env.trim().to_string(),
            secret_key_env: self.secret_key_env.trim().to_string(),
        })
    }

    fn to_backup_settings(&self) -> Result<BackupSettingsRequest, String> {
        let local_keep_last = self
            .local_keep_last
            .trim()
            .parse::<u32>()
            .map_err(|_| "Local backups to keep must be a number".to_string())?;
        if !(1..=100).contains(&local_keep_last) {
            return Err("Local backups to keep must be between 1 and 100".to_string());
        }
        let s3_keep_last = self
            .s3_keep_last
            .trim()
            .parse::<u32>()
            .map_err(|_| "S3 backups to keep must be a number".to_string())?;
        if !(1..=100).contains(&s3_keep_last) {
            return Err("S3 backups to keep must be between 1 and 100".to_string());
        }
        Ok(BackupSettingsRequest {
            local_keep_last,
            s3_keep_last,
        })
    }

    fn next_field(&mut self) {
        self.field = match self.field {
            S3Field::LocalKeepLast => S3Field::S3KeepLast,
            S3Field::S3KeepLast => S3Field::Bucket,
            S3Field::Bucket => S3Field::Region,
            S3Field::Region => S3Field::Endpoint,
            S3Field::Endpoint => S3Field::Prefix,
            S3Field::Prefix => S3Field::AccessKeyEnv,
            S3Field::AccessKeyEnv => S3Field::SecretKeyEnv,
            S3Field::SecretKeyEnv => S3Field::LocalKeepLast,
        };
        self.reset_cursor();
    }

    fn previous_field(&mut self) {
        self.field = match self.field {
            S3Field::LocalKeepLast => S3Field::SecretKeyEnv,
            S3Field::S3KeepLast => S3Field::LocalKeepLast,
            S3Field::Bucket => S3Field::S3KeepLast,
            S3Field::Region => S3Field::Bucket,
            S3Field::Endpoint => S3Field::Region,
            S3Field::Prefix => S3Field::Endpoint,
            S3Field::AccessKeyEnv => S3Field::Prefix,
            S3Field::SecretKeyEnv => S3Field::AccessKeyEnv,
        };
        self.reset_cursor();
    }

    fn input(&self) -> &String {
        match self.field {
            S3Field::LocalKeepLast => &self.local_keep_last,
            S3Field::S3KeepLast => &self.s3_keep_last,
            S3Field::Bucket => &self.bucket,
            S3Field::Region => &self.region,
            S3Field::Endpoint => &self.endpoint_url,
            S3Field::Prefix => &self.prefix,
            S3Field::AccessKeyEnv => &self.access_key_env,
            S3Field::SecretKeyEnv => &self.secret_key_env,
        }
    }

    fn reset_cursor(&mut self) {
        self.cursor = self.input().chars().count();
    }

    fn insert_char(&mut self, ch: char) {
        match self.field {
            S3Field::LocalKeepLast => insert_at(&mut self.local_keep_last, &mut self.cursor, ch),
            S3Field::S3KeepLast => insert_at(&mut self.s3_keep_last, &mut self.cursor, ch),
            S3Field::Bucket => insert_at(&mut self.bucket, &mut self.cursor, ch),
            S3Field::Region => insert_at(&mut self.region, &mut self.cursor, ch),
            S3Field::Endpoint => insert_at(&mut self.endpoint_url, &mut self.cursor, ch),
            S3Field::Prefix => insert_at(&mut self.prefix, &mut self.cursor, ch),
            S3Field::AccessKeyEnv => insert_at(&mut self.access_key_env, &mut self.cursor, ch),
            S3Field::SecretKeyEnv => insert_at(&mut self.secret_key_env, &mut self.cursor, ch),
        }
    }

    fn backspace(&mut self) {
        match self.field {
            S3Field::LocalKeepLast => backspace_at(&mut self.local_keep_last, &mut self.cursor),
            S3Field::S3KeepLast => backspace_at(&mut self.s3_keep_last, &mut self.cursor),
            S3Field::Bucket => backspace_at(&mut self.bucket, &mut self.cursor),
            S3Field::Region => backspace_at(&mut self.region, &mut self.cursor),
            S3Field::Endpoint => backspace_at(&mut self.endpoint_url, &mut self.cursor),
            S3Field::Prefix => backspace_at(&mut self.prefix, &mut self.cursor),
            S3Field::AccessKeyEnv => backspace_at(&mut self.access_key_env, &mut self.cursor),
            S3Field::SecretKeyEnv => backspace_at(&mut self.secret_key_env, &mut self.cursor),
        }
    }

    fn delete_char(&mut self) {
        match self.field {
            S3Field::LocalKeepLast => delete_at(&mut self.local_keep_last, &mut self.cursor),
            S3Field::S3KeepLast => delete_at(&mut self.s3_keep_last, &mut self.cursor),
            S3Field::Bucket => delete_at(&mut self.bucket, &mut self.cursor),
            S3Field::Region => delete_at(&mut self.region, &mut self.cursor),
            S3Field::Endpoint => delete_at(&mut self.endpoint_url, &mut self.cursor),
            S3Field::Prefix => delete_at(&mut self.prefix, &mut self.cursor),
            S3Field::AccessKeyEnv => delete_at(&mut self.access_key_env, &mut self.cursor),
            S3Field::SecretKeyEnv => delete_at(&mut self.secret_key_env, &mut self.cursor),
        }
    }

    fn move_cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_cursor_right(&mut self) {
        let len = self.input().chars().count();
        self.cursor = self.cursor.saturating_add(1).min(len);
    }

    fn move_cursor_start(&mut self) {
        self.cursor = 0;
    }

    fn move_cursor_end(&mut self) {
        self.reset_cursor();
    }
}

impl S3Field {
    fn index(self) -> usize {
        match self {
            S3Field::LocalKeepLast => 0,
            S3Field::S3KeepLast => 1,
            S3Field::Bucket => 2,
            S3Field::Region => 3,
            S3Field::Endpoint => 4,
            S3Field::Prefix => 5,
            S3Field::AccessKeyEnv => 6,
            S3Field::SecretKeyEnv => 7,
        }
    }
}

impl AdminWebDraft {
    fn public_ip_label(&self) -> &str {
        self.public_ip.as_deref().unwrap_or("unknown")
    }

    fn display_target(&self) -> String {
        if let Some(domain) = non_empty(self.domain.clone()) {
            format!("http://{domain}:8080")
        } else if let Some(ip) = &self.public_ip {
            format!("http://{ip}:8080")
        } else {
            "http://<vps-ip>:8080".to_string()
        }
    }
}

impl Default for AdminPasswordDraft {
    fn default() -> Self {
        Self {
            field: AdminPasswordField::NewPassword,
            cursor: 0,
            new_password: String::new(),
            confirm_password: String::new(),
        }
    }
}

impl AdminPasswordDraft {
    fn next_field(&mut self) {
        self.field = match self.field {
            AdminPasswordField::NewPassword => AdminPasswordField::ConfirmPassword,
            AdminPasswordField::ConfirmPassword => AdminPasswordField::NewPassword,
        };
        self.reset_cursor();
    }

    fn previous_field(&mut self) {
        self.next_field();
    }

    fn input(&self) -> &String {
        match self.field {
            AdminPasswordField::NewPassword => &self.new_password,
            AdminPasswordField::ConfirmPassword => &self.confirm_password,
        }
    }

    fn reset_cursor(&mut self) {
        self.cursor = self.input().chars().count();
    }

    fn insert_char(&mut self, ch: char) {
        match self.field {
            AdminPasswordField::NewPassword => {
                insert_at(&mut self.new_password, &mut self.cursor, ch)
            }
            AdminPasswordField::ConfirmPassword => {
                insert_at(&mut self.confirm_password, &mut self.cursor, ch)
            }
        }
    }

    fn backspace(&mut self) {
        match self.field {
            AdminPasswordField::NewPassword => {
                backspace_at(&mut self.new_password, &mut self.cursor)
            }
            AdminPasswordField::ConfirmPassword => {
                backspace_at(&mut self.confirm_password, &mut self.cursor)
            }
        }
    }

    fn delete_char(&mut self) {
        match self.field {
            AdminPasswordField::NewPassword => delete_at(&mut self.new_password, &mut self.cursor),
            AdminPasswordField::ConfirmPassword => {
                delete_at(&mut self.confirm_password, &mut self.cursor)
            }
        }
    }

    fn move_cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_cursor_right(&mut self) {
        let len = self.input().chars().count();
        self.cursor = self.cursor.saturating_add(1).min(len);
    }

    fn move_cursor_start(&mut self) {
        self.cursor = 0;
    }

    fn move_cursor_end(&mut self) {
        self.reset_cursor();
    }
}

#[derive(Debug, Deserialize)]
struct HostableSettings {
    s3: Option<hostable_core::S3Config>,
    #[serde(default)]
    backup: BackupSettingsResponse,
    #[serde(default)]
    admin_web: AdminWebSettingsResponse,
}

#[derive(Debug, Deserialize)]
struct BackupSettingsResponse {
    local_keep_last: u32,
    #[serde(default = "default_keep_last")]
    s3_keep_last: u32,
}

impl Default for BackupSettingsResponse {
    fn default() -> Self {
        Self {
            local_keep_last: default_keep_last(),
            s3_keep_last: default_keep_last(),
        }
    }
}

fn default_keep_last() -> u32 {
    5
}

#[derive(Debug, Default, Deserialize)]
struct AdminWebSettingsResponse {
    enabled: bool,
    domain: Option<String>,
}

#[derive(Debug, Serialize)]
struct AdminWebSettingsRequest {
    enabled: bool,
    domain: Option<String>,
}

#[derive(Debug, Serialize)]
struct AdminPasswordRequest {
    new_password: String,
}

#[derive(Debug, Serialize)]
struct BackupSettingsRequest {
    local_keep_last: u32,
    s3_keep_last: u32,
}

#[derive(Debug, Deserialize)]
struct PublicIpResponse {
    ip: String,
}

#[derive(Debug, Deserialize)]
struct ServiceStatusResponse {
    active: String,
}

#[derive(Debug, Serialize)]
struct CreateServerRequest {
    game: hostable_core::GameKind,
    input: hostable_core::CreateServerInput,
    dry_run: bool,
}

#[derive(Debug, Serialize)]
struct ImportServerRequest {
    game: hostable_core::GameKind,
    name: String,
    install_path: PathBuf,
    dry_run: bool,
}

struct FormViewport {
    header: Rect,
    rows: Vec<(usize, Rect)>,
    start: usize,
    end: usize,
    total: usize,
}

impl FormViewport {
    fn range_label(&self) -> String {
        if self.total == 0 || self.rows.is_empty() {
            "fields 0/0".to_string()
        } else {
            format!("fields {}-{} of {}", self.start + 1, self.end, self.total)
        }
    }
}

fn form_viewport(area: Rect, total_fields: usize, active_index: usize) -> FormViewport {
    let regions = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);
    let header = regions[0];
    let body = regions[1];
    if total_fields == 0 || body.height == 0 {
        return FormViewport {
            header,
            rows: Vec::new(),
            start: 0,
            end: 0,
            total: total_fields,
        };
    }

    let visible_count = if body.height < 3 {
        1
    } else {
        usize::from(body.height / 3)
    }
    .min(total_fields);
    let active_index = active_index.min(total_fields.saturating_sub(1));
    let start = active_index
        .saturating_sub(visible_count.saturating_sub(1))
        .min(total_fields.saturating_sub(visible_count));
    let end = start + visible_count;
    let body_bottom = body.y.saturating_add(body.height);
    let mut rows = Vec::with_capacity(visible_count);

    for field_index in start..end {
        let visible_index = field_index - start;
        let y = body
            .y
            .saturating_add((visible_index as u16).saturating_mul(3));
        let height = body_bottom.saturating_sub(y).min(3);
        if height == 0 {
            break;
        }
        rows.push((
            field_index,
            Rect {
                x: body.x,
                y,
                width: body.width,
                height,
            },
        ));
    }

    FormViewport {
        header,
        rows,
        start,
        end,
        total: total_fields,
    }
}

fn render_input_field(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &'static str,
    value: &str,
    active: bool,
    cursor: usize,
) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let (visible, cursor_col) = visible_input(value, cursor, inner_width);
    let field = Paragraph::new(visible)
        .style(if active {
            Style::default().fg(theme().text)
        } else {
            Style::default().fg(theme().muted)
        })
        .block(block(label, active));
    frame.render_widget(field, area);

    if active && area.width > 2 && area.height > 1 {
        frame.set_cursor_position(Position::new(area.x + 1 + cursor_col as u16, area.y + 1));
    }
}

fn render_secret_input_field(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &'static str,
    value: &str,
    active: bool,
    cursor: usize,
) {
    let masked = "*".repeat(value.chars().count());
    render_input_field(frame, area, label, &masked, active, cursor);
}

fn render_select_field(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &'static str,
    value: &str,
    active: bool,
    hint: &'static str,
) {
    let line = Line::from(vec![
        Span::styled(
            value.to_string(),
            if active {
                Style::default()
                    .fg(theme().text)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme().muted)
            },
        ),
        Span::styled(format!("  {hint}"), Style::default().fg(theme().muted)),
    ]);
    frame.render_widget(Paragraph::new(line).block(block(label, active)), area);
}

fn visible_input(value: &str, cursor: usize, width: usize) -> (String, usize) {
    if width == 0 {
        return (String::new(), 0);
    }
    let char_count = value.chars().count();
    let cursor = cursor.min(char_count);
    let start = cursor.saturating_sub(width.saturating_sub(1));
    let visible = value.chars().skip(start).take(width).collect::<String>();
    (visible, cursor.saturating_sub(start).min(width - 1))
}

fn insert_at(value: &mut String, cursor: &mut usize, ch: char) {
    let byte_index = byte_index(value, *cursor);
    value.insert(byte_index, ch);
    *cursor += 1;
}

fn backspace_at(value: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let remove_at = (*cursor).saturating_sub(1);
    let start = byte_index(value, remove_at);
    let end = byte_index(value, *cursor);
    value.replace_range(start..end, "");
    *cursor = remove_at;
}

fn delete_at(value: &mut String, cursor: &mut usize) {
    let char_count = value.chars().count();
    if *cursor >= char_count {
        return;
    }
    let start = byte_index(value, *cursor);
    let end = byte_index(value, *cursor + 1);
    value.replace_range(start..end, "");
}

fn move_cursor_left(value: &str, cursor: &mut usize) {
    *cursor = (*cursor).saturating_sub(1).min(value.chars().count());
}

fn move_cursor_right(value: &str, cursor: &mut usize) {
    *cursor = (*cursor).saturating_add(1).min(value.chars().count());
}

fn byte_index(value: &str, cursor: usize) -> usize {
    value
        .char_indices()
        .map(|(index, _)| index)
        .nth(cursor)
        .unwrap_or(value.len())
}

fn section_heading(label: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        label,
        Style::default()
            .fg(theme().muted)
            .add_modifier(Modifier::BOLD),
    ))
}

fn muted_kv(label: &'static str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), Style::default().fg(theme().muted)),
        Span::raw(value.into()),
    ])
}

fn game_label(game: hostable_core::GameKind) -> &'static str {
    match game {
        hostable_core::GameKind::Minecraft => "Minecraft",
        hostable_core::GameKind::ArkSurvivalEvolved => "ARK",
        hostable_core::GameKind::Palworld => "Palworld",
    }
}

fn server_type_label(server_type: hostable_core::ServerType) -> &'static str {
    match server_type {
        hostable_core::ServerType::Vanilla => "Vanilla",
        hostable_core::ServerType::Paper => "Paper",
        hostable_core::ServerType::Fabric => "Fabric",
        hostable_core::ServerType::ArkSurvivalEvolved => "ARK",
        hostable_core::ServerType::Palworld => "Palworld",
    }
}

/// Loader filter for a Modrinth search. Modpacks are limited to Fabric, the
/// loader Hostable can run them on.
fn picker_loader(modpack: bool, server_type: &hostable_core::ServerType) -> Option<&'static str> {
    if modpack {
        Some("fabric")
    } else {
        modrinth_loader(server_type)
    }
}

fn modrinth_loader(server_type: &hostable_core::ServerType) -> Option<&'static str> {
    match server_type {
        hostable_core::ServerType::Fabric => Some("fabric"),
        hostable_core::ServerType::Paper => Some("paper"),
        hostable_core::ServerType::Vanilla
        | hostable_core::ServerType::ArkSurvivalEvolved
        | hostable_core::ServerType::Palworld => None,
    }
}

fn protocol_label(protocol: hostable_core::Protocol) -> &'static str {
    match protocol {
        hostable_core::Protocol::Tcp => "tcp",
        hostable_core::Protocol::Udp => "udp",
    }
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn empty_label(value: &str) -> &str {
    if value.trim().is_empty() { "-" } else { value }
}

/// The daemon answers errors as `{"error": "..."}`. Show that, not "HTTP status client error".
fn error_text(response: reqwest::blocking::Response) -> String {
    let status = response.status();
    let body = response.text().unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|e| e.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("HTTP {status}"))
}

fn action_status(name: &str, ok_message: &str, warnings: &[String]) -> String {
    if warnings.is_empty() {
        format!("{name}: {ok_message}")
    } else {
        format!("{name}: {ok_message}, but {}", warnings.join("; "))
    }
}

/// Which messages are worth a toast, and how loudly. Pure navigation chatter is dropped.
fn classify_status(text: &str) -> Option<Level> {
    const QUIET: [&str; 14] = [
        "Opened ",
        "Returned to",
        "Connected to hostabled",
        "Settings loaded",
        "Logs loaded",
        "Config loaded",
        "Not connected",
        "Editing ",
        "Console command",
        "Connect existing folder",
        "Set web admin password",
        "Filter choices",
        "Create server:",
        "Selected ",
    ];
    if text.is_empty() || QUIET.iter().any(|prefix| text.starts_with(prefix)) {
        return None;
    }
    let lower = text.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|word| lower.contains(word));
    if has(&["failed", "unavailable", "error", "cannot", "refused"]) {
        Some(Level::Error)
    } else if has(&[
        "must",
        "required",
        "accept the",
        "no server",
        "no selected",
        "no choice",
        "no config",
        "set a",
        "type a",
        "is empty",
        "does not match",
        "could not",
        "needs",
        "but ",
    ]) {
        Some(Level::Warn)
    } else if has(&[
        "saved",
        "created",
        "connected",
        "finished",
        "started",
        "stopped",
        "restarted",
        "succeeded",
        "updated",
        "wrote",
        "added",
        "copied",
    ]) {
        Some(Level::Ok)
    } else if has(&["cancel", "closed", "switched"]) {
        None
    } else {
        Some(Level::Info)
    }
}

/// OSC 52 asks the terminal to put text on the clipboard. It works over SSH.
fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = text.as_bytes();
    let mut encoded = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        encoded.push(TABLE[(n >> 18) as usize & 63] as char);
        encoded.push(TABLE[(n >> 12) as usize & 63] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]52;c;{encoded}\x07");
    let _ = out.flush();
}
