//! Chrome and the two main screens: header, footer, toast, server list,
//! server detail, and the two overlays (confirm and help).

use super::{App, Confirm, Mode, Screen, game_label, protocol_label};
use crate::theme::{Health, Level, status_badge, theme};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Input boxes use this. Focused boxes get the accent.
pub(super) fn block(title: &'static str, active: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(theme().border(active))
}

/// `key label   key label`, key in bold, label muted.
pub(super) fn action_spans(actions: &[(&'static str, &'static str)]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (key, label) in actions {
        if !spans.is_empty() {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(*key, theme().key()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(*label, theme().muted()));
    }
    spans
}

fn spans_width(spans: &[Span<'_>]) -> u16 {
    spans
        .iter()
        .map(|span| span.content.chars().count() as u16)
        .sum()
}

pub(super) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

impl App {
    pub fn draw(&mut self, frame: &mut Frame<'_>) {
        self.update_toast();
        let area = frame.area();
        if area.width < 40 || area.height < 10 {
            let message = Paragraph::new("Terminal too small\nneeds at least 40x10")
                .alignment(Alignment::Center)
                .style(theme().muted());
            frame.render_widget(message, centered(area, area.width, 2));
            return;
        }

        let compact = area.height < 24;
        let toast = self
            .active_toast()
            .map(|(level, text)| (level, text.to_string()));
        let toast_height = u16::from(toast.is_some() && area.height >= 12);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(if compact { 1 } else { 2 }),
                Constraint::Min(4),
                Constraint::Length(toast_height),
                Constraint::Length(1),
            ])
            .split(area);

        self.render_header(frame, rows[0]);
        let main = rows[1].inner(Margin {
            horizontal: 1,
            vertical: 0,
        });
        match self.active_screen {
            Screen::Server => self.render_servers(frame, main),
            Screen::Settings => self.render_settings(frame, main),
        }
        if let Some((level, text)) = toast
            && toast_height > 0
        {
            self.render_toast(frame, rows[2], level, &text);
        }
        self.render_footer(frame, rows[3]);

        if self.confirm.is_some() {
            self.render_confirm(frame, area);
        } else if self.help {
            self.render_help(frame, area);
        }
    }

    fn render_header(&self, frame: &mut Frame<'_>, area: Rect) {
        let t = theme();
        let area = if area.height >= 2 {
            let rule = Block::default()
                .borders(Borders::BOTTOM)
                .border_style(t.faint());
            let inner = rule.inner(area);
            frame.render_widget(rule, area);
            inner
        } else {
            area
        };
        let tab = |label: &'static str, active: bool| {
            if active {
                Span::styled(label, t.accent())
            } else {
                Span::styled(label, t.muted())
            }
        };
        let left = Line::from(vec![
            Span::raw(" "),
            Span::styled("hostable", t.accent()),
            Span::raw("   "),
            tab("Servers", self.active_screen == Screen::Server),
            Span::raw("  "),
            tab("Settings", self.active_screen == Screen::Settings),
        ]);
        let right = self.connection_spans();
        let right_width = spans_width(&right) + 1;
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(right_width)])
            .split(area);
        frame.render_widget(Paragraph::new(left), columns[0]);
        frame.render_widget(
            Paragraph::new(Line::from(right)).alignment(Alignment::Right),
            columns[1],
        );
    }

    fn connection_spans(&self) -> Vec<Span<'static>> {
        let t = theme();
        match self.connected {
            Some(true) => {
                let count = self.servers.len();
                vec![
                    Span::styled("●", Style::new().fg(t.ok)),
                    Span::styled(
                        format!(
                            " connected · {count} {}",
                            if count == 1 { "server" } else { "servers" }
                        ),
                        t.muted(),
                    ),
                ]
            }
            Some(false) => vec![Span::styled("● offline", Style::new().fg(t.bad))],
            None => vec![Span::styled(
                format!("{} connecting", self.spinner_frame()),
                t.muted(),
            )],
        }
    }

    fn render_toast(&self, frame: &mut Frame<'_>, area: Rect, level: Level, text: &str) {
        let t = theme();
        let marker = match level {
            Level::Ok => "✓",
            Level::Warn => "!",
            Level::Error => "✕",
            Level::Info => "·",
        };
        let line = Line::from(vec![
            Span::raw(" "),
            Span::styled(marker, t.level(level).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled(text.to_string(), t.level(level)),
        ]);
        frame.render_widget(Paragraph::new(line), area);
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.confirm.is_some() {
            return vec![("y", "confirm"), ("n", "cancel"), ("tab", "switch")];
        }
        if self.help {
            return vec![("esc", "close")];
        }
        match self.mode {
            Mode::Normal => match self.active_screen {
                Screen::Server if self.connected == Some(false) => {
                    vec![("r", "retry"), ("2", "settings"), ("q", "quit")]
                }
                Screen::Server => vec![
                    ("↑↓", "select"),
                    ("enter", "open"),
                    ("n", "new"),
                    ("c", "connect"),
                    ("r", "refresh"),
                    ("⇄", "settings"),
                    ("q", "quit"),
                ],
                Screen::Settings => vec![
                    ("e", "S3 backups"),
                    ("t", "test S3"),
                    ("w", "web admin"),
                    ("d", "domain"),
                    ("p", "password"),
                    ("⇄", "servers"),
                    ("esc", "back"),
                ],
            },
            Mode::ServerDetail => vec![("esc", "back"), ("y", "copy address"), ("r", "refresh")],
            Mode::CreateServer => self.create_footer_hints(),
            Mode::ConnectServer => vec![
                ("tab", "next field"),
                ("enter", "review / connect"),
                ("esc", "cancel"),
            ],
            Mode::EditS3 | Mode::EditAdminWeb | Mode::EditAdminPassword => {
                vec![("tab", "next field"), ("enter", "save"), ("esc", "cancel")]
            }
            Mode::Manage => self.manage_footer_hints(),
            Mode::PickModrinth => self.picker_footer_hints(),
            Mode::PickChoice => vec![
                ("type", "filter"),
                ("↑↓", "choose"),
                ("enter", "use"),
                ("esc", "back"),
            ],
        }
    }

    fn render_footer(&self, frame: &mut Frame<'_>, area: Rect) {
        let t = theme();
        let help_hint = self.help_available();
        let help_width: u16 = if help_hint { 8 } else { 0 };
        let budget = area.width.saturating_sub(help_width + 2) as usize;

        let mut spans = vec![Span::raw(" ")];
        let mut used = 1;
        for (index, (key, label)) in self.footer_hints().into_iter().enumerate() {
            let separator = if index == 0 { 0 } else { 2 };
            let width = separator + key.chars().count() + 1 + label.chars().count();
            if used + width > budget {
                break;
            }
            if separator > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(key, t.key()));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(label, t.muted()));
            used += width;
        }
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(help_width)])
            .split(area);
        frame.render_widget(Paragraph::new(Line::from(spans)), columns[0]);
        if help_hint {
            let line = Line::from(vec![
                Span::styled("?", t.key()),
                Span::styled(" help ", t.muted()),
            ]);
            frame.render_widget(Paragraph::new(line).alignment(Alignment::Right), columns[1]);
        }
    }

    /// `?` is a normal character inside text fields, so help only opens elsewhere.
    pub(super) fn help_available(&self) -> bool {
        self.confirm.is_none()
            && !self.help
            && matches!(self.mode, Mode::Normal | Mode::ServerDetail)
    }

    pub(super) fn render_server_table(&self, frame: &mut Frame<'_>, area: Rect) {
        let t = theme();
        if self.operator_token.is_none() {
            return self.render_notice(
                frame,
                area,
                "No operator token",
                &[
                    "The token is only readable by root.",
                    "Run `sudo hostable` instead.",
                ],
                &[],
            );
        }
        if self.connected == Some(false) {
            let url = format!("Nothing answered at {}.", self.daemon_url);
            return self.render_notice(
                frame,
                area,
                "Can't reach hostabled",
                &[url.as_str(), "Check that the service is running:"],
                &[("r", "retry")],
            );
        }
        if self.servers.is_empty() {
            if self.connected.is_none() {
                return;
            }
            return self.render_notice(
                frame,
                area,
                "No servers yet",
                &["Create a Minecraft, Palworld or ARK server, or connect a folder you already have."],
                &[("n", "New server"), ("c", "Connect existing folder")],
            );
        }

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(2)])
            .split(area);
        let heading = Line::from(vec![
            Span::styled("Servers", t.title()),
            Span::styled(format!("  {}", self.servers.len()), t.muted()),
        ]);
        frame.render_widget(Paragraph::new(heading), rows[0]);

        let width = area.width;
        let show_folder = width >= 110;
        let show_port = width >= 80;
        let status_word = width >= 80;

        let visible = rows[1].height.saturating_sub(1).max(1) as usize;
        let offset = if self.selected_server >= visible {
            self.selected_server + 1 - visible
        } else {
            0
        };

        let table_rows = self
            .servers
            .iter()
            .enumerate()
            .skip(offset)
            .take(visible)
            .map(|(index, server)| {
                let selected = index == self.selected_server;
                let health = Health::from_systemd(self.server_status(&server.id.0));
                let mut cells = vec![
                    Cell::from(if selected {
                        Span::styled("▌", t.accent())
                    } else {
                        Span::raw(" ")
                    }),
                    Cell::from(Span::styled(
                        server.name.clone(),
                        if selected {
                            t.title()
                        } else {
                            Style::new().fg(t.text)
                        },
                    )),
                    Cell::from(Line::from(status_badge(health, status_word))),
                    Cell::from(Span::styled(game_label(server.game), t.muted())),
                ];
                if show_port {
                    let mut port = Vec::new();
                    if let Some(first) = server.ports.first() {
                        port.push(Span::raw(format!(
                            "{}/{}",
                            first.port,
                            protocol_label(first.protocol)
                        )));
                        if server.ports.len() > 1 {
                            port.push(Span::styled(
                                format!(" +{}", server.ports.len() - 1),
                                t.muted(),
                            ));
                        }
                    }
                    cells.push(Cell::from(Line::from(port)));
                }
                if show_folder {
                    cells.push(Cell::from(Span::styled(
                        server.install_path.display().to_string(),
                        t.muted(),
                    )));
                }
                Row::new(cells)
            });

        let mut header = vec!["", "Name", if status_word { "Status" } else { "" }, "Game"];
        let name_width = self
            .servers
            .iter()
            .map(|server| server.name.chars().count())
            .max()
            .unwrap_or(8)
            .clamp(12, 32) as u16;
        let mut widths = vec![
            Constraint::Length(1),
            Constraint::Length(name_width),
            Constraint::Length(if status_word { 10 } else { 1 }),
            Constraint::Length(10),
        ];
        if show_port {
            header.push("Port");
            widths.push(Constraint::Length(14));
        }
        if show_folder {
            header.push("Folder");
            widths.push(Constraint::Fill(1));
        }
        let header = Row::new(header).style(t.muted().add_modifier(Modifier::BOLD));
        frame.render_widget(
            Table::new(table_rows, widths)
                .header(header)
                .column_spacing(2),
            rows[1],
        );
    }

    /// A short message with optional keys, used for empty and error states.
    fn render_notice(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        title: &str,
        body: &[&str],
        keys: &[(&'static str, &'static str)],
    ) {
        let t = theme();
        let offline = title.starts_with("Can't reach");
        let mut lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                title.to_string(),
                if offline {
                    t.error().add_modifier(Modifier::BOLD)
                } else {
                    t.title()
                },
            )),
            Line::from(""),
        ];
        for text in body {
            lines.push(Line::from(Span::styled(text.to_string(), t.muted())));
        }
        if offline {
            lines.push(Line::from(Span::styled(
                "    systemctl status hostabled",
                Style::new().fg(t.text),
            )));
        }
        if !keys.is_empty() {
            lines.push(Line::from(""));
            let mut spans = Vec::new();
            for (index, (key, label)) in keys.iter().enumerate() {
                if index > 0 {
                    spans.push(Span::raw("    "));
                }
                spans.push(Span::styled(*key, t.accent()));
                spans.push(Span::raw("  "));
                spans.push(Span::raw(*label));
            }
            lines.push(Line::from(spans));
        }
        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: true }),
            area,
        );
    }

    pub(super) fn render_server_detail(&self, frame: &mut Frame<'_>, area: Rect) {
        let t = theme();
        let Some(server) = self.servers.get(self.selected_server) else {
            return self.render_notice(frame, area, "No server selected", &[], &[]);
        };
        let health = Health::from_systemd(self.server_status(&server.id.0));
        let join = server
            .ports
            .first()
            .map(|port| {
                let host = self
                    .admin_web
                    .public_ip
                    .clone()
                    .unwrap_or_else(|| "<vps-ip>".to_string());
                format!("{host}:{}", port.port)
            })
            .unwrap_or_else(|| "no public port".to_string());
        let ports = server
            .ports
            .iter()
            .map(|port| format!("{}/{}", port.port, protocol_label(port.protocol)))
            .collect::<Vec<_>>()
            .join("  ");

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // name + status
                Constraint::Length(1), // game
                Constraint::Length(1),
                Constraint::Length(1), // join heading
                Constraint::Length(3), // join box
                Constraint::Length(1),
                Constraint::Length(4), // details
                Constraint::Length(1),
                Constraint::Length(1), // actions
                Constraint::Min(0),
            ])
            .split(area);

        let top = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(12)])
            .split(rows[0]);
        frame.render_widget(
            Paragraph::new(Span::styled(server.name.clone(), t.title())),
            top[0],
        );
        frame.render_widget(
            Paragraph::new(Line::from(status_badge(health, true))).alignment(Alignment::Right),
            top[1],
        );
        frame.render_widget(
            Paragraph::new(Span::styled(game_label(server.game), t.muted())),
            rows[1],
        );

        frame.render_widget(
            Paragraph::new(Span::styled("Join address", t.muted())),
            rows[3],
        );
        let box_width = (join.chars().count() as u16 + 4).max(24).min(rows[4].width);
        let join_box = Rect {
            width: box_width,
            ..rows[4]
        };
        frame.render_widget(
            Paragraph::new(Span::styled(join, t.title())).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(t.faint())
                    .padding(ratatui::widgets::Padding::horizontal(1)),
            ),
            join_box,
        );

        let kv = |label: &'static str, value: String| {
            Line::from(vec![
                Span::styled(format!("{label:<9}"), t.muted()),
                Span::raw(value),
            ])
        };
        let details = vec![
            kv("Ports", ports),
            kv("Folder", server.install_path.display().to_string()),
            kv("Service", server.service_name.clone()),
        ];
        frame.render_widget(Paragraph::new(Text::from(details)), rows[6]);

        let running = matches!(health, Health::Online | Health::Starting);
        let minecraft = server.game == hostable_core::GameKind::Minecraft;
        let mut actions: Vec<(&str, &str, bool)> = vec![
            ("s", "start", !running),
            ("x", "stop", running),
            ("R", "restart", running),
            ("b", "backup", true),
            ("c", "console", true),
        ];
        if minecraft {
            actions.push(("p", "players", true));
            actions.push(("m", "mods", true));
        }
        actions.push(("f", "config", true));
        let mut spans = Vec::new();
        for (index, (key, label, enabled)) in actions.iter().enumerate() {
            // A rule separates the service controls from the workspaces.
            if index == 4 {
                spans.push(Span::styled("   │   ", t.faint()));
            } else if index > 0 {
                spans.push(Span::raw("   "));
            }
            let (key_style, label_style) = if *enabled {
                (t.key(), t.muted())
            } else {
                (t.faint(), t.faint())
            };
            spans.push(Span::styled(*key, key_style));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(*label, label_style));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), rows[8]);
    }

    fn render_confirm(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(Confirm {
            action,
            server_name,
            players,
            focus_confirm,
            ..
        }) = &self.confirm
        else {
            return;
        };
        let t = theme();
        let verb = if action == "restart" {
            "Restart"
        } else {
            "Stop"
        };
        let rect = centered(area, 56, 8);
        frame.render_widget(Clear, rect);
        let title = format!(" {verb} {server_name}? ");
        let panel = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(t.border(true))
            .title(Span::styled(title, t.title()));
        let inner = panel.inner(rect).inner(Margin {
            horizontal: 1,
            vertical: 0,
        });
        frame.render_widget(panel, rect);

        let players_text = if *players == 1 {
            "1 player is online.".to_string()
        } else {
            format!("{players} players are online.")
        };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);
        frame.render_widget(
            Paragraph::new(Span::styled(players_text, Style::new().fg(t.warn))),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(format!(
                "{} disconnects them. The world is saved first.",
                if action == "restart" {
                    "Restarting"
                } else {
                    "Stopping"
                }
            ))
            .style(t.muted())
            .wrap(Wrap { trim: true }),
            rows[1],
        );
        let danger_label = format!(" {verb} anyway ");
        let cancel_style = if *focus_confirm {
            t.muted()
        } else {
            t.key().add_modifier(Modifier::REVERSED)
        };
        let danger_style = if *focus_confirm {
            Style::new()
                .fg(t.bad)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::new().fg(t.bad)
        };
        let buttons = Line::from(vec![
            Span::styled(" Cancel ", cancel_style),
            Span::raw("   "),
            Span::styled(danger_label, danger_style),
        ]);
        frame.render_widget(Paragraph::new(buttons).alignment(Alignment::Right), rows[3]);
    }

    fn render_help(&self, frame: &mut Frame<'_>, area: Rect) {
        let t = theme();
        let groups: [(&str, &[(&str, &str)]); 4] = [
            (
                "Everywhere",
                &[
                    ("1 / 2", "Servers / Settings"),
                    ("tab", "switch tab or field"),
                    ("esc", "go back or cancel"),
                    ("ctrl-c", "quit"),
                ],
            ),
            (
                "Server list",
                &[
                    ("↑↓ or j k", "select"),
                    ("enter", "open"),
                    ("n", "new server"),
                    ("c", "connect an existing folder"),
                    ("r", "refresh"),
                    ("q", "quit"),
                ],
            ),
            (
                "Server",
                &[
                    ("s  x  R", "start, stop, restart"),
                    ("b", "back up now"),
                    ("c", "live console: logs and commands"),
                    ("p", "players: whitelist, ops, kick"),
                    ("m", "mods and modpacks (Modrinth)"),
                    ("f", "config files, opened in vi"),
                    ("y", "copy the join address"),
                ],
            ),
            (
                "Settings",
                &[
                    ("e  t", "edit and test S3 backups"),
                    ("w  d  p", "web admin: toggle, domain, password"),
                ],
            ),
        ];
        let mut lines = vec![Line::from("")];
        for (title, keys) in groups {
            lines.push(Line::from(Span::styled(title, t.title())));
            for (key, label) in keys {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {key:<11}"), t.key()),
                    Span::styled(*label, t.muted()),
                ]));
            }
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            format!("Daemon  {}", self.daemon_url),
            t.faint(),
        )));
        let rect = centered(area, 58, (lines.len() as u16 + 2).min(area.height));
        frame.render_widget(Clear, rect);
        let panel = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(t.border(true))
            .title(Span::styled(" Keys ", t.title()))
            .padding(ratatui::widgets::Padding::horizontal(1));
        frame.render_widget(Paragraph::new(Text::from(lines)).block(panel), rect);
    }

    pub(super) fn spinner_frame(&self) -> &'static str {
        let tick = (self.started.elapsed().as_millis() / 100) as usize;
        SPINNER[tick % SPINNER.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::theme;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
    use std::path::PathBuf;
    use std::time::Instant;

    fn record(
        id: &str,
        name: &str,
        game: &str,
        port: u16,
        proto: &str,
    ) -> hostable_core::ServerRecord {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "game": game,
            "install_path": format!("/srv/hostable/servers/{id}"),
            "service_name": format!("hostable-{id}.service"),
            "ports": [{ "port": port, "protocol": proto, "label": "game" }],
            "adapter_config": {}
        }))
        .expect("valid server record")
    }

    fn fixture() -> App {
        let mut app = App::new("http://127.0.0.1:1".into(), PathBuf::from("/nonexistent"));
        app.operator_token = Some("token".into());
        app.connected = Some(true);
        app.servers = vec![
            record("a", "Cobbleverse", "minecraft", 25565, "tcp"),
            record("b", "Palworld Friends", "palworld", 8211, "udp"),
            record("c", "ARK Island", "ark_survival_evolved", 7777, "udp"),
        ];
        for (id, status) in [("a", "active"), ("b", "inactive"), ("c", "failed")] {
            app.server_statuses.insert(id.into(), status.into());
        }
        app
    }

    fn render(app: &mut App, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal.draw(|frame| app.draw(frame)).expect("draw");
        terminal.backend().buffer().clone()
    }

    fn text(buffer: &Buffer) -> String {
        let area = buffer.area;
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn list_shows_status_words_port_and_footer_hints() {
        let screen = text(&render(&mut fixture(), 100, 24));
        for expected in [
            "Cobbleverse",
            "online",
            "stopped",
            "failed",
            "25565/tcp",
            "3 servers",
            "enter open",
            "? help",
        ] {
            assert!(
                screen.contains(expected),
                "missing {expected:?} in:\n{screen}"
            );
        }
        assert!(
            !screen.contains("Folder"),
            "folder column is for wide terminals only"
        );
    }

    #[test]
    fn wide_terminals_add_the_folder_column() {
        let screen = text(&render(&mut fixture(), 140, 30));
        assert!(screen.contains("Folder") && screen.contains("/srv/hostable/servers/a"));
    }

    #[test]
    fn narrow_terminals_drop_port_and_status_words() {
        let screen = text(&render(&mut fixture(), 64, 20));
        assert!(screen.contains("Cobbleverse"));
        assert!(
            !screen.contains("Port"),
            "port column should be gone:\n{screen}"
        );
        assert!(
            !screen.contains("stopped"),
            "status word should be gone:\n{screen}"
        );
    }

    #[test]
    fn tiny_terminals_say_so_instead_of_garbling() {
        let screen = text(&render(&mut fixture(), 30, 8));
        assert!(screen.contains("Terminal too small"));
    }

    #[test]
    fn empty_list_explains_what_to_do() {
        let mut app = fixture();
        app.servers.clear();
        let screen = text(&render(&mut app, 100, 24));
        assert!(screen.contains("No servers yet"));
        assert!(screen.contains("New server") && screen.contains("Connect existing folder"));
    }

    #[test]
    fn unreachable_daemon_gets_its_own_screen() {
        let mut app = fixture();
        app.connected = Some(false);
        let screen = text(&render(&mut app, 100, 24));
        assert!(screen.contains("Can't reach hostabled"));
        assert!(screen.contains("systemctl status hostabled"));
        assert!(screen.contains("offline"));
    }

    #[test]
    fn missing_token_points_at_sudo() {
        let mut app = fixture();
        app.operator_token = None;
        let screen = text(&render(&mut app, 100, 24));
        assert!(screen.contains("No operator token") && screen.contains("sudo hostable"));
    }

    #[test]
    fn selected_row_has_an_accent_marker_and_no_background() {
        let buffer = render(&mut fixture(), 100, 24);
        let marker = (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
            .find(|&(x, y)| buffer[(x, y)].symbol() == "▌")
            .expect("selection marker");
        assert_eq!(buffer[marker].fg, theme().accent);
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                assert_eq!(
                    buffer[(x, y)].bg,
                    ratatui::style::Color::Reset,
                    "no cell may set a background"
                );
            }
        }
    }

    #[test]
    fn detail_shows_join_address_and_dims_impossible_actions() {
        let mut app = fixture();
        app.admin_web.public_ip = Some("203.0.113.7".into());
        app.mode = Mode::ServerDetail;
        let buffer = render(&mut app, 100, 24);
        let screen = text(&buffer);
        assert!(screen.contains("203.0.113.7:25565"), "{screen}");
        assert!(screen.contains("Join address"));
        // The server is online, so "s start" is not possible and is drawn faint.
        let row = screen
            .lines()
            .position(|line| line.contains("s start") && line.contains("x stop"))
            .expect("action bar");
        let col = screen.lines().nth(row).unwrap().find("s start").unwrap();
        assert_eq!(buffer[(col as u16, row as u16)].fg, theme().faint);
    }

    #[test]
    fn detail_footer_does_not_repeat_the_action_bar() {
        let mut app = fixture();
        app.mode = Mode::ServerDetail;
        let screen = text(&render(&mut app, 100, 24));
        let last = screen.lines().last().unwrap();
        assert!(
            last.contains("esc back") && !last.contains("restart"),
            "{last}"
        );
    }

    #[test]
    fn players_online_opens_a_confirm_that_starts_on_cancel() {
        let mut app = fixture();
        app.mode = Mode::ServerDetail;
        app.confirm = Some(Confirm {
            action: "stop".into(),
            ok_message: "stopped".into(),
            server_id: "a".into(),
            server_name: "Cobbleverse".into(),
            players: 3,
            focus_confirm: false,
        });
        let screen = text(&render(&mut app, 100, 24));
        assert!(screen.contains("Stop Cobbleverse?"));
        assert!(screen.contains("3 players are online."));
        assert!(screen.contains("Stop anyway"));

        // Esc backs out without sending anything.
        assert!(!app.handle_event(press(KeyCode::Esc)));
        assert!(app.confirm.is_none());
    }

    #[test]
    fn enter_on_the_default_button_cancels() {
        let mut app = fixture();
        app.confirm = Some(Confirm {
            action: "stop".into(),
            ok_message: "stopped".into(),
            server_id: "a".into(),
            server_name: "Cobbleverse".into(),
            players: 1,
            focus_confirm: false,
        });
        app.handle_event(press(KeyCode::Enter));
        assert!(app.confirm.is_none());
        assert!(app.status.contains("Cancelled"));
    }

    #[test]
    fn help_opens_with_question_mark_and_any_key_closes_it() {
        let mut app = fixture();
        app.handle_event(press(KeyCode::Char('?')));
        assert!(app.help);
        assert!(text(&render(&mut app, 100, 40)).contains("Server list"));
        app.handle_event(press(KeyCode::Char('x')));
        assert!(!app.help);
    }

    #[test]
    fn ctrl_c_quits_and_escape_does_not() {
        let mut app = fixture();
        assert!(!app.handle_event(press(KeyCode::Esc)));
        assert!(app.handle_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(app.handle_event(press(KeyCode::Char('q'))));
    }

    #[test]
    fn other_control_combos_do_not_type_into_the_ui() {
        let mut app = fixture();
        let before = app.mode;
        app.handle_event(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, before);
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut app = fixture();
        let mut release = press(KeyCode::Char('n'));
        release.kind = KeyEventKind::Release;
        app.handle_event(release);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn status_messages_are_classified_for_toasts() {
        use crate::app::classify_status;
        assert_eq!(classify_status("Opened Cobbleverse"), None);
        assert_eq!(classify_status("Returned to server list"), None);
        assert_eq!(classify_status("Server created"), Some(Level::Ok));
        assert_eq!(classify_status("Cobbleverse: stopped"), Some(Level::Ok));
        assert_eq!(
            classify_status("Create server failed: boom"),
            Some(Level::Error)
        );
        assert_eq!(classify_status("Name is required"), Some(Level::Warn));
        assert_eq!(
            classify_status("Cobbleverse: stopped, but could not save"),
            Some(Level::Warn)
        );
    }

    #[test]
    fn toasts_expire() {
        let mut app = fixture();
        app.status = "Server created".into();
        render(&mut app, 100, 24);
        assert!(text(&render(&mut app, 100, 24)).contains("Server created"));
        app.toast_at = Instant::now() - std::time::Duration::from_secs(30);
        assert!(!text(&render(&mut app, 100, 24)).contains("Server created"));
    }

    /// Serves canned `(status line, body)` replies and records each request line.
    fn fake_daemon(
        replies: Vec<(&'static str, &'static str)>,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (status, body) in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = [0u8; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let _ = tx.send(request.lines().next().unwrap_or_default().to_string());
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (url, rx)
    }

    #[test]
    fn stopping_with_players_asks_first_then_forces_on_yes() {
        let (url, requests) = fake_daemon(vec![
            (
                "409 Conflict",
                r#"{"error":"3 player(s) are online.","code":"players_online","players":3}"#,
            ),
            ("200 OK", r#"{"ok":true,"action":"stop","warnings":[]}"#),
            // Status refresh after the action.
            ("200 OK", r#"{"active":"inactive"}"#),
            ("200 OK", r#"{"active":"inactive"}"#),
        ]);
        let mut app = fixture();
        app.daemon_url = url;
        app.mode = Mode::ServerDetail;

        app.handle_event(press(KeyCode::Char('x')));
        assert_eq!(
            requests.recv().unwrap(),
            "POST /api/servers/a/stop HTTP/1.1"
        );
        let confirm = app.confirm.as_ref().expect("a confirmation is required");
        assert_eq!(confirm.players, 3);
        assert!(!confirm.focus_confirm, "focus must start on Cancel");

        app.handle_event(press(KeyCode::Char('y')));
        assert_eq!(
            requests.recv().unwrap(),
            "POST /api/servers/a/stop?force=true HTTP/1.1"
        );
        assert!(app.confirm.is_none());
        assert_eq!(app.status, "Cobbleverse: stopped");
    }

    #[test]
    fn daemon_errors_show_the_message_not_an_http_status() {
        let (url, _requests) = fake_daemon(vec![
            (
                "409 Conflict",
                r#"{"error":"port 25565 is already in use by another process"}"#,
            ),
            ("200 OK", r#"{"active":"inactive"}"#),
            ("200 OK", r#"{"active":"inactive"}"#),
            ("200 OK", r#"{"active":"inactive"}"#),
        ]);
        let mut app = fixture();
        app.daemon_url = url;
        app.mode = Mode::ServerDetail;
        app.handle_event(press(KeyCode::Char('s')));
        assert_eq!(
            app.status,
            "Cobbleverse failed: port 25565 is already in use by another process"
        );
        assert!(app.confirm.is_none());
    }

    fn type_text(app: &mut App, value: &str) {
        for ch in value.chars() {
            app.handle_event(press(KeyCode::Char(ch)));
        }
    }

    #[test]
    fn console_shows_the_live_log_and_runs_commands() {
        let (url, requests) = fake_daemon(vec![
            (
                "200 OK",
                r#"{"service":"s","lines":200,"cursor":"s=1","text":"2026-11-04T10:00:00+0000 vps hostable-a[1]: [10:00:01] [Server thread/INFO]: Blankeos joined the game\n2026-11-04T10:00:02+0000 vps hostable-a[1]: [10:00:02] [Server thread/WARN]: Can't keep up!"}"#,
            ),
            (
                "200 OK",
                r#"{"accepted":true,"message":"There are 1 of a max of 20 players online: Blankeos"}"#,
            ),
        ]);
        let mut app = fixture();
        app.daemon_url = url;
        app.mode = Mode::ServerDetail;

        app.handle_event(press(KeyCode::Char('c')));
        assert_eq!(app.mode, Mode::Manage);
        assert!(
            requests
                .recv()
                .unwrap()
                .starts_with("GET /api/servers/a/logs?lines=200")
        );
        let screen = text(&render(&mut app, 100, 24));
        assert!(screen.contains("Blankeos joined the game"), "{screen}");
        assert!(screen.contains("Can't keep up!"));
        assert!(screen.contains("Players"), "tabs should be visible");

        type_text(&mut app, "/list");
        app.handle_event(press(KeyCode::Enter));
        assert_eq!(
            requests.recv().unwrap(),
            "POST /api/servers/a/console HTTP/1.1"
        );
        let screen = text(&render(&mut app, 100, 24));
        assert!(screen.contains("> list"), "{screen}");
        assert!(screen.contains("There are 1 of a max of 20 players online"));

        // Up recalls the command, Esc leaves the workspace.
        app.handle_event(press(KeyCode::Up));
        assert_eq!(app.manage.console.input, "list");
        app.handle_event(press(KeyCode::Esc));
        assert_eq!(app.mode, Mode::ServerDetail);
    }

    #[test]
    fn players_tab_lists_whitelist_ops_and_flags_odd_names() {
        let (url, requests) = fake_daemon(vec![
            (
                "200 OK",
                r#"{"online":{"online":2,"max":20,"names":["Blankeos","Talaba"]},"whitelist_enabled":false,"online_mode":false,
                "whitelist":[{"name":"Blankeos","level":null,"clean":true}],
                "ops":[{"name":"Blankeos","level":4,"clean":true},{"name":"blankeos[d","level":4,"clean":false}],
                "banned":[]}"#,
            ),
            ("200 OK", r#"{"message":"Added Talaba to the whitelist"}"#),
            (
                "200 OK",
                r#"{"online":null,"whitelist_enabled":false,"online_mode":false,"whitelist":[{"name":"Blankeos","level":null,"clean":true},{"name":"Talaba","level":null,"clean":true}],"ops":[],"banned":[]}"#,
            ),
        ]);
        let mut app = fixture();
        app.daemon_url = url;
        app.mode = Mode::ServerDetail;
        app.handle_event(press(KeyCode::Char('p')));
        assert_eq!(
            requests.recv().unwrap(),
            "GET /api/servers/a/players HTTP/1.1"
        );

        let screen = text(&render(&mut app, 100, 24));
        for expected in [
            "Online 2/20",
            "Whitelist 1",
            "Operators 2",
            "whitelist off, anyone can join",
            "offline accounts",
            "odd name",
            "L4",
        ] {
            assert!(screen.contains(expected), "missing {expected:?}\n{screen}");
        }

        // Move to the second online player and whitelist them with `a`.
        app.handle_event(press(KeyCode::Down));
        app.handle_event(press(KeyCode::Char('a')));
        assert_eq!(
            requests.recv().unwrap(),
            "POST /api/servers/a/players/whitelist HTTP/1.1"
        );
        assert!(
            app.status.contains("Talaba added to the whitelist"),
            "{}",
            app.status
        );
    }

    #[test]
    fn player_names_typed_into_the_add_box_are_restricted() {
        let mut app = fixture();
        app.mode = Mode::Manage;
        app.manage.game = Some(hostable_core::GameKind::Minecraft);
        app.manage.tab = crate::app::manage::ManageTab::Players;
        app.manage.players.add = Some((
            crate::app::manage::PlayerColumn::Whitelist,
            String::new(),
            0,
        ));
        type_text(&mut app, "a b;c");
        let (_, name, _) = app.manage.players.add.clone().unwrap();
        assert_eq!(name, "abc");
    }

    #[test]
    fn mods_tab_shows_state_and_asks_before_deleting() {
        let (url, requests) = fake_daemon(vec![
            (
                "200 OK",
                r#"{"kind":"mod","server_type":"fabric","game_version":"1.21.1","loader":"fabric","items":[
                {"filename":"lithium.jar","enabled":true,"size":2621440,"title":"Lithium","version":"0.12","project_id":"gvQqBUqZ"},
                {"filename":"sodium.jar.disabled","enabled":false,"size":1048576,"title":null,"version":null,"project_id":null}]}"#,
            ),
            (
                "200 OK",
                r#"{"message":"Removed lithium.jar. Restart the server to apply."}"#,
            ),
            (
                "200 OK",
                r#"{"kind":"mod","server_type":"fabric","game_version":"1.21.1","loader":"fabric","items":[]}"#,
            ),
        ]);
        let mut app = fixture();
        app.daemon_url = url;
        app.mode = Mode::ServerDetail;
        app.handle_event(press(KeyCode::Char('m')));
        requests.recv().unwrap();
        let screen = text(&render(&mut app, 100, 24));
        for expected in [
            "1 mod ·",
            "fabric",
            "1.21.1",
            "1 disabled",
            "Lithium",
            "sodium.jar",
            "2.5 MB",
        ] {
            assert!(screen.contains(expected), "missing {expected:?}\n{screen}");
        }

        app.handle_event(press(KeyCode::Char('d')));
        let screen = text(&render(&mut app, 100, 24));
        assert!(screen.contains("lithium.jar? y to confirm"), "{screen}");
        // Any other key cancels without a request.
        app.handle_event(press(KeyCode::Char('n')));
        assert!(app.manage.mods.pending_remove.is_none());

        app.handle_event(press(KeyCode::Char('d')));
        app.handle_event(press(KeyCode::Char('y')));
        assert_eq!(
            requests.recv().unwrap(),
            "POST /api/servers/a/mods/remove HTTP/1.1"
        );
        assert!(app.manage.mods.restart_needed);
    }

    #[test]
    fn mouse_wheel_scrolls_the_console_and_lists() {
        let mut app = fixture();
        assert!(!app.wants_mouse());
        app.mode = Mode::Manage;
        app.manage.game = Some(hostable_core::GameKind::Minecraft);
        app.manage.tab = crate::app::manage::ManageTab::Console;
        assert!(app.wants_mouse());
        for index in 0..20 {
            app.manage
                .console
                .lines
                .push(crate::app::manage::ConsoleLine {
                    kind: crate::app::manage::LineKind::Log,
                    text: format!("line {index}"),
                });
        }
        app.handle_wheel(true);
        assert_eq!(app.manage.console.scroll, 3);
        app.handle_wheel(false);
        app.handle_wheel(false);
        assert_eq!(app.manage.console.scroll, 0);
    }

    #[test]
    fn mods_tab_tells_pack_mods_from_added_ones() {
        let (url, requests) = fake_daemon(vec![(
            "200 OK",
            r#"{"kind":"mod","server_type":"fabric","game_version":"1.21.1","loader":"fabric",
            "modpack":{"name":"COBBLEVERSE","version":"1.7.42","files":["cobblemon.jar"]},
            "items":[
            {"filename":"cobblemon.jar","enabled":true,"size":1024,"title":null,"version":null,"project_id":null,"in_pack":true},
            {"filename":"mine.jar","enabled":true,"size":1024,"title":null,"version":null,"project_id":null,"in_pack":false}]}"#,
        )]);
        let mut app = fixture();
        app.daemon_url = url;
        app.mode = Mode::ServerDetail;
        app.handle_event(press(KeyCode::Char('m')));
        requests.recv().unwrap();
        let screen = text(&render(&mut app, 100, 24));
        for expected in [
            "COBBLEVERSE 1.7.42",
            "1 in pack",
            "1 added",
            "cobblemon.jar",
            "mine.jar",
        ] {
            assert!(screen.contains(expected), "missing {expected:?}\n{screen}");
        }
        app.handle_event(press(KeyCode::Char('p')));
        let screen = text(&render(&mut app, 100, 24));
        assert!(screen.contains("showing pack"), "{screen}");
        assert!(
            screen.contains("cobblemon.jar") && !screen.contains("mine.jar"),
            "{screen}"
        );
        app.handle_event(press(KeyCode::Char('p')));
        let screen = text(&render(&mut app, 100, 24));
        assert!(
            screen.contains("mine.jar") && !screen.contains("cobblemon.jar"),
            "{screen}"
        );
    }

    #[test]
    fn config_tab_lists_files_and_hands_the_chosen_one_to_the_editor() {
        let (url, requests) = fake_daemon(vec![(
            "200 OK",
            r#"{"title":"Minecraft config","files":[
            {"path":"/srv/a/server.properties","format":"properties","contents":"motd=hi\n"},
            {"path":"/srv/a/hostable.minecraft.mods.json","format":"json","contents":"{}"},
            {"path":"/srv/a/ops.json","format":"json","contents":"[]"}]}"#,
        )]);
        let mut app = fixture();
        app.daemon_url = url;
        app.mode = Mode::ServerDetail;
        app.handle_event(press(KeyCode::Char('f')));
        requests.recv().unwrap();
        let screen = text(&render(&mut app, 100, 24));
        assert!(screen.contains("server.properties"), "{screen}");
        assert!(screen.contains("Operators"), "{screen}");
        assert!(!screen.contains("hostable.minecraft.mods.json"), "{screen}");
        assert!(screen.contains("enter open in vi"), "{screen}");
        app.handle_event(press(KeyCode::Down));
        app.handle_event(press(KeyCode::Enter));
        assert_eq!(
            app.take_pending_edit(),
            Some(std::path::PathBuf::from("/srv/a/ops.json"))
        );
    }

    #[test]
    fn mods_open_their_modrinth_page_and_search_switches_to_modpacks() {
        let (url, requests) = fake_daemon(vec![
            (
                "200 OK",
                r#"{"kind":"plugin","server_type":"paper","game_version":"1.21.1","loader":"paper","items":[
                {"filename":"lithium.jar","enabled":true,"size":1024,"title":"Lithium","version":"0.12","project_id":"gvQqBUqZ"},
                {"filename":"hand.jar","enabled":true,"size":1024,"title":null,"version":null,"project_id":null}]}"#,
            ),
            (
                "200 OK",
                r#"{"hits":[{"project_id":"P1","slug":"cobbleverse","title":"Cobbleverse","description":"A pack","project_type":"modpack","downloads":5,"follows":1,"versions":[],"categories":[]}],"offset":0,"limit":10,"total_hits":1}"#,
            ),
            (
                "200 OK",
                r#"{"project_id":"P1","project_type":"modpack","version_id":"V1","version_number":"1.7.42","game_versions":["1.21.1"],"loaders":["fabric"],"file":{"project_id":"P1","version_id":"V1","filename":"pack.mrpack","url":"https://cdn.modrinth.com/pack.mrpack","sha512":null,"size":10},"dependencies":[]}"#,
            ),
        ]);
        let mut app = fixture();
        app.daemon_url = url;
        app.mode = Mode::ServerDetail;
        app.handle_event(press(KeyCode::Char('m')));
        requests.recv().unwrap();
        let screen = text(&render(&mut app, 100, 24));
        // Paper says mods, not plugins, and shows the page of the selected mod.
        assert!(
            screen.contains("2 mods") && !screen.contains("plugin"),
            "{screen}"
        );
        assert!(screen.contains("modrinth.com/project/gvQqBUqZ"), "{screen}");

        app.handle_event(press(KeyCode::Char('o')));
        assert!(
            app.status.contains("modrinth.com/project/gvQqBUqZ"),
            "{}",
            app.status
        );

        // A file Hostable did not install has no page.
        app.handle_event(press(KeyCode::Down));
        app.handle_event(press(KeyCode::Char('o')));
        assert!(app.status.contains("did not install"), "{}", app.status);

        app.handle_event(press(KeyCode::Char('a')));
        type_text(&mut app, "cobble");
        app.handle_event(press(KeyCode::Tab));
        let request = requests.recv().unwrap();
        assert!(request.contains("project_type=modpack"), "{request}");
        // A pack brings its own Minecraft version and runs on Fabric.
        assert!(!request.contains("game_version"), "{request}");
        assert!(request.contains("loader=fabric"), "{request}");
        let screen = text(&render(&mut app, 100, 24));
        assert!(screen.contains("Modpacks"), "{screen}");
        assert!(
            screen.contains("modrinth.com/modpack/cobbleverse"),
            "{screen}"
        );
        app.handle_event(press(KeyCode::Enter));
        let request = requests.recv().unwrap();
        assert!(
            request.starts_with("GET /api/modrinth/projects/P1/install"),
            "{request}"
        );
        assert!(!request.contains("game_version"), "{request}");
        // Enter on a modpack opens a new server with the pack chosen and the
        // version taken from the pack.
        assert_eq!(app.mode, Mode::CreateServer);
        assert_eq!(app.create.version, "1.21.1");
        assert!(app.create.modrinth.modpack.is_some());
        assert_eq!(app.create.field, crate::app::CreateField::Name);
        assert_eq!(app.create.name, "cobbleverse");
        assert!(
            app.create.path.ends_with("/cobbleverse"),
            "{}",
            app.create.path
        );
        assert_eq!(app.status, "Added Cobbleverse");
        let screen = text(&render(&mut app, 80, 24));
        assert!(screen.contains("pack"), "{screen}");
    }

    fn selection(
        project: &str,
        version: &str,
        loader: &str,
    ) -> hostable_core::ModrinthInstallSelection {
        serde_json::from_str(&format!(
            r#"{{"project_id":"{project}","project_type":"mod","version_id":"v","version_number":"1","game_versions":["{version}"],"loaders":["{loader}"],"file":{{"project_id":"{project}","version_id":"v","filename":"a.jar","url":"https://cdn.modrinth.com/a.jar","sha512":null,"size":1}},"dependencies":[]}}"#
        ))
        .unwrap()
    }

    fn open_create(app: &mut App) {
        app.active_screen = crate::app::Screen::Server;
        app.open_create_server();
    }

    #[test]
    fn create_form_has_no_captions_and_fits_small_terminals() {
        let mut app = fixture();
        open_create(&mut app);
        for (width, height) in [(80, 24), (80, 20), (60, 24), (100, 40), (40, 12)] {
            let screen = text(&render(&mut app, width, height));
            if width >= 60 && height >= 20 {
                for label in [
                    "New server",
                    "Game",
                    "Type",
                    "Name",
                    "Folder",
                    "Version",
                    "Memory",
                    "MOTD",
                    "Modrinth",
                    "+ add",
                    "Accept Minecraft EULA",
                ] {
                    assert!(
                        screen.contains(label),
                        "{width}x{height} missing {label:?}\n{screen}"
                    );
                }
            }
            for caption in ["Enter opens", "Enter picks", "fields ", "Space toggles"] {
                assert!(
                    !screen.contains(caption),
                    "{caption:?} still shown\n{screen}"
                );
            }
        }
    }

    #[test]
    fn focus_is_a_marker_on_one_row_and_follows_tab() {
        let mut app = fixture();
        open_create(&mut app);
        let marked = |app: &mut App| -> Vec<String> {
            text(&render(app, 80, 24))
                .lines()
                .filter(|line| line.contains("› "))
                .map(str::to_string)
                .collect()
        };
        let rows = marked(&mut app);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(rows[0].contains("Name"), "{rows:?}");
        app.handle_event(press(KeyCode::Tab));
        let rows = marked(&mut app);
        assert!(rows.len() == 1 && rows[0].contains("Folder"), "{rows:?}");
    }

    #[test]
    fn folder_follows_the_name_until_edited() {
        let mut app = fixture();
        open_create(&mut app);
        app.create.name.clear();
        app.create.cursor = 0;
        type_text(&mut app, "My World");
        assert_eq!(app.create.path, "/srv/hostable/servers/my-world");
        app.handle_event(press(KeyCode::Tab));
        type_text(&mut app, "x");
        app.create.field = crate::app::CreateField::Name;
        type_text(&mut app, "2");
        assert!(app.create.path.ends_with("x"), "{}", app.create.path);
    }

    #[test]
    fn a_pack_and_extra_mods_live_in_one_list() {
        use crate::app::{CreateField, SelectedModrinthProject};
        let (url, requests) = fake_daemon(vec![
            (
                "200 OK",
                r#"{"hits":[{"project_id":"S1","slug":"sodium","title":"Sodium","description":"Faster rendering","project_type":"mod","downloads":2100000,"follows":1,"versions":[],"categories":[]}],"offset":0,"limit":10,"total_hits":1}"#,
            ),
            (
                "200 OK",
                r#"{"project_id":"S1","project_type":"mod","version_id":"v","version_number":"1","game_versions":["1.21.1"],"loaders":["fabric"],"file":{"project_id":"S1","version_id":"v","filename":"sodium.jar","url":"https://cdn.modrinth.com/sodium.jar","sha512":null,"size":1},"dependencies":[]}"#,
            ),
        ]);
        let mut app = fixture();
        app.daemon_url = url;
        open_create(&mut app);
        let status = app.apply_pack(SelectedModrinthProject {
            title: "Cobbleverse".into(),
            selection: selection("P1", "1.21.1", "fabric"),
        });
        assert_eq!(status, "Added Cobbleverse");
        assert_eq!(app.create.version, "1.21.1");

        // Version and type are the pack's, so Tab skips them.
        app.create.field = CreateField::Path;
        app.handle_event(press(KeyCode::Tab));
        assert_eq!(app.create.field, CreateField::Memory);

        // Enter on the list opens the picker on Mods, filtered to the pack.
        app.create.field = CreateField::Content;
        app.create.content_row = 1; // "+ add"
        app.handle_event(press(KeyCode::Enter));
        assert_eq!(app.mode, Mode::PickModrinth);
        let screen = text(&render(&mut app, 80, 24));
        assert!(screen.contains("fabric · 1.21.1"), "{screen}");
        type_text(&mut app, "sodium");
        app.handle_event(press(KeyCode::Enter));
        let request = requests.recv().unwrap();
        assert!(request.contains("game_version=1.21.1"), "{request}");
        assert!(request.contains("loader=fabric"), "{request}");
        app.handle_event(press(KeyCode::Enter));
        requests.recv().unwrap();
        // Adding a mod keeps the picker open and marks the row.
        assert_eq!(app.mode, Mode::PickModrinth);
        assert_eq!(app.status, "Added Sodium");
        let screen = text(&render(&mut app, 80, 24));
        assert!(screen.contains("✓"), "{screen}");
        app.handle_event(press(KeyCode::Enter));
        assert_eq!(app.status, "Already added");
        assert_eq!(app.create.modrinth.mods.len(), 1);

        app.handle_event(press(KeyCode::Esc));
        assert_eq!(app.mode, Mode::CreateServer);
        let screen = text(&render(&mut app, 80, 24));
        assert!(
            screen.contains("Cobbleverse") && screen.contains("Sodium"),
            "{screen}"
        );

        // x removes the highlighted row; removing the pack unlocks the version.
        app.create.content_row = 1;
        app.handle_event(press(KeyCode::Char('x')));
        assert!(app.create.modrinth.mods.is_empty());
        app.create.content_row = 0;
        app.handle_event(press(KeyCode::Char('x')));
        assert!(app.create.modrinth.modpack.is_none());
        assert!(!app.create.locked(CreateField::Version));
    }

    #[test]
    fn a_new_pack_drops_mods_that_do_not_fit_it() {
        use crate::app::SelectedModrinthProject;
        let mut app = fixture();
        open_create(&mut app);
        app.create.modrinth.mods.push(SelectedModrinthProject {
            title: "Old".into(),
            selection: selection("M1", "1.20.4", "fabric"),
        });
        app.create.modrinth.mods.push(SelectedModrinthProject {
            title: "New".into(),
            selection: selection("M2", "1.21.1", "fabric"),
        });
        let status = app.apply_pack(SelectedModrinthProject {
            title: "Cobbleverse".into(),
            selection: selection("P1", "1.21.1", "fabric"),
        });
        assert_eq!(status, "Added Cobbleverse, removed 1 mod");
        assert_eq!(app.create.modrinth.mods.len(), 1);
        assert_eq!(app.create.modrinth.mods[0].title, "New");
    }

    #[test]
    fn a_long_modrinth_list_says_how_many_more() {
        use crate::app::SelectedModrinthProject;
        let mut app = fixture();
        open_create(&mut app);
        for index in 0..12 {
            app.create.modrinth.mods.push(SelectedModrinthProject {
                title: format!("Mod {index}"),
                selection: selection("M", "1.20.4", "fabric"),
            });
        }
        let screen = text(&render(&mut app, 80, 24));
        assert!(screen.contains("more"), "{screen}");
        assert!(screen.contains("Accept Minecraft EULA"), "{screen}");
    }

    #[test]
    fn players_and_mods_stay_out_of_other_games() {
        let mut app = fixture();
        app.selected_server = 1; // Palworld
        app.mode = Mode::ServerDetail;
        app.handle_event(press(KeyCode::Char('p')));
        assert_eq!(app.mode, Mode::ServerDetail);
        assert!(app.status.contains("only available for Minecraft"));
        let screen = text(&render(&mut app, 100, 24));
        assert!(
            screen.contains("console") && !screen.contains("players"),
            "{screen}"
        );
    }

    #[test]
    fn workspace_fits_a_small_terminal() {
        let mut app = fixture();
        app.mode = Mode::Manage;
        app.manage.game = Some(hostable_core::GameKind::Minecraft);
        app.manage.server_id = "a".into();
        app.manage.server_name = "Cobbleverse".into();
        app.manage.console.loaded = true;
        for tab in [
            crate::app::manage::ManageTab::Console,
            crate::app::manage::ManageTab::Players,
            crate::app::manage::ManageTab::Mods,
        ] {
            app.manage.tab = tab;
            let screen = text(&render(&mut app, 44, 12));
            assert!(screen.contains("Cobbleverse"), "{screen}");
        }
    }
}
