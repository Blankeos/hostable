//! One place for every colour the TUI uses.
//!
//! Rules: never set a background colour, use the terminal's own foreground for
//! body text, and spend the accent colour only on the brand mark, the active tab,
//! the selection marker, the focused field and primary actions. The palette
//! degrades from truecolor to 256 colours to the 16 ANSI colours, and to plain
//! bold/dim text when `NO_COLOR` is set.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Mono,
    Ansi16,
    Indexed,
    TrueColor,
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub depth: Depth,
    pub text: Color,
    pub muted: Color,
    /// Rules, unfocused borders and disabled actions.
    pub faint: Color,
    pub accent: Color,
    pub ok: Color,
    pub warn: Color,
    pub bad: Color,
}

impl Theme {
    pub fn detect() -> Self {
        Self::for_depth(detect_depth())
    }

    pub fn for_depth(depth: Depth) -> Self {
        match depth {
            Depth::TrueColor => Self {
                depth,
                text: Color::Reset,
                muted: Color::Rgb(0x8d, 0x91, 0x99),
                faint: Color::Rgb(0x4a, 0x4d, 0x53),
                accent: Color::Rgb(0xe0, 0x66, 0x3a),
                ok: Color::Rgb(0x4c, 0xc3, 0x8a),
                warn: Color::Rgb(0xd9, 0xa4, 0x41),
                bad: Color::Rgb(0xec, 0x6a, 0x6a),
            },
            Depth::Indexed => Self {
                depth,
                text: Color::Reset,
                muted: Color::Indexed(245),
                faint: Color::Indexed(239),
                accent: Color::Indexed(166),
                ok: Color::Indexed(78),
                warn: Color::Indexed(179),
                bad: Color::Indexed(203),
            },
            Depth::Ansi16 => Self {
                depth,
                text: Color::Reset,
                muted: Color::DarkGray,
                faint: Color::DarkGray,
                accent: Color::Reset,
                ok: Color::Green,
                warn: Color::Yellow,
                bad: Color::Red,
            },
            Depth::Mono => Self {
                depth,
                text: Color::Reset,
                muted: Color::Reset,
                faint: Color::Reset,
                accent: Color::Reset,
                ok: Color::Reset,
                warn: Color::Reset,
                bad: Color::Reset,
            },
        }
    }

    pub fn title(&self) -> Style {
        Style::new().fg(self.text).add_modifier(Modifier::BOLD)
    }

    pub fn muted(&self) -> Style {
        let style = Style::new().fg(self.muted);
        if self.depth == Depth::Mono {
            style.add_modifier(Modifier::DIM)
        } else {
            style
        }
    }

    pub fn faint(&self) -> Style {
        let style = Style::new().fg(self.faint);
        if matches!(self.depth, Depth::Mono | Depth::Ansi16) {
            style.add_modifier(Modifier::DIM)
        } else {
            style
        }
    }

    pub fn accent(&self) -> Style {
        Style::new().fg(self.accent).add_modifier(Modifier::BOLD)
    }

    pub fn key(&self) -> Style {
        Style::new().fg(self.text).add_modifier(Modifier::BOLD)
    }

    pub fn error(&self) -> Style {
        Style::new().fg(self.bad)
    }

    pub fn border(&self, focused: bool) -> Style {
        if focused {
            Style::new().fg(self.accent)
        } else {
            self.faint()
        }
    }

    pub fn level(&self, level: Level) -> Style {
        match level {
            Level::Info => self.muted(),
            Level::Ok => Style::new().fg(self.ok),
            Level::Warn => Style::new().fg(self.warn),
            Level::Error => Style::new().fg(self.bad),
        }
    }
}

fn detect_depth() -> Depth {
    if std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) {
        return Depth::Mono;
    }
    let colorterm = std::env::var("COLORTERM")
        .unwrap_or_default()
        .to_lowercase();
    if colorterm.contains("truecolor") || colorterm.contains("24bit") {
        return Depth::TrueColor;
    }
    let term = std::env::var("TERM").unwrap_or_default();
    if term == "dumb" {
        return Depth::Mono;
    }
    if term.contains("256") {
        return Depth::Indexed;
    }
    Depth::Ansi16
}

pub fn theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(Theme::detect)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Ok,
    Warn,
    Error,
}

/// What a systemd `ActiveState` means to the person looking at the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Online,
    Starting,
    Stopping,
    Stopped,
    Failed,
    Unknown,
}

impl Health {
    pub fn from_systemd(active: &str) -> Self {
        match active {
            "active" => Self::Online,
            "activating" => Self::Starting,
            "deactivating" => Self::Stopping,
            "inactive" => Self::Stopped,
            "failed" => Self::Failed,
            _ => Self::Unknown,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Starting => "starting",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }

    fn dot(self) -> &'static str {
        match self {
            Self::Online | Self::Failed => "●",
            Self::Starting | Self::Stopping => "◐",
            Self::Stopped => "○",
            Self::Unknown => "·",
        }
    }

    fn color(self, theme: &Theme) -> Color {
        match self {
            Self::Online => theme.ok,
            Self::Failed => theme.bad,
            Self::Starting | Self::Stopping => theme.warn,
            Self::Stopped => theme.muted,
            Self::Unknown => theme.faint,
        }
    }
}

/// `● online`. The shape alone tells stopped (hollow) from running (solid).
pub fn status_badge(health: Health, with_word: bool) -> Vec<Span<'static>> {
    let theme = theme();
    let style = Style::new().fg(health.color(theme));
    let mut spans = vec![Span::styled(health.dot(), style)];
    if with_word {
        spans.push(Span::styled(format!(" {}", health.word()), style));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_changes_the_palette_but_never_the_text_colour() {
        for depth in [Depth::Mono, Depth::Ansi16, Depth::Indexed, Depth::TrueColor] {
            assert_eq!(Theme::for_depth(depth).text, Color::Reset);
        }
        assert_eq!(
            Theme::for_depth(Depth::TrueColor).accent,
            Color::Rgb(0xe0, 0x66, 0x3a)
        );
        assert_eq!(Theme::for_depth(Depth::Indexed).accent, Color::Indexed(166));
    }

    #[test]
    fn health_maps_systemd_states() {
        assert_eq!(Health::from_systemd("active"), Health::Online);
        assert_eq!(Health::from_systemd("inactive"), Health::Stopped);
        assert_eq!(Health::from_systemd("garbage"), Health::Unknown);
    }
}
