//! The per-server workspace: a live console, players (whitelist and ops) and
//! mods (Modrinth search and install). Opened from the server detail screen.

use super::link;
use super::ui::centered;
use super::{
    App, Mode, backspace_at, delete_at, error_text, insert_at, move_cursor_left, move_cursor_right,
    render_input_field,
};
use crate::theme::{Health, Level, status_badge, theme};
use crossterm::event::KeyCode;
use hostable_core::GameKind;
use hostable_core::minecraft_admin::{InstalledMod, ModpackInfo, OnlinePlayers, PlayerEntry};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use serde::{Deserialize, de::DeserializeOwned};
use std::time::{Duration, Instant};

const MAX_CONSOLE_LINES: usize = 2000;
const CONSOLE_POLL: Duration = Duration::from_secs(1);
const PLAYERS_POLL: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ManageTab {
    #[default]
    Console,
    Players,
    Mods,
    Config,
}

impl ManageTab {
    fn hint(self) -> &'static str {
        match self {
            ManageTab::Console => "console",
            ManageTab::Players => "players",
            ManageTab::Mods => "mods",
            ManageTab::Config => "config",
        }
    }

    fn label(self) -> &'static str {
        match self {
            ManageTab::Console => "Console",
            ManageTab::Players => "Players",
            ManageTab::Mods => "Mods",
            ManageTab::Config => "Config",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineKind {
    Log,
    Warn,
    Error,
    Join,
    Command,
    Reply,
    Note,
}

#[derive(Debug, Clone)]
pub(super) struct ConsoleLine {
    pub kind: LineKind,
    pub text: String,
}

#[derive(Debug, Default)]
pub(super) struct ConsoleView {
    pub lines: Vec<ConsoleLine>,
    pub cursor: Option<String>,
    pub loaded: bool,
    pub input: String,
    pub input_cursor: usize,
    pub history: Vec<String>,
    pub history_at: Option<usize>,
    /// Visual lines scrolled up from the bottom. 0 follows new output.
    pub scroll: usize,
    pub last_poll: Option<Instant>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct PlayersData {
    pub online: Option<OnlinePlayers>,
    pub whitelist_enabled: bool,
    pub whitelist: Vec<PlayerEntry>,
    pub ops: Vec<PlayerEntry>,
    #[serde(default = "yes")]
    pub online_mode: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum PlayerColumn {
    #[default]
    Online,
    Whitelist,
    Ops,
}

impl PlayerColumn {
    fn index(self) -> usize {
        match self {
            PlayerColumn::Online => 0,
            PlayerColumn::Whitelist => 1,
            PlayerColumn::Ops => 2,
        }
    }

    fn from_index(index: usize) -> Self {
        match index {
            0 => PlayerColumn::Online,
            1 => PlayerColumn::Whitelist,
            _ => PlayerColumn::Ops,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct PlayersView {
    pub data: Option<PlayersData>,
    pub error: Option<String>,
    pub column: PlayerColumn,
    pub selected: [usize; 3],
    /// Name being typed to add to the whitelist or ops list.
    pub add: Option<(PlayerColumn, String, usize)>,
    pub last_poll: Option<Instant>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ModsData {
    pub kind: Option<String>,
    pub server_type: String,
    pub game_version: Option<String>,
    pub loader: Option<String>,
    pub items: Vec<InstalledMod>,
    #[serde(default)]
    pub modpack: Option<ModpackInfo>,
}

/// Which mods the list shows when the server was built from a modpack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ModScope {
    #[default]
    All,
    Pack,
    Added,
}

impl ModScope {
    fn next(self) -> Self {
        match self {
            ModScope::All => ModScope::Pack,
            ModScope::Pack => ModScope::Added,
            ModScope::Added => ModScope::All,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ModSearch {
    pub input: String,
    pub cursor: usize,
    pub results: Vec<hostable_core::ModrinthSearchHit>,
    pub selected: usize,
    /// Searching modpacks instead of single mods.
    pub modpacks: bool,
    /// The query changed since the results were fetched.
    pub dirty: bool,
}

#[derive(Debug, Default)]
pub(super) struct ModsView {
    pub data: Option<ModsData>,
    pub error: Option<String>,
    pub selected: usize,
    pub filter: String,
    pub filtering: bool,
    pub scope: ModScope,
    pub search: Option<ModSearch>,
    pub pending_remove: Option<String>,
    pub restart_needed: bool,
}

/// A config file worth opening, as the daemon found it on disk.
#[derive(Debug, Clone)]
pub(super) struct ConfigEntry {
    pub path: std::path::PathBuf,
    pub name: String,
    pub about: &'static str,
}

#[derive(Debug, Default)]
pub(super) struct ConfigView {
    pub files: Vec<ConfigEntry>,
    pub loaded: bool,
    pub error: Option<String>,
    pub selected: usize,
    pub restart_needed: bool,
}

/// What a file is for, so the list reads without opening anything.
fn config_about(name: &str) -> &'static str {
    match name {
        "server.properties" => "Name, difficulty, port, view distance, whitelist",
        "start.sh" => "Launch command and memory (-Xmx)",
        "eula.txt" => "Minecraft EULA",
        "ops.json" => "Operators",
        "whitelist.json" => "Allowed players",
        "banned-players.json" => "Banned players",
        "banned-ips.json" => "Banned IP addresses",
        "PalWorldSettings.ini" => "Rates, difficulty, player limit, passwords",
        "GameUserSettings.ini" => "Map, ports, passwords, player limit",
        "Game.ini" => "Rates, breeding, taming, per-level overrides",
        _ => "",
    }
}

#[derive(Debug, Default)]
pub(super) struct Manage {
    pub server_id: String,
    pub server_name: String,
    pub game: Option<GameKind>,
    pub tab: ManageTab,
    pub console: ConsoleView,
    pub players: PlayersView,
    pub mods: ModsView,
    pub config: ConfigView,
}

#[derive(Debug, Deserialize)]
struct LogChunk {
    text: String,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConsoleReply {
    message: String,
}

#[derive(Debug, Deserialize)]
struct ActionReply {
    message: String,
}

#[derive(Debug, Deserialize)]
struct InstallReply {
    installed: Vec<String>,
    skipped: Vec<String>,
}

impl Manage {
    fn tabs(&self) -> Vec<ManageTab> {
        if self.game == Some(GameKind::Minecraft) {
            vec![
                ManageTab::Console,
                ManageTab::Players,
                ManageTab::Mods,
                ManageTab::Config,
            ]
        } else {
            vec![ManageTab::Console, ManageTab::Config]
        }
    }

    fn push(&mut self, kind: LineKind, text: impl Into<String>) {
        let lines = &mut self.console.lines;
        for line in text.into().lines() {
            lines.push(ConsoleLine {
                kind,
                text: line.to_string(),
            });
        }
        if lines.len() > MAX_CONSOLE_LINES {
            lines.drain(0..lines.len() - MAX_CONSOLE_LINES);
        }
    }

    fn visible_mods(&self) -> Vec<&InstalledMod> {
        let Some(data) = &self.mods.data else {
            return Vec::new();
        };
        let needle = self.mods.filter.to_lowercase();
        data.items
            .iter()
            .filter(|item| match self.mods.scope {
                ModScope::All => true,
                ModScope::Pack => item.in_pack,
                ModScope::Added => !item.in_pack,
            })
            .filter(|item| {
                needle.is_empty()
                    || item.filename.to_lowercase().contains(&needle)
                    || item
                        .title
                        .as_deref()
                        .is_some_and(|title| title.to_lowercase().contains(&needle))
            })
            .collect()
    }

    /// Names shown in a players column, in display order.
    fn column_names(&self, column: PlayerColumn) -> Vec<String> {
        let Some(data) = &self.players.data else {
            return Vec::new();
        };
        match column {
            PlayerColumn::Online => data
                .online
                .as_ref()
                .map(|online| online.names.clone())
                .unwrap_or_default(),
            PlayerColumn::Whitelist => data.whitelist.iter().map(|p| p.name.clone()).collect(),
            PlayerColumn::Ops => data.ops.iter().map(|p| p.name.clone()).collect(),
        }
    }

    fn selected_name(&self) -> Option<String> {
        let names = self.column_names(self.players.column);
        names
            .get(self.players.selected[self.players.column.index()])
            .cloned()
    }
}

/// `2026-11-04T10:00:00+0000 host unit[12]: [10:00:00] [Server thread/INFO]: text`
/// becomes `10:00:00  text` with a kind taken from the log level.
pub(super) fn parse_log_line(raw: &str) -> ConsoleLine {
    let body = raw
        .split_once("]: ")
        .filter(|(prefix, _)| prefix.contains('['))
        .map(|(_, rest)| rest)
        .unwrap_or(raw);
    let mut kind = LineKind::Log;
    let mut text = body.to_string();
    if let Some(rest) = body.strip_prefix('[')
        && let Some((time, after)) = rest.split_once("] [")
        && let Some((level, message)) = after.split_once("]: ")
    {
        let level = level.rsplit('/').next().unwrap_or("");
        kind = match level {
            "WARN" => LineKind::Warn,
            "ERROR" | "FATAL" => LineKind::Error,
            _ => LineKind::Log,
        };
        text = format!("{time}  {message}");
    }
    if text.contains(" joined the game") || text.contains(" left the game") {
        kind = LineKind::Join;
    }
    if text.starts_with('\t') || text.starts_with("Caused by") {
        kind = LineKind::Error;
    }
    ConsoleLine { kind, text }
}

/// Break a line into rows of at most `width` characters.
fn wrap_rows(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let text = text.replace('\t', "    ");
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars
        .chunks(width)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

fn valid_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

impl App {
    fn api_get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
        seconds: u64,
    ) -> Result<T, String> {
        let url = format!("{}{}", self.daemon_url.trim_end_matches('/'), path);
        let response = self
            .with_operator_token(self.client.get(url).query(query))
            .timeout(Duration::from_secs(seconds))
            .send()
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(error_text(response));
        }
        response.json::<T>().map_err(|error| error.to_string())
    }

    fn api_post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
        seconds: u64,
    ) -> Result<T, String> {
        let url = format!("{}{}", self.daemon_url.trim_end_matches('/'), path);
        let response = self
            .with_operator_token(self.client.post(url).json(body))
            .timeout(Duration::from_secs(seconds))
            .send()
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(error_text(response));
        }
        response.json::<T>().map_err(|error| error.to_string())
    }

    pub(super) fn open_manage(&mut self, tab: ManageTab) {
        if self.active_screen != super::Screen::Server {
            return;
        }
        let Some(server) = self.servers.get(self.selected_server) else {
            self.status = "No server selected".to_string();
            return;
        };
        let same = self.manage.server_id == server.id.0;
        let game = server.game;
        if !same {
            self.manage = Manage {
                server_id: server.id.0.clone(),
                server_name: server.name.clone(),
                game: Some(game),
                ..Manage::default()
            };
        }
        if game != GameKind::Minecraft && !matches!(tab, ManageTab::Console | ManageTab::Config) {
            self.status = "Players and mods are only available for Minecraft".to_string();
            return;
        }
        self.manage.tab = tab;
        self.mode = Mode::Manage;
        match tab {
            ManageTab::Console => self.poll_console(),
            ManageTab::Players => self.refresh_players(),
            ManageTab::Mods => self.refresh_mods(),
            ManageTab::Config => self.refresh_config(),
        }
    }

    fn switch_tab(&mut self, forward: bool) {
        let tabs = self.manage.tabs();
        let at = tabs
            .iter()
            .position(|tab| *tab == self.manage.tab)
            .unwrap_or(0);
        let next = if forward {
            (at + 1) % tabs.len()
        } else {
            (at + tabs.len() - 1) % tabs.len()
        };
        if tabs[next] != self.manage.tab {
            self.open_manage(tabs[next]);
        }
    }

    pub(super) fn manage_tick(&mut self) {
        if self.last_status_poll.elapsed() >= Duration::from_secs(5) {
            self.refresh_server_statuses();
        }
        match self.manage.tab {
            ManageTab::Console => {
                if self
                    .manage
                    .console
                    .last_poll
                    .is_none_or(|at| at.elapsed() >= CONSOLE_POLL)
                {
                    self.poll_console();
                }
            }
            ManageTab::Players => {
                if self.manage.players.add.is_none()
                    && self
                        .manage
                        .players
                        .last_poll
                        .is_none_or(|at| at.elapsed() >= PLAYERS_POLL)
                {
                    self.refresh_players();
                }
            }
            ManageTab::Mods | ManageTab::Config => {}
        }
    }

    // ---- config ----

    fn refresh_config(&mut self) {
        let id = self.manage.server_id.clone();
        match self.api_get::<hostable_core::ConfigDocument>(
            &format!("/api/servers/{id}/config"),
            &[],
            8,
        ) {
            Ok(document) => {
                let files: Vec<ConfigEntry> = document
                    .files
                    .into_iter()
                    .filter_map(|file| {
                        let name = file.path.file_name()?.to_string_lossy().to_string();
                        // Hostable's own bookkeeping, not something to hand-edit.
                        (name != "hostable.minecraft.mods.json").then(|| ConfigEntry {
                            about: config_about(&name),
                            path: file.path,
                            name,
                        })
                    })
                    .collect();
                self.manage.config.selected = self
                    .manage
                    .config
                    .selected
                    .min(files.len().saturating_sub(1));
                self.manage.config.files = files;
                self.manage.config.error = None;
            }
            Err(error) => self.manage.config.error = Some(error),
        }
        self.manage.config.loaded = true;
    }

    fn handle_config_tab_key(&mut self, code: KeyCode) {
        let len = self.manage.config.files.len();
        match code {
            KeyCode::Down | KeyCode::Char('j') => {
                if self.manage.config.selected + 1 < len {
                    self.manage.config.selected += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.manage.config.selected = self.manage.config.selected.saturating_sub(1);
            }
            KeyCode::Char('r') => self.refresh_config(),
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(file) = self.manage.config.files.get(self.manage.config.selected) {
                    self.pending_edit = Some(file.path.clone());
                }
            }
            KeyCode::Char('R') => {
                self.restart_selected();
                if self.confirm.is_none() {
                    self.manage.config.restart_needed = false;
                }
            }
            _ => {}
        }
    }

    /// The editor ran: say so, and flag a restart if the file really changed.
    pub fn finish_edit(&mut self, name: &str, changed: bool, error: Option<String>) {
        self.status = match error {
            Some(error) => format!("Could not run the editor: {error}"),
            None if changed => {
                self.manage.config.restart_needed = true;
                format!("Saved {name}. Restart the server to apply it.")
            }
            None => format!("No changes to {name}"),
        };
    }

    pub fn take_pending_edit(&mut self) -> Option<std::path::PathBuf> {
        self.pending_edit.take()
    }

    // ---- console ----

    fn poll_console(&mut self) {
        let id = self.manage.server_id.clone();
        let path = format!("/api/servers/{id}/logs");
        let cursor = self.manage.console.cursor.clone();
        let result = match &cursor {
            Some(cursor) => self.api_get::<LogChunk>(&path, &[("after", cursor.as_str())], 4),
            None => self.api_get::<LogChunk>(&path, &[("lines", "200")], 6),
        };
        self.manage.console.last_poll = Some(Instant::now());
        match result {
            Ok(chunk) => {
                self.manage.console.error = None;
                if let Some(next) = chunk.cursor {
                    self.manage.console.cursor = Some(next);
                }
                let first = !self.manage.console.loaded;
                self.manage.console.loaded = true;
                let fresh: Vec<ConsoleLine> = chunk
                    .text
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(parse_log_line)
                    .collect();
                if self.manage.console.scroll > 0 {
                    // Keep the viewport where the reader left it.
                    self.manage.console.scroll += fresh.len();
                }
                self.manage.console.lines.extend(fresh);
                if first && self.manage.console.lines.is_empty() {
                    self.manage.push(
                        LineKind::Note,
                        "No output yet. Start the server and it will appear here.",
                    );
                }
                let over = self
                    .manage
                    .console
                    .lines
                    .len()
                    .saturating_sub(MAX_CONSOLE_LINES);
                if over > 0 {
                    self.manage.console.lines.drain(0..over);
                }
            }
            Err(error) => self.manage.console.error = Some(error),
        }
    }

    fn send_console_line(&mut self) {
        let command = self
            .manage
            .console
            .input
            .trim()
            .trim_start_matches('/')
            .trim();
        if command.is_empty() {
            return;
        }
        let command = command.to_string();
        let id = self.manage.server_id.clone();
        self.manage.console.input.clear();
        self.manage.console.input_cursor = 0;
        self.manage.console.history_at = None;
        self.manage.console.scroll = 0;
        if self.manage.console.history.last() != Some(&command) {
            self.manage.console.history.push(command.clone());
        }
        self.manage.push(LineKind::Command, format!("> {command}"));
        let body = serde_json::json!({ "command": command });
        match self.api_post::<ConsoleReply>(&format!("/api/servers/{id}/console"), &body, 10) {
            Ok(reply) if reply.message.trim().is_empty() => {}
            Ok(reply) => self.manage.push(LineKind::Reply, reply.message),
            Err(error) => self.manage.push(LineKind::Error, error),
        }
        self.manage.console.last_poll = None;
    }

    fn handle_console_tab_key(&mut self, code: KeyCode) {
        let console = &mut self.manage.console;
        match code {
            KeyCode::Enter => self.send_console_line(),
            KeyCode::Left => move_cursor_left(&console.input, &mut console.input_cursor),
            KeyCode::Right => move_cursor_right(&console.input, &mut console.input_cursor),
            KeyCode::Home => console.input_cursor = 0,
            KeyCode::End => console.input_cursor = console.input.chars().count(),
            KeyCode::Backspace => backspace_at(&mut console.input, &mut console.input_cursor),
            KeyCode::Delete => delete_at(&mut console.input, &mut console.input_cursor),
            KeyCode::PageUp => console.scroll += 10,
            KeyCode::PageDown => console.scroll = console.scroll.saturating_sub(10),
            KeyCode::Up => {
                if console.history.is_empty() {
                    return;
                }
                let at = match console.history_at {
                    None => console.history.len() - 1,
                    Some(at) => at.saturating_sub(1),
                };
                console.history_at = Some(at);
                console.input = console.history[at].clone();
                console.input_cursor = console.input.chars().count();
            }
            KeyCode::Down => match console.history_at {
                Some(at) if at + 1 < console.history.len() => {
                    console.history_at = Some(at + 1);
                    console.input = console.history[at + 1].clone();
                    console.input_cursor = console.input.chars().count();
                }
                Some(_) => {
                    console.history_at = None;
                    console.input.clear();
                    console.input_cursor = 0;
                }
                None => {}
            },
            KeyCode::Char(ch) => insert_at(&mut console.input, &mut console.input_cursor, ch),
            _ => {}
        }
    }

    // ---- players ----

    fn refresh_players(&mut self) {
        let id = self.manage.server_id.clone();
        self.manage.players.last_poll = Some(Instant::now());
        match self.api_get::<PlayersData>(&format!("/api/servers/{id}/players"), &[], 5) {
            Ok(data) => {
                self.manage.players.error = None;
                self.manage.players.data = Some(data);
                self.clamp_player_selection();
            }
            Err(error) => self.manage.players.error = Some(error),
        }
    }

    fn clamp_player_selection(&mut self) {
        for column in 0..3 {
            let len = self
                .manage
                .column_names(PlayerColumn::from_index(column))
                .len();
            let selected = &mut self.manage.players.selected[column];
            *selected = (*selected).min(len.saturating_sub(1));
        }
    }

    fn player_action(&mut self, list: &str, action: &str, name: Option<&str>, done: &str) {
        let id = self.manage.server_id.clone();
        let body = serde_json::json!({ "action": action, "name": name });
        match self.api_post::<ActionReply>(&format!("/api/servers/{id}/players/{list}"), &body, 8) {
            Ok(reply) => {
                let detail = reply.message.trim();
                self.status = if detail.is_empty() {
                    done.to_string()
                } else {
                    format!("{done}: {detail}")
                };
                self.refresh_players();
            }
            Err(error) => self.status = format!("Failed: {error}"),
        }
    }

    fn handle_players_key(&mut self, code: KeyCode) {
        if let Some((column, name, cursor)) = self.manage.players.add.as_mut() {
            let column = *column;
            match code {
                KeyCode::Esc => self.manage.players.add = None,
                KeyCode::Backspace => backspace_at(name, cursor),
                KeyCode::Left => move_cursor_left(name, cursor),
                KeyCode::Right => move_cursor_right(name, cursor),
                KeyCode::Char(ch) if valid_name_char(ch) && name.chars().count() < 16 => {
                    insert_at(name, cursor, ch)
                }
                KeyCode::Enter => {
                    let name = name.clone();
                    self.manage.players.add = None;
                    if name.is_empty() {
                        return;
                    }
                    match column {
                        PlayerColumn::Ops => self.player_action(
                            "ops",
                            "op",
                            Some(&name),
                            &format!("{name} is now an operator"),
                        ),
                        _ => self.player_action(
                            "whitelist",
                            "add",
                            Some(&name),
                            &format!("{name} added to the whitelist"),
                        ),
                    }
                }
                _ => {}
            }
            return;
        }

        let column = self.manage.players.column;
        let selected = self.manage.players.selected[column.index()];
        match code {
            KeyCode::Left | KeyCode::Char('h') => {
                let next = (column.index() + 2) % 3;
                self.manage.players.column = PlayerColumn::from_index(next);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                let next = (column.index() + 1) % 3;
                self.manage.players.column = PlayerColumn::from_index(next);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.manage.column_names(column).len();
                if selected + 1 < len {
                    self.manage.players.selected[column.index()] += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.manage.players.selected[column.index()] = selected.saturating_sub(1);
            }
            KeyCode::Char('r') => self.refresh_players(),
            KeyCode::Char('w') => {
                let enabled = self
                    .manage
                    .players
                    .data
                    .as_ref()
                    .is_some_and(|data| data.whitelist_enabled);
                let (action, done) = if enabled {
                    ("off", "Whitelist off: anyone can join")
                } else {
                    ("on", "Whitelist on: only listed names can join")
                };
                self.player_action("whitelist", action, None, done);
            }
            KeyCode::Char('a') => match column {
                PlayerColumn::Online => {
                    if let Some(name) = self.manage.selected_name() {
                        self.player_action(
                            "whitelist",
                            "add",
                            Some(&name),
                            &format!("{name} added to the whitelist"),
                        );
                    }
                }
                _ => self.manage.players.add = Some((column, String::new(), 0)),
            },
            KeyCode::Char('d') | KeyCode::Delete => {
                let Some(name) = self.manage.selected_name() else {
                    return;
                };
                match column {
                    PlayerColumn::Whitelist => self.player_action(
                        "whitelist",
                        "remove",
                        Some(&name),
                        &format!("{name} removed from the whitelist"),
                    ),
                    PlayerColumn::Ops => self.player_action(
                        "ops",
                        "deop",
                        Some(&name),
                        &format!("{name} is no longer an operator"),
                    ),
                    PlayerColumn::Online => {
                        self.status = "Pick Whitelist or Ops to remove from".to_string()
                    }
                }
            }
            KeyCode::Char('o') => {
                if let Some(name) = self.manage.selected_name() {
                    self.player_action(
                        "ops",
                        "op",
                        Some(&name),
                        &format!("{name} is now an operator"),
                    );
                }
            }
            KeyCode::Char('K') => {
                if column == PlayerColumn::Online
                    && let Some(name) = self.manage.selected_name()
                {
                    let id = self.manage.server_id.clone();
                    let body = serde_json::json!({ "command": format!("kick {name}") });
                    self.status = match self.api_post::<ConsoleReply>(
                        &format!("/api/servers/{id}/console"),
                        &body,
                        8,
                    ) {
                        Ok(_) => format!("Kicked {name}"),
                        Err(error) => format!("Failed: {error}"),
                    };
                    self.refresh_players();
                }
            }
            _ => {}
        }
    }

    // ---- mods ----

    fn refresh_mods(&mut self) {
        let id = self.manage.server_id.clone();
        match self.api_get::<ModsData>(&format!("/api/servers/{id}/mods"), &[], 8) {
            Ok(data) => {
                self.manage.mods.error = None;
                self.manage.mods.data = Some(data);
                let len = self.manage.visible_mods().len();
                self.manage.mods.selected = self.manage.mods.selected.min(len.saturating_sub(1));
            }
            Err(error) => self.manage.mods.error = Some(error),
        }
    }

    fn selected_mod(&self) -> Option<InstalledMod> {
        self.manage
            .visible_mods()
            .get(self.manage.mods.selected)
            .map(|item| (*item).clone())
    }

    fn search_mods(&mut self) {
        let Some(data) = self.manage.mods.data.clone() else {
            return;
        };
        let Some(search) = self.manage.mods.search.as_mut() else {
            return;
        };
        if search.input.trim().is_empty() {
            self.status = "Type something to search for".to_string();
            return;
        }
        let version = data.game_version.clone().unwrap_or_default();
        if version.is_empty() && !search.modpacks {
            self.status =
                "Could not tell the Minecraft version. Start the server once, then try again."
                    .to_string();
            return;
        }
        let query = search.input.trim().to_string();
        let modpacks = search.modpacks;
        let mut params: Vec<(&str, &str)> = vec![
            ("query", query.as_str()),
            ("project_type", if modpacks { "modpack" } else { "mod" }),
        ];
        if modpacks {
            // A pack carries its own Minecraft version; Hostable runs Fabric packs.
            params.push(("loader", "fabric"));
        } else {
            params.push(("game_version", version.as_str()));
            if let Some(loader) = data.loader.as_deref() {
                params.push(("loader", loader));
            }
        }
        match self.api_get::<hostable_core::ModrinthSearchResponse>(
            "/api/modrinth/search",
            &params,
            15,
        ) {
            Ok(response) => {
                if let Some(search) = self.manage.mods.search.as_mut() {
                    search.results = response.hits;
                    search.selected = 0;
                    search.dirty = false;
                    if search.results.is_empty() {
                        self.status = "Nothing found for that version and loader".to_string();
                    }
                }
            }
            Err(error) => self.status = format!("Modrinth search failed: {error}"),
        }
    }

    /// The Modrinth page for what is selected: a search result while searching,
    /// otherwise the highlighted installed mod.
    pub(super) fn current_mod_link(&self) -> Option<String> {
        if let Some(search) = &self.manage.mods.search {
            let hit = search.results.get(search.selected)?;
            let slug_ok = !hit.slug.is_empty()
                && hit
                    .slug
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
            return Some(if slug_ok {
                link::modrinth_page(&hit.project_type, &hit.slug)
            } else {
                link::modrinth_project(&hit.project_id)
            });
        }
        let item = self
            .manage
            .visible_mods()
            .get(self.manage.mods.selected)
            .copied()?;
        let id = item.project_id.as_deref()?;
        id.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            .then(|| link::modrinth_project(id))
    }

    pub(super) fn open_mod_link(&mut self) {
        if self.manage.tab != ManageTab::Mods {
            return;
        }
        let Some(url) = self.current_mod_link() else {
            self.status = if self.manage.mods.search.is_some() {
                "Nothing selected to open".to_string()
            } else {
                "Hostable did not install this file, so it has no Modrinth page to open".to_string()
            };
            return;
        };
        let short = url.trim_start_matches("https://");
        self.status = match link::open(&url, super::copy_to_clipboard) {
            Ok(link::Opened::Browser) => format!("Opened {short} in your browser"),
            Ok(link::Opened::Copied) => {
                format!("No browser here (SSH?). Copied {short}, paste it in your browser")
            }
            Err(error) => error,
        };
    }

    fn install_selected_result(&mut self) {
        let Some(search) = self.manage.mods.search.as_ref() else {
            return;
        };
        let Some(hit) = search.results.get(search.selected).cloned() else {
            return;
        };
        if search.modpacks {
            self.manage.mods.search = None;
            self.create_server_from_modpack(hit);
            return;
        }
        let id = self.manage.server_id.clone();
        let body = serde_json::json!({ "project_id": hit.project_id, "title": hit.title });
        match self.api_post::<InstallReply>(&format!("/api/servers/{id}/mods/install"), &body, 180)
        {
            Ok(reply) => {
                let extra = reply.installed.len().saturating_sub(1);
                self.status = if reply.installed.is_empty() {
                    format!("{} is already installed", hit.title)
                } else if extra > 0 {
                    format!(
                        "Installed {} and {extra} required {}. Restart to load.",
                        hit.title,
                        if extra == 1 {
                            "dependency"
                        } else {
                            "dependencies"
                        }
                    )
                } else {
                    format!("Installed {}. Restart to load.", hit.title)
                };
                let _ = reply.skipped;
                if !reply.installed.is_empty() {
                    self.manage.mods.restart_needed = true;
                }
                self.manage.mods.search = None;
                self.refresh_mods();
            }
            Err(error) => self.status = format!("Install failed: {error}"),
        }
    }

    fn handle_mods_key(&mut self, code: KeyCode) {
        if self.manage.mods.search.is_some() {
            return self.handle_mod_search_key(code);
        }
        if self.manage.mods.filtering {
            match code {
                KeyCode::Esc => {
                    self.manage.mods.filter.clear();
                    self.manage.mods.filtering = false;
                }
                KeyCode::Enter => self.manage.mods.filtering = false,
                KeyCode::Backspace => {
                    self.manage.mods.filter.pop();
                }
                KeyCode::Char(ch) => self.manage.mods.filter.push(ch),
                _ => {}
            }
            self.manage.mods.selected = 0;
            return;
        }
        if let Some(filename) = self.manage.mods.pending_remove.take() {
            if matches!(code, KeyCode::Char('y') | KeyCode::Enter) {
                let id = self.manage.server_id.clone();
                let body = serde_json::json!({ "filename": filename });
                self.status = match self.api_post::<ActionReply>(
                    &format!("/api/servers/{id}/mods/remove"),
                    &body,
                    10,
                ) {
                    Ok(reply) => {
                        self.manage.mods.restart_needed = true;
                        reply.message
                    }
                    Err(error) => format!("Failed: {error}"),
                };
                self.refresh_mods();
            }
            return;
        }

        let len = self.manage.visible_mods().len();
        match code {
            KeyCode::Down | KeyCode::Char('j') => {
                if self.manage.mods.selected + 1 < len {
                    self.manage.mods.selected += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.manage.mods.selected = self.manage.mods.selected.saturating_sub(1);
            }
            KeyCode::PageDown => {
                self.manage.mods.selected =
                    (self.manage.mods.selected + 10).min(len.saturating_sub(1));
            }
            KeyCode::PageUp => {
                self.manage.mods.selected = self.manage.mods.selected.saturating_sub(10);
            }
            KeyCode::Char('/') => self.manage.mods.filtering = true,
            KeyCode::Char('p')
                if self
                    .manage
                    .mods
                    .data
                    .as_ref()
                    .is_some_and(|d| d.modpack.is_some()) =>
            {
                self.manage.mods.scope = self.manage.mods.scope.next();
                self.manage.mods.selected = 0;
            }
            KeyCode::Char('r') => self.refresh_mods(),
            KeyCode::Char('o') => self.open_mod_link(),
            KeyCode::Char('a') => {
                let can_install = self
                    .manage
                    .mods
                    .data
                    .as_ref()
                    .is_some_and(|data| data.kind.is_some());
                if can_install {
                    self.manage.mods.search = Some(ModSearch::default());
                } else {
                    self.status =
                        "Vanilla servers cannot load mods. Use Fabric or Paper.".to_string();
                }
            }
            KeyCode::Char(' ') | KeyCode::Char('e') => {
                if let Some(item) = self.selected_mod() {
                    let id = self.manage.server_id.clone();
                    let body = serde_json::json!({ "filename": item.filename });
                    self.status = match self.api_post::<ActionReply>(
                        &format!("/api/servers/{id}/mods/toggle"),
                        &body,
                        10,
                    ) {
                        Ok(reply) => {
                            self.manage.mods.restart_needed = true;
                            reply.message
                        }
                        Err(error) => format!("Failed: {error}"),
                    };
                    self.refresh_mods();
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(item) = self.selected_mod() {
                    self.manage.mods.pending_remove = Some(item.filename);
                }
            }
            KeyCode::Char('R') => {
                self.restart_selected();
                if self.confirm.is_none() {
                    self.manage.mods.restart_needed = false;
                }
            }
            _ => {}
        }
    }

    fn handle_mod_search_key(&mut self, code: KeyCode) {
        let Some(search) = self.manage.mods.search.as_mut() else {
            return;
        };
        match code {
            KeyCode::Esc => self.manage.mods.search = None,
            KeyCode::Tab | KeyCode::BackTab => {
                search.modpacks = !search.modpacks;
                search.results.clear();
                search.selected = 0;
                search.dirty = true;
                if !search.input.trim().is_empty() {
                    self.search_mods();
                }
            }
            KeyCode::Down => {
                if search.selected + 1 < search.results.len() {
                    search.selected += 1;
                }
            }
            KeyCode::Up => search.selected = search.selected.saturating_sub(1),
            KeyCode::Left => move_cursor_left(&search.input, &mut search.cursor),
            KeyCode::Right => move_cursor_right(&search.input, &mut search.cursor),
            KeyCode::Backspace => {
                backspace_at(&mut search.input, &mut search.cursor);
                search.dirty = true;
            }
            KeyCode::Char(ch) => {
                insert_at(&mut search.input, &mut search.cursor, ch);
                search.dirty = true;
            }
            KeyCode::Enter => {
                if search.dirty || search.results.is_empty() {
                    self.search_mods();
                } else {
                    self.install_selected_result();
                }
            }
            _ => {}
        }
    }

    // ---- keys ----

    pub(super) fn handle_manage_key(&mut self, code: KeyCode) {
        let typing = match self.manage.tab {
            ManageTab::Console | ManageTab::Config => false,
            ManageTab::Players => self.manage.players.add.is_some(),
            ManageTab::Mods => {
                self.manage.mods.search.is_some()
                    || self.manage.mods.filtering
                    || self.manage.mods.pending_remove.is_some()
            }
        };
        if code == KeyCode::Esc {
            match self.manage.tab {
                ManageTab::Players if typing => self.manage.players.add = None,
                ManageTab::Mods if self.manage.mods.search.is_some() => {
                    self.manage.mods.search = None
                }
                ManageTab::Mods if self.manage.mods.filtering => {
                    self.manage.mods.filter.clear();
                    self.manage.mods.filtering = false;
                }
                ManageTab::Mods if self.manage.mods.pending_remove.is_some() => {
                    self.manage.mods.pending_remove = None
                }
                _ => {
                    self.mode = Mode::ServerDetail;
                }
            }
            return;
        }
        if !typing && matches!(code, KeyCode::Tab | KeyCode::BackTab) {
            self.switch_tab(code == KeyCode::Tab);
            return;
        }
        match self.manage.tab {
            ManageTab::Console => self.handle_console_tab_key(code),
            ManageTab::Players => self.handle_players_key(code),
            ManageTab::Mods => self.handle_mods_key(code),
            ManageTab::Config => self.handle_config_tab_key(code),
        }
    }

    fn next_tab_label(&self) -> &'static str {
        let tabs = self.manage.tabs();
        let at = tabs
            .iter()
            .position(|tab| *tab == self.manage.tab)
            .unwrap_or(0);
        tabs[(at + 1) % tabs.len()].hint()
    }

    pub(super) fn manage_footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        match self.manage.tab {
            ManageTab::Console => {
                let mut hints = vec![("enter", "send"), ("↑↓", "history"), ("pgup", "scroll")];
                hints.push(("tab", self.next_tab_label()));
                hints.push(("esc", "back"));
                hints
            }
            ManageTab::Players if self.manage.players.add.is_some() => {
                vec![("enter", "add"), ("esc", "cancel")]
            }
            ManageTab::Players => vec![
                ("←→", "list"),
                ("a", "add"),
                ("d", "remove"),
                ("o", "op"),
                ("w", "whitelist on/off"),
                ("K", "kick"),
                ("tab", "mods"),
                ("esc", "back"),
            ],
            ManageTab::Config => vec![
                ("↑↓", "select"),
                ("enter", "open in vi"),
                ("R", "restart"),
                ("tab", self.next_tab_label()),
                ("esc", "back"),
            ],
            ManageTab::Mods if self.manage.mods.search.is_some() => {
                let enter = if self.manage.mods.search.as_ref().is_some_and(|s| s.modpacks) {
                    "new server"
                } else {
                    "install"
                };
                vec![
                    ("enter", enter),
                    ("tab", "switch"),
                    ("↑↓", "move"),
                    ("ctrl+o", "page"),
                    ("esc", "close"),
                ]
            }
            ManageTab::Mods if self.manage.mods.filtering => {
                vec![("enter", "keep filter"), ("esc", "clear")]
            }
            ManageTab::Mods if self.manage.mods.pending_remove.is_some() => {
                vec![("y", "delete file"), ("any", "cancel")]
            }
            ManageTab::Mods => {
                let mut hints = vec![
                    ("a", "add"),
                    ("o", "open page"),
                    ("space", "on/off"),
                    ("d", "remove"),
                    ("/", "filter"),
                ];
                if self
                    .manage
                    .mods
                    .data
                    .as_ref()
                    .is_some_and(|d| d.modpack.is_some())
                {
                    hints.push(("p", "pack / added"));
                }
                hints.push(("R", "restart"));
                hints.push(("tab", self.next_tab_label()));
                hints.push(("esc", "back"));
                hints
            }
        }
    }

    // ---- drawing ----

    pub(super) fn render_manage(&self, frame: &mut Frame<'_>, area: Rect) {
        let t = theme();
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(3),
            ])
            .split(area);

        let health = Health::from_systemd(self.server_status(&self.manage.server_id));
        let mut head = vec![Span::styled(self.manage.server_name.clone(), t.title())];
        head.push(Span::raw("   "));
        for tab in self.manage.tabs() {
            if tab == self.manage.tab {
                head.push(Span::styled(
                    tab.label(),
                    t.accent().add_modifier(Modifier::UNDERLINED),
                ));
            } else {
                head.push(Span::styled(tab.label(), t.muted()));
            }
            head.push(Span::raw("  "));
        }
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(12)])
            .split(rows[0]);
        frame.render_widget(Paragraph::new(Line::from(head)), columns[0]);
        frame.render_widget(
            Paragraph::new(Line::from(status_badge(health, true))).alignment(Alignment::Right),
            columns[1],
        );

        match self.manage.tab {
            ManageTab::Console => self.render_console_tab(frame, rows[1], rows[2]),
            ManageTab::Players => self.render_players_tab(frame, rows[1], rows[2]),
            ManageTab::Mods => self.render_mods_tab(frame, rows[1], rows[2]),
            ManageTab::Config => self.render_config_tab(frame, rows[1], rows[2]),
        }
    }

    fn render_config_tab(&self, frame: &mut Frame<'_>, sub: Rect, area: Rect) {
        let t = theme();
        let config = &self.manage.config;
        let mut spans = vec![Span::styled(
            "Opens in vi. Changes apply after a restart.",
            t.muted(),
        )];
        if config.restart_needed {
            spans.push(Span::styled(
                "   restart needed to apply changes (R)",
                t.level(Level::Warn),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), sub);
        if !config.loaded || config.files.is_empty() {
            let message = match (&config.error, config.loaded) {
                (Some(error), _) => error.clone(),
                (None, false) => "Loading…".to_string(),
                (None, true) => {
                    "No config files yet. Start the server once so it creates them.".to_string()
                }
            };
            frame.render_widget(Paragraph::new(Span::styled(message, t.muted())), area);
            return;
        }
        let width = config
            .files
            .iter()
            .map(|f| f.name.chars().count())
            .max()
            .unwrap_or(0);
        let lines: Vec<Line> = config
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                let chosen = index == config.selected;
                let style = if chosen {
                    t.accent().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Line::from(vec![
                    Span::styled(if chosen { "› " } else { "  " }, t.accent()),
                    Span::styled(format!("{:<width$}", file.name), style),
                    Span::styled(format!("   {}", file.about), t.faint()),
                ])
            })
            .collect();
        frame.render_widget(Paragraph::new(Text::from(lines)), area);
    }

    fn render_console_tab(&self, frame: &mut Frame<'_>, sub: Rect, area: Rect) {
        let t = theme();
        let console = &self.manage.console;
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(2), Constraint::Length(3)])
            .split(area);

        let hint = if let Some(error) = &console.error {
            Span::styled(
                format!("cannot read the log: {error}"),
                t.level(Level::Error),
            )
        } else if console.scroll > 0 {
            Span::styled(
                "scrolled up · pgdn to follow the live log",
                t.level(Level::Warn),
            )
        } else {
            Span::styled(
                "live log, newest at the bottom. Commands run on the server console.",
                t.muted(),
            )
        };
        frame.render_widget(Paragraph::new(hint), sub);

        let log_area = rows[0];
        let width = log_area.width.saturating_sub(1) as usize;
        let height = log_area.height as usize;
        let mut visual: Vec<(LineKind, String, bool)> = Vec::new();
        for line in &console.lines {
            for (index, row) in wrap_rows(&line.text, width.saturating_sub(2))
                .into_iter()
                .enumerate()
            {
                visual.push((line.kind, row, index > 0));
            }
        }
        let end = visual
            .len()
            .saturating_sub(console.scroll.min(visual.len()));
        let start = end.saturating_sub(height);
        let shown: Vec<Line> = visual[start..end]
            .iter()
            .map(|(kind, row, continued)| {
                let style = match kind {
                    LineKind::Log => Style::default(),
                    LineKind::Warn => t.level(Level::Warn),
                    LineKind::Error => t.level(Level::Error),
                    LineKind::Join => t.level(Level::Ok),
                    LineKind::Command => t.accent(),
                    LineKind::Reply => t.title(),
                    LineKind::Note => t.muted(),
                };
                let text = if *continued {
                    format!("  {row}")
                } else {
                    row.clone()
                };
                Line::from(Span::styled(text, style))
            })
            .collect();
        frame.render_widget(Paragraph::new(Text::from(shown)), log_area);

        let label = "Command";
        render_input_field(
            frame,
            rows[1],
            label,
            &console.input,
            true,
            console.input_cursor,
        );
        if console.input.is_empty() && rows[1].width > 30 {
            let examples = match self.manage.game {
                Some(GameKind::Minecraft) => {
                    "try: list · say hello · save-all · whitelist add name"
                }
                _ => "type a console command",
            };
            let hint_area = Rect {
                x: rows[1].x + 2,
                y: rows[1].y + 1,
                width: rows[1].width.saturating_sub(4),
                height: 1,
            };
            frame.render_widget(Paragraph::new(Span::styled(examples, t.faint())), hint_area);
        }
    }

    fn render_players_tab(&self, frame: &mut Frame<'_>, sub: Rect, area: Rect) {
        let t = theme();
        let players = &self.manage.players;
        let Some(data) = &players.data else {
            let message = players
                .error
                .clone()
                .unwrap_or_else(|| "Loading players…".to_string());
            frame.render_widget(Paragraph::new(Span::styled(message, t.muted())), sub);
            return;
        };

        let whitelist = if data.whitelist_enabled {
            Span::styled("whitelist on", t.level(Level::Ok))
        } else {
            Span::styled("whitelist off, anyone can join", t.level(Level::Warn))
        };
        let mut summary = vec![whitelist];
        if !data.online_mode {
            summary.push(Span::styled(
                "  ·  offline accounts: names are not verified",
                t.muted(),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(summary)), sub);

        let area = if let Some((column, name, cursor)) = &players.add {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(3)])
                .split(area);
            let label = if *column == PlayerColumn::Ops {
                "Make operator"
            } else {
                "Add to whitelist"
            };
            render_input_field(frame, rows[1], label, name, true, *cursor);
            rows[0]
        } else {
            area
        };

        let wide = area.width >= 66;
        let slots: Vec<(PlayerColumn, Rect)> = if wide {
            let parts = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Ratio(1, 3); 3])
                .split(area);
            (0..3)
                .map(|index| (PlayerColumn::from_index(index), parts[index]))
                .collect()
        } else {
            vec![(players.column, area)]
        };
        for (column, rect) in slots {
            let (title, entries): (String, Vec<(String, bool, Option<u8>)>) = match column {
                PlayerColumn::Online => {
                    let online = data.online.as_ref();
                    let count = online.map(|o| o.online).unwrap_or(0);
                    let title = match online.and_then(|o| o.max) {
                        Some(max) => format!("Online {count}/{max}"),
                        None => format!("Online {count}"),
                    };
                    (
                        title,
                        online
                            .map(|o| o.names.iter().map(|n| (n.clone(), true, None)).collect())
                            .unwrap_or_default(),
                    )
                }
                PlayerColumn::Whitelist => (
                    format!("Whitelist {}", data.whitelist.len()),
                    data.whitelist
                        .iter()
                        .map(|p| (p.name.clone(), p.clean, None))
                        .collect(),
                ),
                PlayerColumn::Ops => (
                    format!("Operators {}", data.ops.len()),
                    data.ops
                        .iter()
                        .map(|p| (p.name.clone(), p.clean, p.level))
                        .collect(),
                ),
            };
            let focused = column == players.column;
            let empty = match column {
                PlayerColumn::Online if data.online.is_none() => "server stopped or console off",
                PlayerColumn::Online => "nobody online",
                PlayerColumn::Whitelist if data.whitelist_enabled => "empty: nobody can join",
                PlayerColumn::Whitelist => "empty",
                PlayerColumn::Ops => "no operators",
            };
            let panel = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(t.border(focused))
                .title(Span::styled(
                    format!(" {title} "),
                    if focused { t.accent() } else { t.muted() },
                ));
            let inner = panel.inner(rect);
            frame.render_widget(panel, rect);
            if entries.is_empty() {
                frame.render_widget(
                    Paragraph::new(Span::styled(empty, t.faint())).alignment(Alignment::Left),
                    inner.inner(Margin {
                        horizontal: 1,
                        vertical: 0,
                    }),
                );
                continue;
            }
            let selected = players.selected[column.index()];
            let height = inner.height as usize;
            let offset = selected.saturating_sub(height.saturating_sub(1));
            let lines: Vec<Line> = entries
                .iter()
                .enumerate()
                .skip(offset)
                .take(height)
                .map(|(index, (name, clean, level))| {
                    let chosen = focused && index == selected;
                    let mut style = if *clean {
                        Style::default()
                    } else {
                        t.level(Level::Warn)
                    };
                    if chosen {
                        style = t.accent().add_modifier(Modifier::BOLD);
                    }
                    let mut spans = vec![
                        Span::styled(if chosen { "› " } else { "  " }, t.accent()),
                        Span::styled(name.clone(), style),
                    ];
                    if let Some(level) = level {
                        spans.push(Span::styled(format!("  L{level}"), t.faint()));
                    }
                    if !clean {
                        spans.push(Span::styled("  ! odd name", t.level(Level::Warn)));
                    }
                    Line::from(spans)
                })
                .collect();
            frame.render_widget(Paragraph::new(Text::from(lines)), inner);
        }
    }

    fn render_mods_tab(&self, frame: &mut Frame<'_>, sub: Rect, area: Rect) {
        let t = theme();
        let mods = &self.manage.mods;
        let Some(data) = &mods.data else {
            let message = mods.error.clone().unwrap_or_else(|| "Loading…".to_string());
            frame.render_widget(Paragraph::new(Span::styled(message, t.muted())), sub);
            return;
        };
        let Some(_) = data.kind.clone() else {
            let rect = centered(area, 60, 5);
            frame.render_widget(
                Paragraph::new(Text::from(vec![
                    Line::from(Span::styled("Vanilla servers cannot load mods", t.title())),
                    Line::from(Span::styled(
                        "Create the server as Fabric or Paper to use mods.",
                        t.muted(),
                    )),
                ])),
                rect,
            );
            return;
        };

        let enabled = data.items.iter().filter(|item| item.enabled).count();
        let disabled = data.items.len() - enabled;
        let engine = format!(
            "{} · Minecraft {}",
            data.server_type,
            data.game_version.as_deref().unwrap_or("unknown")
        );
        let mut summary = match &data.modpack {
            Some(pack) => {
                let in_pack = data.items.iter().filter(|item| item.in_pack).count();
                let added = data.items.len() - in_pack;
                let name = match &pack.version {
                    Some(version) => format!("{} {version}", pack.name),
                    None => pack.name.clone(),
                };
                let scope = match mods.scope {
                    ModScope::All => "",
                    ModScope::Pack => " · showing pack",
                    ModScope::Added => " · showing added",
                };
                format!("{name} · {in_pack} in pack · {added} added · {engine}{scope}")
            }
            None => format!(
                "{} mod{} · {engine}",
                enabled,
                if enabled == 1 { "" } else { "s" }
            ),
        };
        if disabled > 0 {
            summary.push_str(&format!(" · {disabled} disabled"));
        }
        let mut spans = vec![Span::styled(summary, t.muted())];
        if mods.restart_needed {
            spans.push(Span::styled(
                "   restart needed to apply changes (R)",
                t.level(Level::Warn),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), sub);

        // Selected item's page, as plain text so terminals can make it clickable
        // (over SSH this is the way to reach it).
        let mut area = area;
        if let Some(url) = self.current_mod_link()
            && area.height > 4
        {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(2), Constraint::Length(1)])
                .split(area);
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("page ", t.faint()),
                    Span::styled(url, t.muted()),
                ])),
                rows[1],
            );
            area = rows[0];
        }

        if let Some(search) = &mods.search {
            return self.render_mod_search(frame, area, search);
        }

        let mut list_area = area;
        if mods.filtering || !mods.filter.is_empty() {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(2)])
                .split(area);
            let cursor = if mods.filtering { "▏" } else { "" };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("filter ", t.muted()),
                    Span::raw(format!("{}{cursor}", mods.filter)),
                ])),
                rows[0],
            );
            list_area = rows[1];
        }

        let visible = self.manage.visible_mods();
        if visible.is_empty() {
            let message = if data.items.is_empty() {
                "No mods yet. Press a to search Modrinth.".to_string()
            } else {
                "No match.".to_string()
            };
            frame.render_widget(Paragraph::new(Span::styled(message, t.muted())), list_area);
        } else {
            let height = list_area
                .height
                .saturating_sub(u16::from(mods.pending_remove.is_some()))
                as usize;
            let offset = mods.selected.saturating_sub(height.saturating_sub(1));
            let name_width = (list_area.width as usize).saturating_sub(36).clamp(12, 40);
            let lines: Vec<Line> = visible
                .iter()
                .enumerate()
                .skip(offset)
                .take(height)
                .map(|(index, item)| {
                    let chosen = index == mods.selected;
                    let name = item
                        .title
                        .clone()
                        .unwrap_or_else(|| item.filename.trim_end_matches(".disabled").to_string());
                    let name: String = name.chars().take(name_width).collect();
                    let style = if chosen {
                        t.accent().add_modifier(Modifier::BOLD)
                    } else if item.enabled {
                        Style::default()
                    } else {
                        t.faint()
                    };
                    let state = if item.enabled {
                        Span::styled("on ", t.level(Level::Ok))
                    } else {
                        Span::styled("off", t.faint())
                    };
                    let added = data.modpack.is_some() && !item.in_pack;
                    Line::from(vec![
                        Span::styled(if chosen { "› " } else { "  " }, t.accent()),
                        Span::styled(if added { "+ " } else { "  " }, t.accent()),
                        state,
                        Span::raw("  "),
                        Span::styled(format!("{name:<name_width$}"), style),
                        Span::styled(
                            format!("  {:<10}", item.version.as_deref().unwrap_or("")),
                            t.faint(),
                        ),
                        Span::styled(format!("  {:>9}", human_size(item.size)), t.faint()),
                    ])
                })
                .collect();
            frame.render_widget(Paragraph::new(Text::from(lines)), list_area);
        }

        if let Some(filename) = &mods.pending_remove {
            let line = Rect {
                y: list_area.y + list_area.height.saturating_sub(1),
                height: 1,
                ..list_area
            };
            frame.render_widget(Clear, line);
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("Delete ", t.level(Level::Warn)),
                    Span::raw(filename.clone()),
                    Span::styled("? y to confirm", t.level(Level::Warn)),
                ])),
                line,
            );
        }
    }

    fn render_mod_search(&self, frame: &mut Frame<'_>, area: Rect, search: &ModSearch) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(2),
            ])
            .split(area);
        let filter = if search.modpacks {
            "fabric".to_string()
        } else {
            self.manage
                .mods
                .data
                .as_ref()
                .map(|data| {
                    format!(
                        "{} · {}",
                        data.loader.as_deref().unwrap_or("any"),
                        data.game_version.as_deref().unwrap_or("?")
                    )
                })
                .unwrap_or_default()
        };
        super::create::segments(frame, rows[0], search.modpacks, &filter);
        super::create::search_row(frame, rows[1], &search.input, search.cursor);
        super::create::rule(frame, rows[2]);
        let t = theme();
        let rows = [rows[0], rows[3]];
        if search.results.is_empty() {
            return;
        }
        let per_item = 2usize;
        let capacity = (rows[1].height as usize / per_item).max(1);
        let offset = search.selected.saturating_sub(capacity - 1);
        let mut lines: Vec<Line> = Vec::new();
        for (index, hit) in search
            .results
            .iter()
            .enumerate()
            .skip(offset)
            .take(capacity)
        {
            let chosen = index == search.selected;
            let width = rows[1].width as usize;
            lines.push(Line::from(vec![
                Span::styled(if chosen { "› " } else { "  " }, t.accent()),
                Span::styled(
                    hit.title.clone(),
                    if chosen {
                        t.accent().add_modifier(Modifier::BOLD)
                    } else {
                        t.title()
                    },
                ),
                Span::styled(
                    format!("  {} downloads", compact_count(hit.downloads)),
                    t.faint(),
                ),
            ]));
            let description: String = hit
                .description
                .chars()
                .take(width.saturating_sub(4))
                .collect();
            lines.push(Line::from(Span::styled(
                format!("  {description}"),
                t.muted(),
            )));
        }
        frame.render_widget(Paragraph::new(Text::from(lines)), rows[1]);
    }
}

