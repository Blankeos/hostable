//! The "New server" form and the Modrinth picker that feeds it.
//!
//! One row per field, two short fields to a row where they fit, no boxes and no
//! captions: the layout and the footer keys carry the meaning.

use super::manage::compact_count;
use super::*;
use crate::theme::Theme;

const LABEL_WIDTH: u16 = 9;
const MARKER: &str = "› ";
const SERVERS_ROOT: &str = "/srv/hostable/servers";

// ---- draft helpers ------------------------------------------------------------------

/// Lowercase letters, digits and dashes: what a folder or server id can be made of.
pub(super) fn slugify(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    trimmed
        .chars()
        .take(32)
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

impl CreateDraft {
    /// Version and type belong to the chosen pack, so they cannot be edited.
    pub(super) fn locked(&self, field: CreateField) -> bool {
        self.modrinth.modpack.is_some() && matches!(field, CreateField::Type | CreateField::Version)
    }

    /// Mods only exist for Minecraft.
    pub(super) fn has_content(&self) -> bool {
        self.game == hostable_core::GameKind::Minecraft
    }

    pub(super) fn skipped(&self, field: CreateField) -> bool {
        self.locked(field) || (field == CreateField::Content && !self.has_content())
    }

    /// Keep the folder in step with the name until the folder is edited by hand.
    pub(super) fn sync_path(&mut self) {
        if !self.path_edited {
            let slug = slugify(&self.name);
            let slug = if slug.is_empty() {
                "server"
            } else {
                slug.as_str()
            };
            self.path = format!("{SERVERS_ROOT}/{slug}");
        }
    }

    fn content_len(&self) -> usize {
        self.modrinth.len() + 1
    }
}

impl ModrinthDraft {
    pub(super) fn len(&self) -> usize {
        usize::from(self.modpack.is_some()) + self.mods.len()
    }

    /// Title and tag for row `index`: the pack first, then the mods.
    fn row(&self, index: usize) -> Option<(&str, &'static str)> {
        if let Some(pack) = &self.modpack {
            if index == 0 {
                return Some((pack.title.as_str(), "pack"));
            }
            return self
                .mods
                .get(index - 1)
                .map(|item| (item.title.as_str(), "mod"));
        }
        self.mods
            .get(index)
            .map(|item| (item.title.as_str(), "mod"))
    }

    fn remove(&mut self, index: usize) -> Option<String> {
        if self.modpack.is_some() {
            if index == 0 {
                return self.modpack.take().map(|pack| pack.title);
            }
            return (index - 1 < self.mods.len()).then(|| self.mods.remove(index - 1).title);
        }
        (index < self.mods.len()).then(|| self.mods.remove(index).title)
    }

    pub(super) fn contains(&self, project_id: &str) -> bool {
        self.modpack
            .iter()
            .chain(self.mods.iter())
            .any(|item| item.selection.project_id == project_id)
    }

    /// Drop mods that do not fit the (new) Minecraft version or loader. Returns how many.
    fn retain_compatible(&mut self, version: &str, loader: &str) -> usize {
        let before = self.mods.len();
        self.mods.retain(|item| {
            let versions = &item.selection.game_versions;
            let loaders = &item.selection.loaders;
            (versions.is_empty() || versions.iter().any(|v| v == version))
                && (loaders.is_empty() || loaders.iter().any(|l| l == loader))
        });
        before - self.mods.len()
    }
}

fn short(title: &str) -> String {
    if title.chars().count() > 32 {
        format!("{}…", title.chars().take(31).collect::<String>())
    } else {
        title.to_string()
    }
}

// ---- drawing ------------------------------------------------------------------------

struct FieldView<'a> {
    label: &'static str,
    value: &'a str,
    suffix: &'static str,
    focused: bool,
    locked: bool,
    /// Cursor position when this is a text field.
    cursor: Option<usize>,
}

