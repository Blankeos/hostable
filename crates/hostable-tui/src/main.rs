mod app;

use anyhow::Result;
use app::App;
use clap::Parser;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "hostable")]
#[command(about = "Manage self-hosted game servers from your VPS")]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    daemon_url: String,

    #[arg(long, default_value = "/etc/hostable/operator-token")]
    operator_token: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run(&mut terminal, args.daemon_url, args.operator_token);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    daemon_url: String,
    operator_token: PathBuf,
) -> Result<()> {
    let mut app = App::new(daemon_url, operator_token);
    app.refresh_servers();

    loop {
        terminal.draw(|frame| app.draw(frame))?;

        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
            && app.handle_key(key.code)
        {
            break;
        }
    }

    Ok(())
}
