mod app;
mod cli;
mod theme;

use anyhow::Result;
use app::App;
use clap::Parser;
use crossterm::event::{self, Event};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "hostable", version)]
#[command(about = "Manage self-hosted game servers from your VPS")]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    daemon_url: String,

    #[arg(long, default_value = "/etc/hostable/operator-token")]
    operator_token: PathBuf,

    /// Run one command and exit. Without a command, the interactive TUI opens.
    #[command(subcommand)]
    command: Option<cli::Command>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if let Some(command) = args.command {
        let api = cli::Api::new(&args.daemon_url, &args.operator_token)?;
        return cli::run(command, &api);
    }
    // `init` also installs a panic hook, so a crash restores the terminal.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, args.daemon_url, args.operator_token);
    ratatui::restore();
    result
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    daemon_url: String,
    operator_token: PathBuf,
) -> Result<()> {
    let mut app = App::new(daemon_url, operator_token);
    // Draw once before the first (blocking) request so the screen is not blank.
    terminal.draw(|frame| app.draw(frame))?;
    app.refresh_servers();
    app.refresh_public_ip();

    loop {
        terminal.draw(|frame| app.draw(frame))?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()?
                && app.handle_event(key)
            {
                break;
            }
        } else {
            app.tick();
        }
    }

    Ok(())
}