pub(super) fn compact_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_lines_are_trimmed_and_classified() {
        let line = parse_log_line(
            "2026-11-04T10:00:00+0000 vps hostable-x[12]: [10:00:01] [Server thread/WARN]: Can't keep up!",
        );
        assert_eq!(line.kind, LineKind::Warn);
        assert_eq!(line.text, "10:00:01  Can't keep up!");

        let join = parse_log_line(
            "2026-11-04T10:00:00+0000 vps hostable-x[12]: [10:00:02] [Server thread/INFO]: Blankeos joined the game",
        );
        assert_eq!(join.kind, LineKind::Join);

        let plain =
            parse_log_line("2026-11-04T10:00:00+0000 vps systemd[1]: Started hostable-x.service.");
        assert_eq!(plain.kind, LineKind::Log);
        assert!(plain.text.contains("Started"));

        let trace = parse_log_line(
            "2026-11-04T10:00:00+0000 vps hostable-x[12]: \tat net.minecraft.Foo(Foo.java:1)",
        );
        assert_eq!(trace.kind, LineKind::Error);
    }

    #[test]
    fn long_lines_wrap_to_the_width() {
        assert_eq!(wrap_rows("abcdef", 4), vec!["abcd", "ef"]);
        assert_eq!(wrap_rows("", 4), vec![""]);
        assert!(wrap_rows("abc", 0).is_empty());
    }

    #[test]
    fn sizes_and_counts_are_short() {
        assert_eq!(human_size(2_621_440), "2.5 MB");
        assert_eq!(human_size(100), "1 KB");
        assert_eq!(compact_count(12_345), "12.3k");
        assert_eq!(compact_count(3_400_000), "3.4M");
    }
}
