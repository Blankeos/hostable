use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};
use reqwest::{
    blocking::{Client, RequestBuilder},
    header::HeaderValue,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Server,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    CreateServer,
    ConnectServer,
    EditS3,
    EditAdminWeb,
    ConsoleCommand,
    PickModrinth,
    EditConfig,
    ViewLogs,
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
    Modrinth,
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
enum ConnectField {
    Game,
    Name,
    Path,
}

#[derive(Debug, Clone)]
struct CreateDraft {
    game: hostable_core::GameKind,
    server_type: hostable_core::ServerType,
    field: CreateField,
    name: String,
    path: String,
    version: String,
    memory_mb: String,
    motd: String,
    modrinth: ModrinthDraft,
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
    results: Vec<hostable_core::ModrinthSearchHit>,
    selected: usize,
    dirty: bool,
}

#[derive(Debug, Clone)]
struct ConnectDraft {
    field: ConnectField,
    game: hostable_core::GameKind,
    name: String,
    path: String,
    dry_run_reviewed: bool,
}

#[derive(Debug, Clone)]
struct S3Draft {
    field: S3Field,
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
    public_ip: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ConfigEditor {
    server_id: String,
    document: Option<hostable_core::ConfigDocument>,
    selected_file: usize,
    buffer: String,
}

#[derive(Debug, Clone, Default)]
struct LogsPanel {
    server_id: String,
    service: String,
    text: String,
}

pub struct App {
    active_screen: Screen,
    mode: Mode,
    create: CreateDraft,
    connect: ConnectDraft,
    s3: S3Draft,
    admin_web: AdminWebDraft,
    modrinth_picker: ModrinthPicker,
    config_editor: ConfigEditor,
    logs_panel: LogsPanel,
    console_command: String,
    selected_server: usize,
    daemon_url: String,
    operator_token: Option<String>,
    client: Client,
    servers: Vec<hostable_core::ServerRecord>,
    server_statuses: HashMap<String, String>,
    status: String,
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
                "Not connected | no local token at {}",
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
            modrinth_picker: ModrinthPicker::default(),
            config_editor: ConfigEditor::default(),
            logs_panel: LogsPanel::default(),
            console_command: String::new(),
            selected_server: 0,
            daemon_url,
            operator_token,
            client: Client::new(),
            servers: Vec::new(),
            server_statuses: HashMap::new(),
            status,
        }
    }

    pub fn handle_key(&mut self, code: KeyCode) -> bool {
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
        if self.mode == Mode::ConsoleCommand {
            self.handle_console_key(code);
            return false;
        }
        if self.mode == Mode::PickModrinth {
            self.handle_modrinth_key(code);
            return false;
        }
        if self.mode == Mode::EditConfig {
            self.handle_config_key(code);
            return false;
        }
        if self.mode == Mode::ViewLogs {
            self.handle_logs_key(code);
            return false;
        }

        match code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
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
            KeyCode::Char('n') => self.open_create_server(),
            KeyCode::Char('c') => self.open_connect_server(),
            KeyCode::Char('s') => self.start_selected(),
            KeyCode::Char('x') => self.stop_selected(),
            KeyCode::Char('R') => self.restart_selected(),
            KeyCode::Char('b') => self.backup_selected(),
            KeyCode::Char('m') => self.open_console_command(),
            KeyCode::Char('f') => self.open_config_editor(),
            KeyCode::Char('L') => self.open_logs_panel(),
            _ => {}
        }
    }

    fn handle_settings_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('e') => self.open_s3_settings(),
            KeyCode::Char('t') => self.test_s3_settings(),
            KeyCode::Char('w') => self.toggle_admin_web(),
            KeyCode::Char('d') => self.open_admin_web_domain(),
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
        let url = format!("{}/api/servers", self.daemon_url.trim_end_matches('/'));
        let request = self.with_operator_token(self.client.get(url));
        match request
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json::<Vec<hostable_core::ServerRecord>>())
        {
            Ok(servers) => {
                self.status = format!("Connected to hostabled | {} server(s)", servers.len());
                self.servers = servers;
                if self.selected_server >= self.servers.len() {
                    self.selected_server = self.servers.len().saturating_sub(1);
                }
                self.refresh_server_statuses();
            }
            Err(error) => {
                self.status = format!("hostabled unavailable: {error}");
                self.servers.clear();
                self.server_statuses.clear();
                self.selected_server = 0;
            }
        }
    }

    fn refresh_server_statuses(&mut self) {
        self.server_statuses.clear();
        let server_ids = self
            .servers
            .iter()
            .map(|server| server.id.0.clone())
            .collect::<Vec<_>>();
        for server_id in server_ids {
            let url = format!(
                "{}/api/servers/{}/status",
                self.daemon_url.trim_end_matches('/'),
                server_id
            );
            let active = self
                .with_operator_token(self.client.get(url))
                .send()
                .and_then(|response| response.error_for_status())
                .and_then(|response| response.json::<ServiceStatusResponse>())
                .map(|response| response.active)
                .unwrap_or_else(|_| "unknown".to_string());
            self.server_statuses.insert(server_id, active);
        }
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
        if self.active_screen != Screen::Settings {
            return;
        }
        let url = format!(
            "{}/api/network/public-ip",
            self.daemon_url.trim_end_matches('/')
        );
        let request = self.with_operator_token(self.client.get(url));
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
        self.post_selected_server_action("start", "started");
        self.refresh_server_statuses();
    }

    pub fn stop_selected(&mut self) {
        self.post_selected_server_action("stop", "stopped");
        self.refresh_server_statuses();
    }

    pub fn restart_selected(&mut self) {
        self.post_selected_server_action("restart", "restart requested");
        self.refresh_server_statuses();
    }

    pub fn backup_selected(&mut self) {
        self.post_selected_server_action("backups/run", "backup finished");
    }

    pub fn open_create_server(&mut self) {
        self.active_screen = Screen::Server;
        self.mode = Mode::CreateServer;
        self.create = CreateDraft::default();
        self.status =
            "Create server: edit fields, Space accepts license, Enter reviews plan".to_string();
    }

    pub fn open_modrinth_picker(&mut self) {
        if self.create.game != hostable_core::GameKind::Minecraft {
            self.status = "Modrinth is only available for Minecraft".to_string();
            return;
        }
        self.mode = Mode::PickModrinth;
        self.modrinth_picker = ModrinthPicker::default();
        self.status = "Type a Modrinth search, Enter searches, Enter again selects".to_string();
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
        self.status = "Editing S3 settings".to_string();
    }

    pub fn open_admin_web_domain(&mut self) {
        if self.active_screen != Screen::Settings {
            return;
        }
        self.mode = Mode::EditAdminWeb;
        self.status = "Editing admin web domain".to_string();
    }

    pub fn toggle_admin_web(&mut self) {
        if self.active_screen != Screen::Settings {
            return;
        }
        self.admin_web.enabled = !self.admin_web.enabled;
        self.save_admin_web_settings();
    }

    pub fn open_console_command(&mut self) {
        if self.active_screen != Screen::Server || self.servers.is_empty() {
            return;
        }
        self.mode = Mode::ConsoleCommand;
        self.console_command.clear();
        self.status = "Console command".to_string();
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

    pub fn open_logs_panel(&mut self) {
        if self.active_screen != Screen::Server {
            return;
        }
        let Some(server) = self.servers.get(self.selected_server) else {
            self.status = "No selected server".to_string();
            return;
        };
        let url = format!(
            "{}/api/servers/{}/logs",
            self.daemon_url.trim_end_matches('/'),
            server.id.0
        );
        let request = self
            .with_operator_token(self.client.get(url))
            .query(&[("lines", "120")]);

        match request
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json::<LogsResponse>())
        {
            Ok(response) => {
                self.logs_panel = LogsPanel {
                    server_id: server.id.0.clone(),
                    service: response.service,
                    text: response.text,
                };
                self.mode = Mode::ViewLogs;
                self.status = "Logs loaded".to_string();
            }
            Err(error) => {
                self.status = format!("Failed to load logs: {error}");
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

    pub fn draw(&mut self, frame: &mut Frame<'_>) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(1)])
            .split(frame.area());

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(22),
                Constraint::Length(2),
                Constraint::Min(40),
            ])
            .split(rows[0]);

        self.render_sidebar(frame, columns[0]);
        match self.active_screen {
            Screen::Server => self.render_servers(frame, columns[2]),
            Screen::Settings => self.render_settings(frame, columns[2]),
        }
        self.render_footer(frame, rows[1]);
    }

    fn render_sidebar(&self, frame: &mut Frame<'_>, area: Rect) {
        let nav = Paragraph::new(Text::from(vec![
            Line::from(Span::styled(
                "hostable",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "VPS game servers",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            nav_line("1 Server", self.active_screen == Screen::Server),
            nav_line("2 Settings", self.active_screen == Screen::Settings),
            Line::from(""),
            Line::from(Span::styled(
                "tab changes section",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "q quits",
                Style::default().fg(Color::DarkGray),
            )),
        ]));
        frame.render_widget(nav, area);
    }

    fn render_servers(&self, frame: &mut Frame<'_>, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(8)])
            .split(area);

        self.render_server_toolbar(frame, rows[0]);

        if rows[1].width >= 92 {
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(32),
                    Constraint::Length(2),
                    Constraint::Min(32),
                    Constraint::Length(2),
                    Constraint::Length(25),
                ])
                .split(rows[1]);
            self.render_server_list(frame, columns[0]);
            self.render_server_workspace(frame, columns[2], false);
            self.render_server_actions(frame, columns[4]);
        } else {
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(30),
                    Constraint::Length(2),
                    Constraint::Min(30),
                ])
                .split(rows[1]);
            self.render_server_list(frame, columns[0]);
            self.render_server_workspace(frame, columns[2], true);
        }
    }

    fn render_server_workspace(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        show_actions_inline: bool,
    ) {
        if self.mode == Mode::CreateServer {
            self.render_create_server(frame, area);
        } else if self.mode == Mode::ConnectServer {
            self.render_connect_server(frame, area);
        } else if self.mode == Mode::PickModrinth {
            self.render_modrinth_picker(frame, area);
        } else if self.mode == Mode::ConsoleCommand {
            self.render_console_command(frame, area);
        } else if self.mode == Mode::EditConfig {
            self.render_config_editor(frame, area);
        } else if self.mode == Mode::ViewLogs {
            self.render_logs_panel(frame, area);
        } else {
            self.render_server_detail(frame, area, show_actions_inline);
        }
    }

    fn render_server_toolbar(&self, frame: &mut Frame<'_>, area: Rect) {
        let selected = self
            .servers
            .get(self.selected_server)
            .map(|server| server.name.as_str())
            .unwrap_or("none");

        let context = server_context_label(self.mode);
        let actions = if self.mode == Mode::Normal {
            action_spans(&[("n", "new"), ("c", "connect"), ("r", "refresh")])
        } else {
            task_action_spans(self.mode)
        };

        let toolbar = Paragraph::new(Text::from(vec![
            Line::from(vec![
                Span::styled(
                    "Server",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {} registered", self.servers.len()),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(format!("  {context}"), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(actions),
            Line::from(vec![
                Span::styled("selected", Style::default().fg(Color::DarkGray)),
                Span::raw(": "),
                Span::styled(selected.to_string(), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled("up/down", Style::default().fg(Color::Green)),
                Span::raw(" select"),
            ]),
        ]));
        frame.render_widget(toolbar, area);
    }

    fn render_server_list(&self, frame: &mut Frame<'_>, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(area);
        let heading = Paragraph::new(Text::from(vec![
            section_heading("Servers"),
            Line::from(Span::styled(
                "up/down to choose target",
                Style::default().fg(Color::DarkGray),
            )),
        ]));
        frame.render_widget(heading, rows[0]);

        let items = if self.servers.is_empty() {
            vec![ListItem::new(Text::from(vec![
                Line::from(Span::styled(
                    "No servers yet",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(Span::styled(
                    "n create   c connect",
                    Style::default().fg(Color::DarkGray),
                )),
            ]))]
        } else {
            self.servers
                .iter()
                .map(|server| {
                    let status = self.server_status(&server.id.0);
                    ListItem::new(Text::from(vec![
                        Line::from(vec![
                            Span::styled(
                                server.name.clone(),
                                Style::default().add_modifier(Modifier::BOLD),
                            ),
                            Span::raw("  "),
                            Span::styled(status_label(status), status_style(status)),
                        ]),
                        Line::from(Span::styled(
                            format!(
                                "{}  {}",
                                game_label(server.game),
                                server.install_path.display()
                            ),
                            Style::default().fg(Color::DarkGray),
                        )),
                    ]))
                })
                .collect()
        };

        let mut state = ListState::default();
        if !self.servers.is_empty() {
            state.select(Some(self.selected_server));
        }

        let list = List::new(items).highlight_symbol("> ").highlight_style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_stateful_widget(list, rows[1], &mut state);
    }

    fn render_server_detail(&self, frame: &mut Frame<'_>, area: Rect, show_actions_inline: bool) {
        let Some(server) = self.servers.get(self.selected_server) else {
            let empty = Paragraph::new(Text::from(vec![
                section_heading("Get started"),
                Line::from(""),
                Line::from(vec![
                    Span::styled("n", Style::default().fg(Color::Green)),
                    Span::raw(" new Minecraft or ARK server"),
                ]),
                Line::from(vec![
                    Span::styled("c", Style::default().fg(Color::Green)),
                    Span::raw(" connect an existing folder"),
                ]),
            ]))
            .wrap(Wrap { trim: true });
            frame.render_widget(empty, area);
            return;
        };

        let ports = server
            .ports
            .iter()
            .map(|port| format!("{}/{}", port.port, protocol_label(port.protocol)))
            .collect::<Vec<_>>()
            .join(", ");
        let join = server
            .ports
            .first()
            .map(|port| format!("<vps-ip>:{}", port.port))
            .unwrap_or_else(|| "No public port".to_string());
        let status = self.server_status(&server.id.0);

        let mut lines = vec![
            Line::from(Span::styled(
                server.name.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            section_heading("Overview"),
            status_kv("status", status),
            muted_kv("game", game_label(server.game).to_string()),
            muted_kv("folder", server.install_path.display().to_string()),
            muted_kv("service", server.service_name.clone()),
            muted_kv("ports", ports),
            Line::from(""),
            section_heading("Connect"),
            muted_kv("address", join),
        ];
        if show_actions_inline {
            lines.push(Line::from(""));
            lines.push(section_heading("Controls"));
            lines.push(Line::from(action_spans(&[
                ("s", "start"),
                ("x", "stop"),
                ("R", "restart"),
                ("b", "backup"),
                ("L", "logs"),
                ("m", "command"),
                ("f", "config"),
            ])));
        }
        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: true }),
            area,
        );
    }

    fn render_server_actions(&self, frame: &mut Frame<'_>, area: Rect) {
        let mut lines = vec![section_heading("Controls")];

        if self.mode != Mode::Normal {
            lines.push(section_heading("Current task"));
            lines.push(Line::from(Span::styled(
                server_context_label(self.mode),
                Style::default().fg(Color::White),
            )));
            lines.push(Line::from(task_action_spans(self.mode)));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Esc returns to the selected server controls.",
                Style::default().fg(Color::DarkGray),
            )));
            frame.render_widget(
                Paragraph::new(Text::from(lines)).wrap(Wrap { trim: true }),
                area,
            );
            return;
        }

        lines.push(Line::from(action_spans(&[("up/down", "select")])));
        lines.push(Line::from(action_spans(&[("n", "new"), ("c", "connect")])));
        lines.push(Line::from(action_spans(&[("r", "refresh")])));
        lines.push(Line::from(""));

        if self.servers.is_empty() {
            lines.push(Line::from(Span::styled(
                "No server selected yet.",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(section_heading("Selected server"));
            lines.push(Line::from(action_spans(&[("s", "start"), ("x", "stop")])));
            lines.push(Line::from(action_spans(&[
                ("R", "restart"),
                ("b", "backup"),
            ])));
            lines.push(Line::from(action_spans(&[("L", "logs"), ("m", "command")])));
            lines.push(Line::from(action_spans(&[("f", "config")])));
        }

        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: true }),
            area,
        );
    }

    fn render_create_server(&self, frame: &mut Frame<'_>, area: Rect) {
        let game = game_label(self.create.game);
        let server_type = server_type_label(self.create.server_type.clone());
        let license = if self.create.accepted_license {
            "accepted"
        } else {
            "required"
        };
        let submit = if self.create.dry_run_reviewed {
            "Enter install"
        } else {
            "Enter dry-run"
        };
        let modrinth = self.create.modrinth.summary();
        let text = format!(
            "Create server\n\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n\n{submit}\nEsc cancel",
            self.create.field_line(CreateField::Game, "Game", game),
            self.create
                .field_line(CreateField::Type, "Type", server_type),
            self.create
                .field_line(CreateField::Name, "Name", &self.create.name),
            self.create
                .field_line(CreateField::Path, "Folder", &self.create.path),
            self.create
                .field_line(CreateField::Version, "Version", &self.create.version),
            self.create
                .field_line(CreateField::Memory, "Memory MB", &self.create.memory_mb),
            self.create
                .field_line(CreateField::Motd, "MOTD", &self.create.motd),
            self.create
                .field_line(CreateField::Modrinth, "Modrinth", &modrinth),
            self.create
                .field_line(CreateField::License, "License", license),
        );
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), area);
    }

    fn render_modrinth_picker(&self, frame: &mut Frame<'_>, area: Rect) {
        let kind = match self.modrinth_picker.kind {
            ModrinthPickKind::Modpack => "Modpack",
            ModrinthPickKind::Mod => "Mod",
        };
        let loader = modrinth_loader(&self.create.server_type).unwrap_or("any");
        let mut lines = vec![
            "Modrinth picker".to_string(),
            String::new(),
            format!("Kind: {kind}  [tab] switch"),
            format!("Minecraft: {}  Loader: {loader}", self.create.version),
            format!("Search: {}", self.modrinth_picker.query),
            String::new(),
        ];
        if self.modrinth_picker.results.is_empty() {
            lines.push("Enter searches Modrinth.".to_string());
        } else {
            for (index, hit) in self.modrinth_picker.results.iter().take(8).enumerate() {
                let marker = if index == self.modrinth_picker.selected {
                    ">"
                } else {
                    " "
                };
                lines.push(format!(
                    "{marker} {}  {} downloads",
                    hit.title, hit.downloads
                ));
            }
        }
        lines.push(String::new());
        lines.push("Enter select   Esc back".to_string());
        frame.render_widget(
            Paragraph::new(lines.join("\n")).wrap(Wrap { trim: true }),
            area,
        );
    }

    fn render_connect_server(&self, frame: &mut Frame<'_>, area: Rect) {
        let game = game_label(self.connect.game);
        let submit = if self.connect.dry_run_reviewed {
            "Enter connect"
        } else {
            "Enter dry-run"
        };
        let text = format!(
            "Connect folder\n\n{}\n{}\n{}\n\n{submit}\nEsc cancel",
            self.connect.field_line(ConnectField::Game, "Game", game),
            self.connect
                .field_line(ConnectField::Name, "Name", &self.connect.name),
            self.connect
                .field_line(ConnectField::Path, "Folder", &self.connect.path),
        );
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), area);
    }

    fn render_console_command(&self, frame: &mut Frame<'_>, area: Rect) {
        let server_name = self
            .servers
            .get(self.selected_server)
            .map(|server| server.name.as_str())
            .unwrap_or("server");
        let text = format!(
            "Send command\n\n{server_name}\n\n> {}\n\nEnter send\nEsc cancel",
            self.console_command
        );
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), area);
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

    fn render_logs_panel(&self, frame: &mut Frame<'_>, area: Rect) {
        let server_name = self
            .servers
            .iter()
            .find(|server| server.id.0 == self.logs_panel.server_id)
            .map(|server| server.name.as_str())
            .unwrap_or("server");
        let logs = if self.logs_panel.text.trim().is_empty() {
            "No journal entries yet.".to_string()
        } else {
            self.logs_panel
                .text
                .lines()
                .rev()
                .take(32)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n")
        };
        let text = format!(
            "Logs\n{server_name}\n{}\n\n{}\n\nR refresh   Esc close",
            self.logs_panel.service, logs
        );
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), area);
    }

    fn render_settings(&self, frame: &mut Frame<'_>, area: Rect) {
        let text = if self.mode == Mode::EditS3 {
            Text::from(format!(
                "Backups\n\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n\nEnter save\nEsc cancel",
                self.s3.field_line(
                    S3Field::LocalKeepLast,
                    "Keep local",
                    &self.s3.local_keep_last
                ),
                self.s3
                    .field_line(S3Field::S3KeepLast, "Keep S3", &self.s3.s3_keep_last),
                self.s3
                    .field_line(S3Field::Bucket, "Bucket", &self.s3.bucket),
                self.s3
                    .field_line(S3Field::Region, "Region", &self.s3.region),
                self.s3
                    .field_line(S3Field::Endpoint, "Endpoint", &self.s3.endpoint_url),
                self.s3
                    .field_line(S3Field::Prefix, "Prefix", &self.s3.prefix),
                self.s3
                    .field_line(S3Field::AccessKeyEnv, "Access env", &self.s3.access_key_env),
                self.s3
                    .field_line(S3Field::SecretKeyEnv, "Secret env", &self.s3.secret_key_env),
            ))
        } else if self.mode == Mode::EditAdminWeb {
            Text::from(format!(
                "Admin web\n\nDomain\n  {}\n\nYou can also add a domain by typing it here. Use this VPS IP for the domain provider: {}\n\nEnter save\nEsc cancel",
                self.admin_web.domain,
                self.admin_web.public_ip_label(),
            ))
        } else {
            let admin_state = if self.admin_web.enabled { "on" } else { "off" };
            let admin_target = self.admin_web.display_target();
            Text::from(vec![
                Line::from(Span::styled(
                    "Settings",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                section_heading("S3 backups"),
                muted_kv("keep local", self.s3.local_keep_last.clone()),
                muted_kv("keep S3", self.s3.s3_keep_last.clone()),
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
                Line::from(action_spans(&[("w", "toggle"), ("d", "domain")])),
                Line::from(Span::styled(
                    "Use this IP at your domain provider.",
                    Style::default().fg(Color::DarkGray),
                )),
            ])
        };
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), area);
    }

    fn render_footer(&self, frame: &mut Frame<'_>, area: Rect) {
        let keys = if self.mode == Mode::CreateServer {
            "tab move  left/right adjust  enter review/install/open mods  esc cancel"
        } else if self.mode == Mode::ConnectServer {
            "tab move  left/right game  enter review/connect  esc cancel"
        } else if self.mode == Mode::EditS3 {
            "tab move  enter save  esc cancel"
        } else if self.mode == Mode::EditAdminWeb {
            "type domain  enter save  esc cancel"
        } else if self.mode == Mode::ConsoleCommand {
            "type command  enter send  esc cancel"
        } else if self.mode == Mode::PickModrinth {
            "type search  tab kind  enter search/select  esc back"
        } else if self.mode == Mode::EditConfig {
            "type raw config  f2 save  tab file  esc close"
        } else if self.mode == Mode::ViewLogs {
            "R refresh logs  esc close"
        } else if self.active_screen == Screen::Server {
            "tab/2 settings  up/down select  n new  c connect  s start  x stop  R restart  L logs  q quit"
        } else {
            "tab/1 server  e edit S3  t test S3  w admin web  d domain  q quit"
        };
        let footer = Paragraph::new(format!("{keys}  | {}", self.status))
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(footer, area);
    }

    fn post_selected_server_action(&mut self, action: &str, ok_message: &str) {
        if self.active_screen != Screen::Server {
            return;
        }
        let Some(server) = self.servers.get(self.selected_server) else {
            self.status = "No selected server".to_string();
            return;
        };
        let server_id = server.id.0.clone();
        let server_name = server.name.clone();
        let url = format!(
            "{}/api/servers/{}/{}",
            self.daemon_url.trim_end_matches('/'),
            server_id,
            action
        );
        let request = self.with_operator_token(self.client.post(url));

        match request
            .send()
            .and_then(|response| response.error_for_status())
        {
            Ok(_) => {
                self.status = format!("{}: {}", server_name, ok_message);
            }
            Err(error) => {
                self.status = format!("{} failed: {error}", server_name);
            }
        }
    }

    fn handle_create_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status = "Create server cancelled".to_string();
            }
            KeyCode::Tab | KeyCode::Down => self.create.next_field(),
            KeyCode::BackTab | KeyCode::Up => self.create.previous_field(),
            KeyCode::Left | KeyCode::Right => self.create.adjust_choice(),
            KeyCode::Char(' ') => {
                if self.create.field == CreateField::License {
                    self.create.toggle_license();
                } else if let Some(input) = self.create.input_mut() {
                    input.push(' ');
                    self.create.dry_run_reviewed = false;
                }
            }
            KeyCode::Backspace => {
                if let Some(input) = self.create.input_mut() {
                    input.pop();
                    self.create.dry_run_reviewed = false;
                }
            }
            KeyCode::Enter => {
                if self.create.field == CreateField::Modrinth {
                    self.open_modrinth_picker();
                } else {
                    self.submit_create_server();
                }
            }
            KeyCode::Char(ch) => {
                if let Some(input) = self.create.input_mut() {
                    input.push(ch);
                    self.create.dry_run_reviewed = false;
                }
            }
            _ => {}
        }
    }

    fn handle_modrinth_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::CreateServer;
                self.status = "Returned to create server".to_string();
            }
            KeyCode::Tab => {
                self.modrinth_picker.cycle_kind();
                self.status = "Switched Modrinth search kind".to_string();
            }
            KeyCode::Down => self.modrinth_picker.next_result(),
            KeyCode::Up => self.modrinth_picker.previous_result(),
            KeyCode::Backspace => {
                self.modrinth_picker.query.pop();
                self.modrinth_picker.mark_dirty();
            }
            KeyCode::Enter => {
                if self.modrinth_picker.dirty || self.modrinth_picker.results.is_empty() {
                    self.search_modrinth();
                } else {
                    self.select_modrinth_result();
                }
            }
            KeyCode::Char(ch) => {
                self.modrinth_picker.query.push(ch);
                self.modrinth_picker.mark_dirty();
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
            KeyCode::Left | KeyCode::Right => self.connect.cycle_game(),
            KeyCode::Backspace => {
                if let Some(input) = self.connect.input_mut() {
                    input.pop();
                    self.connect.dry_run_reviewed = false;
                }
            }
            KeyCode::Enter => self.submit_connect_server(),
            KeyCode::Char(ch) => {
                if let Some(input) = self.connect.input_mut() {
                    input.push(ch);
                    self.connect.dry_run_reviewed = false;
                }
            }
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
            KeyCode::Backspace => {
                self.s3.input_mut().pop();
            }
            KeyCode::Enter => self.save_s3_settings(),
            KeyCode::Char(ch) => {
                self.s3.input_mut().push(ch);
            }
            _ => {}
        }
    }

    fn handle_admin_web_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status = "Admin web edit cancelled".to_string();
            }
            KeyCode::Backspace => {
                self.admin_web.domain.pop();
            }
            KeyCode::Enter => self.save_admin_web_settings(),
            KeyCode::Char(ch) => {
                self.admin_web.domain.push(ch);
            }
            _ => {}
        }
    }

    fn search_modrinth(&mut self) {
        if self.modrinth_picker.query.trim().is_empty() {
            self.status = "Type a Modrinth search first".to_string();
            return;
        }

        let Some(game_version) = non_empty(self.create.version.clone()) else {
            self.status = "Set a Minecraft version before searching Modrinth".to_string();
            return;
        };
        let project_type = self.modrinth_picker.kind.as_query_value();
        let mut request = self
            .client
            .get(format!(
                "{}/api/modrinth/search",
                self.daemon_url.trim_end_matches('/')
            ))
            .query(&[
                ("query", self.modrinth_picker.query.trim()),
                ("project_type", project_type),
                ("game_version", game_version.as_str()),
            ]);
        if let Some(loader) = modrinth_loader(&self.create.server_type) {
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
                self.status = format!(
                    "Modrinth returned {} result(s)",
                    self.modrinth_picker.results.len()
                );
            }
            Err(error) => {
                self.status = format!("Modrinth search failed: {error}");
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
            self.status = "No Modrinth result selected".to_string();
            return;
        };
        let Some(game_version) = non_empty(self.create.version.clone()) else {
            self.status = "Set a Minecraft version before selecting Modrinth content".to_string();
            return;
        };
        let project_type = self.modrinth_picker.kind.as_query_value();
        let mut request = self
            .client
            .get(format!(
                "{}/api/modrinth/projects/{}/install",
                self.daemon_url.trim_end_matches('/'),
                hit.project_id
            ))
            .query(&[
                ("project_type", project_type),
                ("game_version", game_version.as_str()),
            ]);
        if let Some(loader) = modrinth_loader(&self.create.server_type) {
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
                    title: hit.title,
                    selection,
                };
                match self.modrinth_picker.kind {
                    ModrinthPickKind::Modpack => self.create.modrinth.modpack = Some(selected),
                    ModrinthPickKind::Mod => self.create.modrinth.mods.push(selected),
                }
                self.create.dry_run_reviewed = false;
                self.mode = Mode::CreateServer;
                self.status = "Added Modrinth selection".to_string();
            }
            Err(error) => {
                self.status = format!("Modrinth selection failed: {error}");
            }
        }
    }

    fn handle_console_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status = "Console command cancelled".to_string();
            }
            KeyCode::Backspace => {
                self.console_command.pop();
            }
            KeyCode::Enter => self.send_console_command(),
            KeyCode::Char(ch) => {
                self.console_command.push(ch);
            }
            _ => {}
        }
    }

    fn handle_logs_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status = "Closed logs".to_string();
            }
            KeyCode::Char('R') | KeyCode::Char('r') => self.open_logs_panel(),
            _ => {}
        }
    }

    fn send_console_command(&mut self) {
        let Some(server) = self.servers.get(self.selected_server) else {
            self.status = "No selected server".to_string();
            return;
        };
        if self.console_command.trim().is_empty() {
            self.status = "Console command is empty".to_string();
            return;
        }

        let request = ConsoleCommandRequest {
            command: self.console_command.trim().to_string(),
        };
        let url = format!(
            "{}/api/servers/{}/console",
            self.daemon_url.trim_end_matches('/'),
            server.id.0
        );
        let http = self.with_operator_token(self.client.post(url).json(&request));

        match http.send().and_then(|response| response.error_for_status()) {
            Ok(response) => {
                let result = response.json::<ConsoleCommandResponse>();
                self.mode = Mode::Normal;
                self.status = match result {
                    Ok(result) if result.message.trim().is_empty() => {
                        "Console command sent".to_string()
                    }
                    Ok(result) => format!("Console: {}", result.message.trim()),
                    Err(_) => "Console command sent".to_string(),
                };
            }
            Err(error) => {
                self.status = format!("Console command failed: {error}");
            }
        }
    }

    fn handle_config_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
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

    fn submit_create_server(&mut self) {
        if !self.create.accepted_license {
            self.status = "Accept the required game license before review".to_string();
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

        match http.send().and_then(|response| response.error_for_status()) {
            Ok(_) if dry_run => {
                self.create.dry_run_reviewed = true;
                self.status = "Dry-run succeeded. Press Enter again to install.".to_string();
            }
            Ok(_) => {
                self.mode = Mode::Normal;
                self.status = "Server created".to_string();
                self.refresh_servers();
            }
            Err(error) => {
                self.status = format!("Create server failed: {error}");
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
            name: "friends".to_string(),
            path: "/srv/hostable/servers/friends".to_string(),
            version: "1.20.4".to_string(),
            memory_mb: "4096".to_string(),
            motd: "A Hostable Minecraft server".to_string(),
            modrinth: ModrinthDraft::default(),
            accepted_license: false,
            dry_run_reviewed: false,
        }
    }
}

impl CreateDraft {
    fn next_field(&mut self) {
        self.field = match self.field {
            CreateField::Game => CreateField::Type,
            CreateField::Type => CreateField::Name,
            CreateField::Name => CreateField::Path,
            CreateField::Path => CreateField::Version,
            CreateField::Version => CreateField::Memory,
            CreateField::Memory => CreateField::Motd,
            CreateField::Motd => CreateField::Modrinth,
            CreateField::Modrinth => CreateField::License,
            CreateField::License => CreateField::Game,
        };
    }

    fn previous_field(&mut self) {
        self.field = match self.field {
            CreateField::Game => CreateField::License,
            CreateField::Type => CreateField::Game,
            CreateField::Name => CreateField::Type,
            CreateField::Path => CreateField::Name,
            CreateField::Version => CreateField::Path,
            CreateField::Memory => CreateField::Version,
            CreateField::Motd => CreateField::Memory,
            CreateField::Modrinth => CreateField::Motd,
            CreateField::License => CreateField::Modrinth,
        };
    }

    fn input_mut(&mut self) -> Option<&mut String> {
        match self.field {
            CreateField::Name => Some(&mut self.name),
            CreateField::Path => Some(&mut self.path),
            CreateField::Version => Some(&mut self.version),
            CreateField::Memory => Some(&mut self.memory_mb),
            CreateField::Motd => Some(&mut self.motd),
            CreateField::Game
            | CreateField::Type
            | CreateField::Modrinth
            | CreateField::License => None,
        }
    }

    fn adjust_choice(&mut self) {
        match self.field {
            CreateField::Game => self.cycle_game(),
            CreateField::Type => self.cycle_type(),
            CreateField::Modrinth => {}
            CreateField::License => self.toggle_license(),
            _ => {}
        }
    }

    fn toggle_license(&mut self) {
        self.accepted_license = !self.accepted_license;
        self.dry_run_reviewed = false;
    }

    fn cycle_game(&mut self) {
        self.game = match self.game {
            hostable_core::GameKind::Minecraft => {
                self.server_type = hostable_core::ServerType::ArkSurvivalEvolved;
                self.version.clear();
                self.motd = "A Hostable ARK server".to_string();
                self.modrinth = ModrinthDraft::default();
                hostable_core::GameKind::ArkSurvivalEvolved
            }
            hostable_core::GameKind::ArkSurvivalEvolved => {
                self.server_type = hostable_core::ServerType::Fabric;
                self.version = "1.20.4".to_string();
                self.motd = "A Hostable Minecraft server".to_string();
                hostable_core::GameKind::Minecraft
            }
        };
        self.dry_run_reviewed = false;
    }

    fn cycle_type(&mut self) {
        if self.game == hostable_core::GameKind::ArkSurvivalEvolved {
            self.server_type = hostable_core::ServerType::ArkSurvivalEvolved;
            return;
        }

        self.server_type = match self.server_type {
            hostable_core::ServerType::Vanilla => hostable_core::ServerType::Paper,
            hostable_core::ServerType::Paper => hostable_core::ServerType::Fabric,
            hostable_core::ServerType::Fabric | hostable_core::ServerType::ArkSurvivalEvolved => {
                hostable_core::ServerType::Vanilla
            }
        };
        self.dry_run_reviewed = false;
    }

    fn field_line(&self, field: CreateField, label: &str, value: &str) -> String {
        let marker = if self.field == field { ">" } else { " " };
        format!("{marker} {label}: {value}")
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
    fn summary(&self) -> String {
        match (&self.modpack, self.mods.len()) {
            (None, 0) => "none".to_string(),
            (Some(modpack), 0) => format!("pack: {}", modpack.title),
            (None, mods) => format!("{mods} mod(s)"),
            (Some(modpack), mods) => format!("pack: {} + {mods} mod(s)", modpack.title),
        }
    }

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

impl Default for ModrinthPicker {
    fn default() -> Self {
        Self {
            kind: ModrinthPickKind::Modpack,
            query: String::new(),
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
            game: hostable_core::GameKind::Minecraft,
            name: "friends".to_string(),
            path: "/srv/hostable/servers/friends".to_string(),
            dry_run_reviewed: false,
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
    }

    fn previous_field(&mut self) {
        self.field = match self.field {
            ConnectField::Game => ConnectField::Path,
            ConnectField::Name => ConnectField::Game,
            ConnectField::Path => ConnectField::Name,
        };
    }

    fn input_mut(&mut self) -> Option<&mut String> {
        match self.field {
            ConnectField::Name => Some(&mut self.name),
            ConnectField::Path => Some(&mut self.path),
            ConnectField::Game => None,
        }
    }

    fn cycle_game(&mut self) {
        self.game = match self.game {
            hostable_core::GameKind::Minecraft => hostable_core::GameKind::ArkSurvivalEvolved,
            hostable_core::GameKind::ArkSurvivalEvolved => hostable_core::GameKind::Minecraft,
        };
        self.dry_run_reviewed = false;
    }

    fn field_line(&self, field: ConnectField, label: &str, value: &str) -> String {
        let marker = if self.field == field { ">" } else { " " };
        format!("{marker} {label}: {value}")
    }
}

impl Default for S3Draft {
    fn default() -> Self {
        Self {
            field: S3Field::LocalKeepLast,
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
            .map_err(|_| "Keep local must be a number".to_string())?;
        if !(1..=100).contains(&local_keep_last) {
            return Err("Keep local must be between 1 and 100".to_string());
        }
        let s3_keep_last = self
            .s3_keep_last
            .trim()
            .parse::<u32>()
            .map_err(|_| "Keep S3 must be a number".to_string())?;
        if !(1..=100).contains(&s3_keep_last) {
            return Err("Keep S3 must be between 1 and 100".to_string());
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
    }

    fn input_mut(&mut self) -> &mut String {
        match self.field {
            S3Field::LocalKeepLast => &mut self.local_keep_last,
            S3Field::S3KeepLast => &mut self.s3_keep_last,
            S3Field::Bucket => &mut self.bucket,
            S3Field::Region => &mut self.region,
            S3Field::Endpoint => &mut self.endpoint_url,
            S3Field::Prefix => &mut self.prefix,
            S3Field::AccessKeyEnv => &mut self.access_key_env,
            S3Field::SecretKeyEnv => &mut self.secret_key_env,
        }
    }

    fn field_line(&self, field: S3Field, label: &str, value: &str) -> String {
        let marker = if self.field == field { ">" } else { " " };
        format!("{marker} {label}: {value}")
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

#[derive(Debug, Deserialize)]
struct LogsResponse {
    service: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct CreateServerRequest {
    game: hostable_core::GameKind,
    input: hostable_core::CreateServerInput,
    dry_run: bool,
}

#[derive(Debug, Serialize)]
struct ConsoleCommandRequest {
    command: String,
}

#[derive(Debug, Serialize)]
struct ImportServerRequest {
    game: hostable_core::GameKind,
    name: String,
    install_path: PathBuf,
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
struct ConsoleCommandResponse {
    message: String,
}

fn nav_line(label: &'static str, active: bool) -> Line<'static> {
    if active {
        Line::from(vec![
            Span::styled(
                ">",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                label,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(" ", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(label, Style::default().fg(Color::DarkGray)),
        ])
    }
}

fn push_hint(spans: &mut Vec<Span<'static>>, key: &'static str, label: &'static str) {
    if !spans.is_empty() {
        spans.push(Span::raw("   "));
    }
    spans.push(Span::styled(
        key,
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(label, Style::default().fg(Color::DarkGray)));
}

fn action_spans(actions: &[(&'static str, &'static str)]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (key, label) in actions {
        push_hint(&mut spans, key, label);
    }
    spans
}

fn task_action_spans(mode: Mode) -> Vec<Span<'static>> {
    match mode {
        Mode::Normal => action_spans(&[("up/down", "select")]),
        Mode::CreateServer => action_spans(&[("tab", "field"), ("enter", "review/install")]),
        Mode::ConnectServer => action_spans(&[("tab", "field"), ("enter", "review/connect")]),
        Mode::EditS3 => action_spans(&[("tab", "field"), ("enter", "save")]),
        Mode::EditAdminWeb => action_spans(&[("enter", "save")]),
        Mode::ConsoleCommand => action_spans(&[("enter", "send")]),
        Mode::PickModrinth => action_spans(&[("tab", "kind"), ("enter", "search/select")]),
        Mode::EditConfig => action_spans(&[("tab", "file"), ("F2", "save")]),
        Mode::ViewLogs => action_spans(&[("R", "refresh")]),
    }
}

fn section_heading(label: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        label,
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    ))
}

fn muted_kv(label: &'static str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), Style::default().fg(Color::DarkGray)),
        Span::raw(value.into()),
    ])
}

fn status_kv(label: &'static str, status: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), Style::default().fg(Color::DarkGray)),
        Span::styled(status_label(status), status_style(status)),
    ])
}

fn status_label(status: &str) -> &'static str {
    match status {
        "active" => "online",
        "inactive" => "stopped",
        "activating" => "starting",
        "deactivating" => "stopping",
        "failed" => "failed",
        _ => "unknown",
    }
}

fn status_style(status: &str) -> Style {
    match status {
        "active" => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        "activating" | "deactivating" => Style::default().fg(Color::Yellow),
        "failed" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        "inactive" => Style::default().fg(Color::DarkGray),
        _ => Style::default().fg(Color::DarkGray),
    }
}

fn server_context_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "manage selected server",
        Mode::CreateServer => "creating server",
        Mode::ConnectServer => "connecting folder",
        Mode::EditS3 | Mode::EditAdminWeb => "editing settings",
        Mode::ConsoleCommand => "sending command",
        Mode::PickModrinth => "choosing Modrinth content",
        Mode::EditConfig => "editing config",
        Mode::ViewLogs => "viewing logs",
    }
}

fn game_label(game: hostable_core::GameKind) -> &'static str {
    match game {
        hostable_core::GameKind::Minecraft => "Minecraft",
        hostable_core::GameKind::ArkSurvivalEvolved => "ARK",
    }
}

fn server_type_label(server_type: hostable_core::ServerType) -> &'static str {
    match server_type {
        hostable_core::ServerType::Vanilla => "Vanilla",
        hostable_core::ServerType::Paper => "Paper",
        hostable_core::ServerType::Fabric => "Fabric",
        hostable_core::ServerType::ArkSurvivalEvolved => "ARK",
    }
}

fn modrinth_loader(server_type: &hostable_core::ServerType) -> Option<&'static str> {
    match server_type {
        hostable_core::ServerType::Fabric => Some("fabric"),
        hostable_core::ServerType::Paper => Some("paper"),
        hostable_core::ServerType::Vanilla | hostable_core::ServerType::ArkSurvivalEvolved => None,
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
