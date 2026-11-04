//! Opening a web page from the TUI. On the server's own desktop that launches
//! the browser. Over SSH there is no browser on this machine, so the link is
//! copied to the local clipboard instead (OSC 52) and shown on screen, where
//! most terminals make it clickable.

use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Opened {
    /// A browser was launched on this machine.
    Browser,
    /// No browser here (SSH or headless); the link was copied instead.
    Copied,
}

/// True when a browser launched from this process would not show up in front of the user.
pub(super) fn is_remote() -> bool {
    remote_from(
        |key| std::env::var_os(key).is_some(),
        cfg!(target_os = "linux"),
    )
}

fn remote_from(has: impl Fn(&str) -> bool, linux: bool) -> bool {
    if has("SSH_CONNECTION") || has("SSH_CLIENT") || has("SSH_TTY") {
        return true;
    }
    linux && !has("DISPLAY") && !has("WAYLAND_DISPLAY")
}

fn launcher() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    }
}

/// Only plain https links are ever handed to the OS.
pub(super) fn safe_url(url: &str) -> bool {
    url.starts_with("https://") && !url.chars().any(|ch| ch.is_control() || ch == ' ')
}

pub(super) fn open(url: &str, copy: impl FnOnce(&str)) -> Result<Opened, String> {
    if !safe_url(url) {
        return Err("Refusing to open an unexpected link".to_string());
    }
    if is_remote() {
        copy(url);
        return Ok(Opened::Copied);
    }
    if cfg!(test) {
        // Tests must never pop open a real browser.
        return Ok(Opened::Browser);
    }
    match Command::new(launcher())
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(Opened::Browser)
        }
        Err(_) => {
            copy(url);
            Ok(Opened::Copied)
        }
    }
}

pub(super) fn modrinth_page(project_type: &str, slug: &str) -> String {
    let kind = match project_type {
        "modpack" => "modpack",
        "plugin" => "plugin",
        _ => "mod",
    };
    format!("https://modrinth.com/{kind}/{slug}")
}

pub(super) fn modrinth_project(project_id: &str) -> String {
    // Modrinth redirects /project/<id> to the canonical page for any project type.
    format!("https://modrinth.com/project/{project_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_and_headless_linux_are_remote() {
        assert!(remote_from(|k| k == "SSH_CONNECTION", false));
        assert!(remote_from(|k| k == "SSH_TTY", true));
        assert!(remote_from(|_| false, true));
        assert!(!remote_from(|k| k == "DISPLAY", true));
        assert!(!remote_from(|_| false, false));
    }

    #[test]
    fn only_clean_https_links_open() {
        assert!(safe_url("https://modrinth.com/mod/lithium"));
        assert!(!safe_url("http://modrinth.com/mod/x"));
        assert!(!safe_url("file:///etc/passwd"));
        assert!(!safe_url("https://a.com/x y"));
        assert!(!safe_url("https://a.com/\u{1b}[31m"));
    }

    #[test]
    fn remote_open_copies_instead_of_launching() {
        // Force the remote path regardless of the test machine.
        let mut copied = None;
        let url = modrinth_page("mod", "lithium");
        if is_remote() {
            let result = open(&url, |text| copied = Some(text.to_string()));
            assert_eq!(result, Ok(Opened::Copied));
            assert_eq!(copied.as_deref(), Some(url.as_str()));
        }
        assert_eq!(url, "https://modrinth.com/mod/lithium");
        assert_eq!(
            modrinth_project("AANobbMI"),
            "https://modrinth.com/project/AANobbMI"
        );
    }
}
