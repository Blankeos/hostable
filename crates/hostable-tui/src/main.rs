mod app;

use anyhow::Result;
use app::App;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "hostable")]
#[command(about = "Manage self-hosted game servers from your VPS")]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    daemon_url: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run(&mut terminal, args.daemon_url);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, daemon_url: String) -> Result<()> {
    let mut app = App::new(daemon_url);
    app.refresh_servers();

    loop {
        terminal.draw(|frame| app.draw(frame))?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Down | KeyCode::Char('j') => app.next_sidebar_item(),
                    KeyCode::Up | KeyCode::Char('k') => app.previous_sidebar_item(),
                    KeyCode::Char('r') => app.refresh_servers(),
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
