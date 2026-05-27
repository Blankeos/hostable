use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

pub struct App {
    sidebar_items: Vec<&'static str>,
    selected_sidebar: usize,
    daemon_url: String,
    servers: Vec<hostable_core::ServerRecord>,
    status: String,
}

impl App {
    pub fn new(daemon_url: String) -> Self {
        Self {
            sidebar_items: vec![
                "Servers",
                "Add server",
                "Connect folder",
                "Backups",
                "Settings",
            ],
            selected_sidebar: 0,
            daemon_url,
            servers: Vec::new(),
            status: "Not connected".to_string(),
        }
    }

    pub fn refresh_servers(&mut self) {
        let url = format!("{}/api/servers", self.daemon_url.trim_end_matches('/'));
        match reqwest::blocking::get(url)
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json::<Vec<hostable_core::ServerRecord>>())
        {
            Ok(servers) => {
                self.status = format!("Connected to hostabled | {} server(s)", servers.len());
                self.servers = servers;
            }
            Err(error) => {
                self.status = format!("hostabled unavailable: {error}");
                self.servers.clear();
            }
        }
    }

    pub fn next_sidebar_item(&mut self) {
        self.selected_sidebar = (self.selected_sidebar + 1) % self.sidebar_items.len();
    }

    pub fn previous_sidebar_item(&mut self) {
        self.selected_sidebar = if self.selected_sidebar == 0 {
            self.sidebar_items.len() - 1
        } else {
            self.selected_sidebar - 1
        };
    }

    pub fn draw(&mut self, frame: &mut Frame<'_>) {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(3),
            ])
            .split(frame.area());

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(24), Constraint::Min(20)])
            .split(outer[1]);

        let title = Paragraph::new(Line::from(vec![
            Span::styled(
                "hostable",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  servers for you and your friends in minutes"),
        ]))
        .block(Block::default().borders(Borders::BOTTOM));
        frame.render_widget(title, outer[0]);

        let items = self
            .sidebar_items
            .iter()
            .map(|item| ListItem::new(*item))
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        state.select(Some(self.selected_sidebar));
        let sidebar = List::new(items)
            .block(Block::default().title("Menu").borders(Borders::ALL))
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            );
        frame.render_stateful_widget(sidebar, body[0], &mut state);

        let content = Paragraph::new(self.content_text())
            .block(
                Block::default()
                    .title(self.sidebar_items[self.selected_sidebar])
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: true });
        frame.render_widget(content, body[1]);

        let footer = Paragraph::new(format!(
            "q/Esc quit | j/k move | r refresh | {}",
            self.status
        ))
        .block(Block::default().borders(Borders::TOP));
        frame.render_widget(footer, outer[2]);
    }

    fn content_text(&self) -> String {
        match self.sidebar_items[self.selected_sidebar] {
            "Servers" => self.servers_text(),
            "Add server" => "Wizard placeholder:\n\n1. Choose Minecraft or ARK\n2. Accept required licenses\n3. Configure game settings\n4. Review dry-run actions\n5. Install".to_string(),
            "Connect folder" => "Import placeholder:\n\nPoint Hostable at an existing Minecraft or ARK folder and let the adapter detect it.".to_string(),
            "Backups" => "Manual backup placeholder:\n\nConfigure S3 in Settings, test the connection, then run backup now from a server.".to_string(),
            "Settings" => "Settings placeholder:\n\nS3 config, admin web toggle, admin password, and network doctor will live here.".to_string(),
            _ => String::new(),
        }
    }

    fn servers_text(&self) -> String {
        if self.servers.is_empty() {
            return "No servers registered yet.\n\nStart hostabled, then use Add server to create a Minecraft or ARK server.".to_string();
        }

        self.servers
            .iter()
            .map(|server| {
                let ports = server
                    .ports
                    .iter()
                    .map(|port| format!("{}/{}", port.port, protocol_label(port.protocol)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{}\n  id: {}\n  game: {:?}\n  path: {}\n  service: {}\n  ports: {}",
                    server.name,
                    server.id.0,
                    server.game,
                    server.install_path.display(),
                    server.service_name,
                    ports
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

fn protocol_label(protocol: hostable_core::Protocol) -> &'static str {
    match protocol {
        hostable_core::Protocol::Tcp => "tcp",
        hostable_core::Protocol::Udp => "udp",
    }
}
