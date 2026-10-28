//! The look: a colour theme (Catppuccin Mocha unless the config names another), rounded panes, and the tab rows shared by the
//! header and every view. Under `NO_COLOR` the frame keeps only bold, underline and
//! reversed text (see `strip_colour`).

use crate::app::Action;
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

/// The colours of a theme, by role.
pub struct Palette {
    pub name: &'static str,
    pub base: Color,
    pub surface: Color,
    pub border: Color,
    pub overlay: Color,
    pub subtext: Color,
    pub text: Color,
    /// Titles, the shown tab and keys.
    pub accent: Color,
    pub peach: Color,
    pub teal: Color,
    pub blue: Color,
    pub green: Color,
    pub red: Color,
    pub yellow: Color,
    pub pink: Color,
    pub sky: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// The themes to choose from; the first is the default.
pub const THEMES: [Palette; 7] = [
    Palette {
        name: "catppuccin-mocha",
        base: rgb(0x1e1e2e),
        surface: rgb(0x363a4f),
        border: rgb(0x45475a),
        overlay: rgb(0x6c7086),
        subtext: rgb(0xa6adc8),
        text: rgb(0xcdd6f4),
        accent: rgb(0xcba6f7),
        peach: rgb(0xfab387),
        teal: rgb(0x94e2d5),
        blue: rgb(0x89b4fa),
        green: rgb(0xa6e3a1),
        red: rgb(0xf38ba8),
        yellow: rgb(0xf9e2af),
        pink: rgb(0xf5c2e7),
        sky: rgb(0x89dceb),
    },
    Palette {
        name: "catppuccin-latte",
        base: rgb(0xeff1f5),
        surface: rgb(0xccd0da),
        border: rgb(0xbcc0cc),
        overlay: rgb(0x8c8fa1),
        subtext: rgb(0x6c6f85),
        text: rgb(0x4c4f69),
        accent: rgb(0x8839ef),
        peach: rgb(0xfe640b),
        teal: rgb(0x179299),
        blue: rgb(0x1e66f5),
        green: rgb(0x40a02b),
        red: rgb(0xd20f39),
        yellow: rgb(0xdf8e1d),
        pink: rgb(0xea76cb),
        sky: rgb(0x04a5e5),
    },
    Palette {
        name: "tokyo-night",
        base: rgb(0x1a1b26),
        surface: rgb(0x292e42),
        border: rgb(0x3b4261),
        overlay: rgb(0x565f89),
        subtext: rgb(0xa9b1d6),
        text: rgb(0xc0caf5),
        accent: rgb(0xbb9af7),
        peach: rgb(0xff9e64),
        teal: rgb(0x73daca),
        blue: rgb(0x7aa2f7),
        green: rgb(0x9ece6a),
        red: rgb(0xf7768e),
        yellow: rgb(0xe0af68),
        pink: rgb(0xff007c),
        sky: rgb(0x7dcfff),
    },
    Palette {
        name: "dracula",
        base: rgb(0x282a36),
        surface: rgb(0x44475a),
        border: rgb(0x44475a),
        overlay: rgb(0x6272a4),
        subtext: rgb(0xbfc2d0),
        text: rgb(0xf8f8f2),
        accent: rgb(0xbd93f9),
        peach: rgb(0xffb86c),
        teal: rgb(0x8be9fd),
        blue: rgb(0x82aaff),
        green: rgb(0x50fa7b),
        red: rgb(0xff5555),
        yellow: rgb(0xf1fa8c),
        pink: rgb(0xff79c6),
        sky: rgb(0x8be9fd),
    },
    Palette {
        name: "gruvbox-dark",
        base: rgb(0x282828),
        surface: rgb(0x3c3836),
        border: rgb(0x504945),
        overlay: rgb(0x928374),
        subtext: rgb(0xbdae93),
        text: rgb(0xebdbb2),
        accent: rgb(0xfabd2f),
        peach: rgb(0xfe8019),
        teal: rgb(0x8ec07c),
        blue: rgb(0x83a598),
        green: rgb(0xb8bb26),
        red: rgb(0xfb4934),
        yellow: rgb(0xfabd2f),
        pink: rgb(0xd3869b),
        sky: rgb(0x83a598),
    },
    Palette {
        name: "nord",
        base: rgb(0x2e3440),
        surface: rgb(0x3b4252),
        border: rgb(0x4c566a),
        overlay: rgb(0x7b88a1),
        subtext: rgb(0xd8dee9),
        text: rgb(0xeceff4),
        accent: rgb(0x88c0d0),
        peach: rgb(0xd08770),
        teal: rgb(0x8fbcbb),
        blue: rgb(0x81a1c1),
        green: rgb(0xa3be8c),
        red: rgb(0xbf616a),
        yellow: rgb(0xebcb8b),
        pink: rgb(0xb48ead),
        sky: rgb(0x88c0d0),
    },
    // The terminal's own colours and background, for an already themed terminal.
    Palette {
        name: "terminal",
        base: Color::Reset,
        surface: Color::DarkGray,
        border: Color::DarkGray,
        overlay: Color::DarkGray,
        subtext: Color::Gray,
        text: Color::Reset,
        accent: Color::Magenta,
        peach: Color::LightRed,
        teal: Color::Cyan,
        blue: Color::Blue,
        green: Color::Green,
        red: Color::Red,
        yellow: Color::Yellow,
        pink: Color::LightMagenta,
        sky: Color::LightCyan,
    },
];

static CURRENT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The theme in use.
pub fn p() -> &'static Palette {
    &THEMES[CURRENT.load(std::sync::atomic::Ordering::Relaxed)]
}