fn draw_label(frame: &mut Frame<'_>, rect: Rect, label: &str, focused: bool, t: &Theme) {
    let style = if focused {
        t.accent().add_modifier(Modifier::BOLD)
    } else {
        t.muted()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(if focused { MARKER } else { "  " }, t.accent()),
            Span::styled(format!("{label:<w$}", w = LABEL_WIDTH as usize), style),
        ])),
        Rect {
            width: rect.width.min(LABEL_WIDTH + 2),
            ..rect
        },
    );
}

fn draw_field(frame: &mut Frame<'_>, rect: Rect, field: FieldView<'_>) {
    let t = theme();
    if rect.width <= LABEL_WIDTH + 3 || rect.height == 0 {
        return;
    }
    draw_label(frame, rect, field.label, field.focused, t);
    let value_rect = Rect {
        x: rect.x + LABEL_WIDTH + 2,
        width: rect.width - LABEL_WIDTH - 2,
        ..rect
    };
    let suffix_width = field.suffix.chars().count();
    let text_width = (value_rect.width as usize).saturating_sub(suffix_width);
    let (visible, cursor_col) = match field.cursor {
        Some(cursor) if field.focused => visible_input(field.value, cursor, text_width),
        _ => (field.value.chars().take(text_width).collect::<String>(), 0),
    };
    let shown = visible.chars().count();
    let style = if field.locked {
        t.faint()
    } else {
        Style::default()
    };
    let mut spans = vec![Span::styled(visible, style)];
    if !field.suffix.is_empty() {
        spans.push(Span::styled(field.suffix, t.faint()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), value_rect);
    if field.focused && field.cursor.is_some() {
        let col = cursor_col.min(shown.max(cursor_col));
        frame.set_cursor_position(Position::new(value_rect.x + col as u16, value_rect.y));
    }
}

/// `Mods   Modpacks`: the active segment in the accent colour.
pub(super) fn segments(frame: &mut Frame<'_>, rect: Rect, packs: bool, right: &str) {
    let t = theme();
    let on = t.accent().add_modifier(Modifier::BOLD);
    let off = t.muted();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Mods", if packs { off } else { on }),
            Span::raw("   "),
            Span::styled("Modpacks", if packs { on } else { off }),
        ])),
        rect,
    );
    if !right.is_empty() {
        let width = right.chars().count() as u16;
        if width + 20 < rect.width {
            frame.render_widget(
                Paragraph::new(Span::styled(right.to_string(), t.faint())),
                Rect {
                    x: rect.x + rect.width - width,
                    width,
                    ..rect
                },
            );
        }
    }
}

/// `search  text▏` on one row with the real terminal cursor.
pub(super) fn search_row(frame: &mut Frame<'_>, rect: Rect, value: &str, cursor: usize) {
    let t = theme();
    let label_width = 8u16;
    frame.render_widget(
        Paragraph::new(Span::styled(format!("{:<8}", "search"), t.muted())),
        rect,
    );
    if rect.width <= label_width + 1 {
        return;
    }
    let value_rect = Rect {
        x: rect.x + label_width,
        width: rect.width - label_width,
        ..rect
    };
    let (visible, col) = visible_input(value, cursor, value_rect.width as usize);
    frame.render_widget(Paragraph::new(visible), value_rect);
    frame.set_cursor_position(Position::new(value_rect.x + col as u16, value_rect.y));
}

pub(super) fn rule(frame: &mut Frame<'_>, rect: Rect) {
    frame.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(rect.width as usize),
            theme().faint(),
        )),
        rect,
    );
}

fn row_rects(area: Rect, heights: &[u16]) -> Vec<Rect> {
    let mut y = area.y;
    let bottom = area.y + area.height;
    heights
        .iter()
        .map(|height| {
            let h = (*height).min(bottom.saturating_sub(y));
            let rect = Rect {
                y,
                height: h,
                ..area
            };
            y += h;
            rect
        })
        .collect()
}

