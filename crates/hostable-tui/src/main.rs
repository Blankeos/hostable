mod app;
mod cli;
mod theme;
mod upgrade;

use anyhow::Result;
use app::App;
use clap::Parser;
use crossterm::event::{self, Event};
use std::path::{Path, PathBuf};
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
    // An upgrade replaces the binaries, so it must not need the daemon or its token.
    if let Some(cli::Command::Upgrade { version, check }) = &args.command {
        return upgrade::run(version.as_deref(), *check);
    }
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
        if let Some(path) = app.take_pending_edit() {
            edit_file(terminal, &mut app, &path);
        }
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

/// Hand the terminal to the user's editor (vi unless `$VISUAL` or `$EDITOR` says otherwise),
/// then take it back. The editor runs as the TUI's user, so a root-owned write must not
/// leave the game server unable to read its own file: owner and mode are put back.
fn edit_file(terminal: &mut ratatui::DefaultTerminal, app: &mut App, path: &Path) {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let before = std::fs::metadata(path).ok();
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "vi".to_string());
    let mut words = editor.split_whitespace();
    let program = words.next().unwrap_or("vi").to_string();

    ratatui::restore();
    let result = std::process::Command::new(program)
        .args(words)
        .arg(path)
        .status();
    *terminal = ratatui::init();

    let after = std::fs::metadata(path).ok();
    if let (Some(before), Some(after)) = (&before, &after) {
        if before.uid() != after.uid() || before.gid() != after.gid() {
            let _ = std::os::unix::fs::chown(path, Some(before.uid()), Some(before.gid()));
        }
        if before.mode() != after.mode() {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(before.mode()));
        }
    }
    let changed = match (&before, &after) {
        (Some(before), Some(after)) => {
            before.mtime() != after.mtime() || before.mtime_nsec() != after.mtime_nsec()
        }
        _ => false,
    };
    let error = match result {
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    };
    app.finish_edit(&name, changed, error);
    let _ = terminal.clear();
}