/// Switches to the theme called `name`; `false` (and no change) if there is none.
pub fn set(name: &str) -> bool {
    match THEMES.iter().position(|theme| theme.name == name) {
        Some(index) => {
            CURRENT.store(index, std::sync::atomic::Ordering::Relaxed);
            true
        }
        None => false,
    }
}

/// Every theme's name, for messages and the picker.
pub fn names() -> Vec<&'static str> {
    THEMES.iter().map(|theme| theme.name).collect()
}

/// The accent each tab's key takes, in order, so neighbouring tabs differ.
fn accents() -> [Color; 6] {
    let p = p();
    [p.peach, p.teal, p.blue, p.green, p.pink, p.yellow]
}

/// Text on the app's background.
pub fn base() -> Style {
    Style::default().fg(p().text).bg(p().base)
}

pub fn muted() -> Style {
    Style::default().fg(p().subtext)
}

pub fn fg(colour: Color) -> Style {
    Style::default().fg(colour)
}

/// The selected row. Reversed as well, so it still shows when colour is stripped.
pub fn selected() -> Style {
    if plain() || p().base == Color::Reset {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
            .bg(p().surface)
            .add_modifier(Modifier::BOLD)
    }
}

/// A pane: rounded border, mauve title, the app's background inside.
pub fn pane(title: impl Into<String>) -> Block<'static> {
    let title = title.into();
    let title = title.trim();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(fg(p().border))
        .style(base());
    if title.is_empty() {
        block
    } else {
        block.title(Span::styled(
            format!(" {title} "),
            fg(p().accent).add_modifier(Modifier::BOLD),
        ))
    }
}

/// A row of tabs (`key Label`, `active`, its action) from `area.x + 1`; returns each
/// drawn tab's click target. Tabs that do not fit are left off.
pub fn tabs(
    frame: &mut Frame,
    area: Rect,
    tabs: &[(String, &str, bool, Action)],
) -> Vec<(Rect, Action)> {
    let mut spans = Vec::new();
    let mut hits = Vec::new();
    let mut x = area.x + 1;
    for (index, (key, label, active, action)) in tabs.iter().enumerate() {
        let width = (key.chars().count() + 1 + label.chars().count()) as u16;
        if x + width > area.right() {
            break;
        }
        let accent = accents()[index % 6];
        let (key_style, label_style) = if *active {
            let on = fg(p().accent).add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
            (on, on)
        } else {
            (fg(accent).add_modifier(Modifier::BOLD), muted())
        };
        spans.push(Span::styled(key.clone(), key_style));
        spans.push(Span::styled(format!(" {label}"), label_style));
        spans.push(Span::raw("   "));
        hits.push((Rect::new(x, area.y, width, 1), action.clone()));
        x += width + 3;
    }
    let row = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(1),
        ..area
    };
    frame.render_widget(Paragraph::new(Line::from(spans)), row);
    hits
}

/// The header: the app's badge, then the views as pills (the shown one filled), and
/// `right` (the account) at the right edge.
pub fn header(
    frame: &mut Frame,
    area: Rect,
    views: &[(char, &str, bool, Action)],
    right: &str,
) -> Vec<(Rect, Action)> {
    let mut spans = vec![
        Span::styled(
            " ghtuix ",
            Style::default()
                .fg(p().base)
                .bg(p().accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    let mut hits = Vec::new();
    let mut x = area.x + 9;
    let room = area
        .right()
        .saturating_sub(right.chars().count() as u16 + 1);
    for (index, (key, label, active, action)) in views.iter().enumerate() {
        let width = (label.chars().count() + 4) as u16; // " 1 Inbox "
        if x + width > room {
            break;
        }
        if *active {
            spans.push(Span::styled(
                format!(" {key} {label} "),
                Style::default()
                    .fg(p().base)
                    .bg(p().accent)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ));
        } else {
            let accent = accents()[index % 6];
            spans.push(Span::styled(
                format!(" {key}"),
                fg(accent).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(format!(" {label} "), muted()));
        }
        spans.push(Span::raw(" "));
        hits.push((Rect::new(x, area.y, width, 1), action.clone()));
        x += width + 1;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    let width = right.chars().count() as u16;
    if width + 1 < area.width {
        let at = Rect::new(area.right() - width - 1, area.y, width, 1);
        frame.render_widget(Paragraph::new(Span::styled(right.to_owned(), muted())), at);
    }
    hits
}

/// `NO_COLOR` (any non-empty value) asks for no colour at all.
fn plain() -> bool {
    static PLAIN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PLAIN.get_or_init(|| std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()))
}

/// Under `NO_COLOR`, takes the colours out of a drawn frame (bold, underline and
/// reversed stay).
pub fn strip_colour(buffer: &mut Buffer) {
    if !plain() {
        return;
    }
    for cell in buffer.content.iter_mut() {
        cell.set_fg(Color::Reset);
        cell.set_bg(Color::Reset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn each_tab_is_drawn_where_its_click_target_is() {
        let mut terminal = Terminal::new(TestBackend::new(60, 1)).unwrap();
        let tabs = vec![
            ("a".to_string(), "All", true, Action::Refresh),
            ("c".to_string(), "Created", false, Action::Quit),
        ];
        let mut hits = Vec::new();
        terminal
            .draw(|frame| hits = super::tabs(frame, frame.area(), &tabs))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = |rect: Rect| -> String {
            (rect.x..rect.right())
                .map(|x| buffer[(x, 0)].symbol().to_owned())
                .collect()
        };
        assert_eq!(text(hits[0].0), "a All");
        assert_eq!(text(hits[1].0), "c Created");
    }

    #[test]
    fn every_theme_has_a_unique_name_and_the_default_is_first() {
        let names = names();
        assert_eq!(names[0], "catppuccin-mocha");
        let mut unique = names.clone();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
    }
}