impl App {
    pub(super) fn render_create_server(&self, frame: &mut Frame<'_>, area: Rect) {
        let t = theme();
        let draft = &self.create;
        let gap = u16::from(area.height >= 20);
        let paired = area.width >= 64;
        let minecraft = draft.game == hostable_core::GameKind::Minecraft;
        let focus = |field: CreateField| draft.field == field;

        // title, blank, game/type, name, folder, version/memory, motd, blank, content, blank, license
        let field_rows: u16 = if paired { 5 } else { 7 };
        let mut heights = vec![1, gap];
        heights.extend(std::iter::repeat_n(1, field_rows as usize));
        let used_above: u16 = heights.iter().sum();
        let license_rows = 1 + gap;
        let content_header = if minecraft { gap + 1 } else { 0 };
        let list_room = area
            .height
            .saturating_sub(used_above + license_rows + content_header);
        heights.push(content_header);
        // The list takes what it needs (up to the room left); the licence follows it.
        let list_rows = (draft.content_len() as u16).min(list_room);
        heights.push(if minecraft { list_rows } else { 0 });
        heights.push(gap);
        heights.push(1);
        let rects = row_rects(area, &heights);

        frame.render_widget(
            Paragraph::new(Span::styled("New server", t.title())),
            rects[0],
        );

        let game = game_label(draft.game);
        let server_type = server_type_label(draft.server_type.clone());
        let pair =
            |left: FieldView<'_>, right: FieldView<'_>, rect: Rect, frame: &mut Frame<'_>| {
                let left_width = (rect.width * 55 / 100).min(44);
                let halves = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Length(left_width), Constraint::Min(0)])
                    .split(rect);
                draw_field(frame, halves[0], left);
                draw_field(frame, halves[1], right);
            };
        let select = |label, value, field| FieldView {
            label,
            value,
            suffix: "",
            focused: focus(field),
            locked: draft.locked(field),
            cursor: None,
        };
        let text = |label, value, field| FieldView {
            label,
            value,
            suffix: "",
            focused: focus(field),
            locked: draft.locked(field),
            cursor: Some(draft.cursor),
        };
        let memory = FieldView {
            suffix: " MB",
            ..text("Memory", draft.memory_mb.as_str(), CreateField::Memory)
        };

        let mut at = 2;
        if paired {
            pair(
                select("Game", game, CreateField::Game),
                select("Type", server_type, CreateField::Type),
                rects[at],
                frame,
            );
            at += 1;
        } else {
            draw_field(frame, rects[at], select("Game", game, CreateField::Game));
            draw_field(
                frame,
                rects[at + 1],
                select("Type", server_type, CreateField::Type),
            );
            at += 2;
        }
        draw_field(
            frame,
            rects[at],
            text("Name", &draft.name, CreateField::Name),
        );
        draw_field(
            frame,
            rects[at + 1],
            text("Folder", &draft.path, CreateField::Path),
        );
        at += 2;
        if paired {
            pair(
                text("Version", &draft.version, CreateField::Version),
                memory,
                rects[at],
                frame,
            );
            at += 1;
        } else {
            draw_field(
                frame,
                rects[at],
                text("Version", &draft.version, CreateField::Version),
            );
            draw_field(frame, rects[at + 1], memory);
            at += 2;
        }
        draw_field(
            frame,
            rects[at],
            text("MOTD", &draft.motd, CreateField::Motd),
        );
        at += 1;

        // Modrinth: the pack, the mods, then "+ add".
        if minecraft {
            let header = rects[at];
            let list = rects[at + 1];
            let header_row = Rect {
                y: header.y + header.height.saturating_sub(1),
                height: 1.min(header.height),
                ..header
            };
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "  Modrinth",
                    if focus(CreateField::Content) {
                        t.accent().add_modifier(Modifier::BOLD)
                    } else {
                        t.muted()
                    },
                )),
                header_row,
            );
            self.render_content_list(frame, list);
        }
        at += 2;

        let license_rect = rects[at + 1];
        let label = if minecraft {
            "Accept Minecraft EULA"
        } else {
            "Accept license"
        };
        let focused = focus(CreateField::License);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(if focused { MARKER } else { "  " }, t.accent()),
                Span::styled(
                    format!(
                        "[{}] {label}",
                        if draft.accepted_license { "x" } else { " " }
                    ),
                    if focused {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ])),
            license_rect,
        );
    }

    fn render_content_list(&self, frame: &mut Frame<'_>, rect: Rect) {
        let t = theme();
        let draft = &self.create;
        if rect.height == 0 {
            return;
        }
        let total = draft.content_len();
        let room = rect.height as usize;
        let focused = draft.field == CreateField::Content;
        // Window the rows so the highlighted one stays visible.
        let (start, shown, more) = if total <= room {
            (0, total, 0)
        } else {
            let visible = room.saturating_sub(1).max(1);
            let start = draft
                .content_row
                .saturating_sub(visible.saturating_sub(1))
                .min(total - visible);
            (start, visible, total - start - visible)
        };
        let width = rect.width as usize;
        let mut lines: Vec<Line> = Vec::new();
        for index in start..start + shown {
            let chosen = focused && index == draft.content_row;
            let marker = Span::styled(if chosen { MARKER } else { "  " }, t.accent());
            match draft.modrinth.row(index) {
                Some((title, tag)) => {
                    let name_width = width.saturating_sub(2 + 2 + tag.len() + 2).max(4);
                    let name: String = title.chars().take(name_width).collect();
                    let pad = width
                        .saturating_sub(2 + 2 + name.chars().count() + tag.len())
                        .max(1);
                    lines.push(Line::from(vec![
                        marker,
                        Span::raw("  "),
                        Span::styled(
                            name,
                            if chosen {
                                Style::default().add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                            },
                        ),
                        Span::raw(" ".repeat(pad)),
                        Span::styled(tag, t.faint()),
                    ]));
                }
                None => lines.push(Line::from(vec![
                    marker,
                    Span::raw("  "),
                    Span::styled(
                        "+ add",
                        if chosen {
                            t.accent().add_modifier(Modifier::BOLD)
                        } else {
                            t.muted()
                        },
                    ),
                ])),
            }
        }
        if more > 0 {
            lines.push(Line::from(Span::styled(
                format!("    +{more} more"),
                t.faint(),
            )));
        }
        frame.render_widget(Paragraph::new(Text::from(lines)), rect);
    }

    pub(super) fn render_modrinth_picker(&self, frame: &mut Frame<'_>, area: Rect) {
        let t = theme();
        let picker = &self.modrinth_picker;
        let packs = picker.kind == ModrinthPickKind::Modpack;
        let rects = row_rects(area, &[1, 1, 1, area.height.saturating_sub(3)]);
        let filter = if packs {
            "fabric".to_string()
        } else {
            let loader = modrinth_loader(&self.create.server_type).unwrap_or("any");
            format!("{loader} · {}", self.create.version)
        };
        segments(frame, rects[0], packs, &filter);
        search_row(frame, rects[1], &picker.query, picker.cursor);
        rule(frame, rects[2]);

        let list = rects[3];
        let per_item = if list.height >= 8 { 2 } else { 1 };
        let capacity = (list.height as usize / per_item).max(1);
        let offset = picker.selected.saturating_sub(capacity - 1);
        let width = list.width as usize;
        let mut lines: Vec<Line> = Vec::new();
        for (index, hit) in picker
            .results
            .iter()
            .enumerate()
            .skip(offset)
            .take(capacity)
        {
            let chosen = index == picker.selected;
            let added = self.create.modrinth.contains(&hit.project_id);
            let downloads = compact_count(hit.downloads);
            let tail = if added {
                format!("{downloads}  ✓")
            } else {
                downloads
            };
            let name_width = width.saturating_sub(2 + tail.chars().count() + 2).max(4);
            let name: String = hit.title.chars().take(name_width).collect();
            let pad = width
                .saturating_sub(2 + name.chars().count() + tail.chars().count())
                .max(1);
            lines.push(Line::from(vec![
                Span::styled(if chosen { MARKER } else { "  " }, t.accent()),
                Span::styled(
                    name,
                    if chosen {
                        t.accent().add_modifier(Modifier::BOLD)
                    } else {
                        t.title()
                    },
                ),
                Span::raw(" ".repeat(pad)),
                Span::styled(tail, if added { t.level(Level::Ok) } else { t.faint() }),
            ]));
            if per_item == 2 {
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
        }
        if picker.results.is_empty() && !picker.dirty {
            lines.push(Line::from(Span::styled("  No results", t.muted())));
        }
        frame.render_widget(Paragraph::new(Text::from(lines)), list);
    }

    pub(super) fn create_footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        let review = if self.create.dry_run_reviewed {
            "install"
        } else {
            "review"
        };
        match self.create.field {
            CreateField::Game | CreateField::Type => {
                vec![("tab", "next"), ("enter", "choose"), ("esc", "cancel")]
            }
            CreateField::Content => {
                let mut hints = vec![("tab", "next"), ("enter", "add")];
                if self.create.content_row < self.create.modrinth.len() {
                    hints.push(("x", "remove"));
                }
                hints.push(("esc", "cancel"));
                hints
            }
            CreateField::License => {
                vec![("space", "accept"), ("enter", review), ("esc", "cancel")]
            }
            _ => vec![("tab", "next"), ("enter", review), ("esc", "cancel")],
        }
    }

    pub(super) fn picker_footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        let enter = if self.modrinth_picker.dirty || self.modrinth_picker.results.is_empty() {
            "search"
        } else if self.modrinth_picker.kind == ModrinthPickKind::Modpack {
            "use"
        } else {
            "add"
        };
        vec![
            ("enter", enter),
            ("tab", "switch"),
            ("↑↓", "move"),
            ("esc", "done"),
        ]
    }

    // ---- keys -----------------------------------------------------------------------

    pub(super) fn open_modrinth_picker(&mut self) {
        if !self.create.has_content() {
            return;
        }
        self.mode = Mode::PickModrinth;
        self.modrinth_picker = ModrinthPicker {
            kind: if self.create.modrinth.modpack.is_some() {
                ModrinthPickKind::Mod
            } else {
                ModrinthPickKind::Modpack
            },
            ..ModrinthPicker::default()
        };
        self.status.clear();
    }

    pub(super) fn handle_create_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status.clear();
            }
            KeyCode::Tab => self.create.next_field(),
            KeyCode::BackTab => self.create.previous_field(),
            KeyCode::Down => self.create_move(true),
            KeyCode::Up => self.create_move(false),
            KeyCode::Left => self.create.move_cursor_left(),
            KeyCode::Right => self.create.move_cursor_right(),
            KeyCode::Home => self.create.move_cursor_start(),
            KeyCode::End => self.create.move_cursor_end(),
            KeyCode::Char(' ') => {
                if self.create.field == CreateField::License {
                    self.create.toggle_license();
                } else {
                    self.create.insert_char(' ');
                }
            }
            KeyCode::Char('x') if self.create.field == CreateField::Content => {
                self.remove_content()
            }
            KeyCode::Delete if self.create.field == CreateField::Content => self.remove_content(),
            KeyCode::Backspace => self.create.backspace(),
            KeyCode::Delete => self.create.delete_char(),
            KeyCode::Enter => match self.create.field {
                CreateField::Game => self.open_choice_picker(ChoiceTarget::CreateGame),
                CreateField::Type => self.open_choice_picker(ChoiceTarget::CreateType),
                CreateField::Content => self.open_modrinth_picker(),
                _ => self.submit_create_server(),
            },
            KeyCode::Char(ch) => self.create.insert_char(ch),
            _ => {}
        }
    }

    /// Up and down walk the Modrinth rows one by one, then move on to the next field.
    fn create_move(&mut self, down: bool) {
        let draft = &mut self.create;
        if draft.field == CreateField::Content {
            let last = draft.content_len() - 1;
            if down && draft.content_row < last {
                draft.content_row += 1;
                return;
            }
            if !down && draft.content_row > 0 {
                draft.content_row -= 1;
                return;
            }
        }
        if down {
            draft.next_field();
            if draft.field == CreateField::Content {
                draft.content_row = 0;
            }
        } else {
            draft.previous_field();
            if draft.field == CreateField::Content {
                draft.content_row = draft.content_len() - 1;
            }
        }
    }

    fn remove_content(&mut self) {
        let row = self.create.content_row;
        if let Some(title) = self.create.modrinth.remove(row) {
            self.status = format!("Removed {}", short(&title));
            self.create.dry_run_reviewed = false;
            let last = self.create.content_len() - 1;
            self.create.content_row = row.min(last);
        }
    }

    pub(super) fn handle_modrinth_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::CreateServer;
                self.status.clear();
            }
            KeyCode::Tab | KeyCode::BackTab => self.modrinth_picker.cycle_kind(),
            KeyCode::Down => self.modrinth_picker.next_result(),
            KeyCode::Up => self.modrinth_picker.previous_result(),
            KeyCode::Left => self.modrinth_picker.move_cursor_left(),
            KeyCode::Right => self.modrinth_picker.move_cursor_right(),
            KeyCode::Home => self.modrinth_picker.move_cursor_start(),
            KeyCode::End => self.modrinth_picker.move_cursor_end(),
            KeyCode::Backspace => self.modrinth_picker.backspace(),
            KeyCode::Delete => self.modrinth_picker.delete_char(),
            KeyCode::Enter => {
                if self.modrinth_picker.dirty || self.modrinth_picker.results.is_empty() {
                    self.search_modrinth();
                } else {
                    self.select_modrinth_result();
                }
            }
            KeyCode::Char(ch) => self.modrinth_picker.insert_char(ch),
            _ => {}
        }
    }

    /// Adding a pack replaces the version and loader, so drop mods that no longer fit.
    pub(super) fn apply_pack(&mut self, selected: SelectedModrinthProject) -> String {
        let title = short(&selected.title);
        let version = selected.selection.game_versions.first().cloned();
        let fabric = selected.selection.loaders.iter().any(|l| l == "fabric");
        self.create.modrinth.modpack = Some(selected);
        if let Some(version) = version.clone() {
            self.create.version = version;
        }
        if fabric {
            self.create.server_type = hostable_core::ServerType::Fabric;
        }
        let dropped = self.create.modrinth.retain_compatible(
            version.as_deref().unwrap_or(""),
            if fabric { "fabric" } else { "" },
        );
        self.create.content_row = 0;
        if dropped > 0 {
            format!(
                "Added {title}, removed {dropped} {}",
                if dropped == 1 { "mod" } else { "mods" }
            )
        } else {
            format!("Added {title}")
        }
    }

    /// From the Mods tab: a modpack replaces the whole server (version, loader,
    /// every mod), so it becomes a new server with the pack already chosen.
    pub(super) fn create_server_from_modpack(&mut self, hit: hostable_core::ModrinthSearchHit) {
        self.open_create_server();
        self.modrinth_picker = ModrinthPicker {
            kind: ModrinthPickKind::Modpack,
            results: vec![hit.clone()],
            ..ModrinthPicker::default()
        };
        self.select_modrinth_result();
        if self.mode == Mode::CreateServer {
            let slug = slugify(&hit.slug);
            self.create.name = if slug.is_empty() {
                slugify(&hit.title)
            } else {
                slug
            };
            self.create.sync_path();
            self.create.field = CreateField::Name;
            self.create.reset_cursor();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_folder_slugs() {
        assert_eq!(
            slugify("COBBLEVERSE - Pokemon Adventure"),
            "cobbleverse-pokemon-adventure"
        );
        assert_eq!(slugify("  Hello,  World!  "), "hello-world");
        assert_eq!(slugify("日本語"), "");
        assert_eq!(slugify(""), "");
        assert!(slugify(&"a".repeat(80)).len() <= 32);
    }
}
