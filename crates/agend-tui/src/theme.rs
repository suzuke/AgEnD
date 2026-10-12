//! Local dashboard appearance. Native agent cells never pass through this map.
pub mod preferences;
use crate::{
    App,
    app::Screen,
    i18n::{Language, Text},
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

pub(crate) const BG: Color = Color::Rgb(24, 28, 39);
pub(crate) const SIDE: Color = Color::Rgb(20, 24, 35);
pub(crate) const FG: Color = Color::Rgb(204, 210, 226);
pub(crate) const MUTED: Color = Color::Rgb(129, 139, 161);
pub(crate) const BLUE: Color = Color::Rgb(164, 189, 242);
pub(crate) const SELECT: Color = Color::Rgb(37, 51, 76);
pub(crate) const HEADER: Color = Color::Rgb(32, 40, 56);
pub(crate) const AMBER: Color = Color::Rgb(223, 183, 129);
pub(crate) const GREEN: Color = Color::Rgb(143, 198, 176);
pub(crate) const RED: Color = Color::Rgb(228, 143, 143);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Dark,
    Terminal,
    Light,
    HighContrast,
}
impl Theme {
    pub const ALL: [Self; 4] = [Self::Dark, Self::Terminal, Self::Light, Self::HighContrast];
    pub fn id(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Terminal => "terminal",
            Self::Light => "light",
            Self::HighContrast => "high-contrast",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == value)
    }
    pub fn label(self, lang: Language) -> &'static str {
        lang.tr(match self {
            Self::Dark => Text::ThemeDark,
            Self::Terminal => Text::ThemeTerminal,
            Self::Light => Text::ThemeLight,
            Self::HighContrast => Text::ThemeContrast,
        })
    }
    fn colors(self) -> [Color; 10] {
        use Color::{Reset as D, Rgb as R};
        match self {
            Self::Dark => [BG, SIDE, FG, MUTED, BLUE, SELECT, HEADER, AMBER, GREEN, RED],
            Self::Terminal => [
                D,
                D,
                D,
                D,
                Color::Cyan,
                D,
                D,
                Color::Yellow,
                Color::Green,
                Color::Red,
            ],
            Self::Light => [
                R(246, 248, 252),
                R(233, 237, 245),
                R(32, 43, 61),
                R(76, 89, 109),
                R(32, 70, 135),
                R(211, 226, 250),
                R(223, 230, 242),
                R(125, 76, 0),
                R(20, 105, 70),
                R(170, 40, 45),
            ],
            Self::HighContrast => [
                Color::Black,
                Color::Black,
                Color::White,
                Color::White,
                Color::Cyan,
                Color::Blue,
                Color::Black,
                Color::Yellow,
                Color::LightGreen,
                Color::LightRed,
            ],
        }
    }
}

pub(crate) fn paint(buf: &mut Buffer, app: &App) {
    if matches!(app.screen(), Screen::Terminal { .. }) {
        return;
    }
    if app.theme_original.is_some() {
        dialog(buf, app);
    }
    let colors = app.theme.colors();
    let roles = [BG, SIDE, FG, MUTED, BLUE, SELECT, HEADER, AMBER, GREEN, RED];
    for cell in &mut buf.content {
        let selected = cell.bg == SELECT;
        let map = |color, fallback| {
            if color == Color::Reset {
                fallback
            } else {
                roles
                    .iter()
                    .position(|c| *c == color)
                    .map_or(color, |i| colors[i])
            }
        };
        cell.fg = map(cell.fg, colors[2]);
        cell.bg = map(cell.bg, colors[0]);
        if app.theme == Theme::Terminal && selected {
            cell.fg = Color::Reset;
            cell.bg = Color::Reset;
            cell.modifier.insert(Modifier::REVERSED);
        }
        if app.theme == Theme::HighContrast {
            cell.modifier.remove(Modifier::DIM);
        }
    }
}
fn dialog(buf: &mut Buffer, app: &App) {
    let area = buf.area;
    let width = area.width.min(64);
    let height = area.height.min(10);
    let panel = Rect::new(
        (area.width - width) / 2,
        (area.height - height) / 2,
        width,
        height,
    );
    let style = Style::default().fg(FG).bg(SIDE);
    for y in panel.y..panel.bottom() {
        for x in panel.x..panel.right() {
            buf[(x, y)].reset();
            buf[(x, y)].set_style(style);
        }
    }
    let mut line = |dy: u16, text: &str, selected: bool| {
        if dy >= height {
            return;
        }
        buf.set_stringn(
            panel.x,
            panel.y + dy,
            crate::ui::truncate(text, width as usize),
            width as usize,
            if selected {
                style.bg(SELECT).fg(BLUE)
            } else {
                style
            },
        );
    };
    line(0, app.lang.tr(Text::ThemeTitle), false);
    for (i, theme) in Theme::ALL.iter().enumerate() {
        line(
            i as u16 + 2,
            &format!(
                " {} [{}] {}",
                if *theme == app.theme { "›" } else { " " },
                i + 1,
                theme.label(app.lang)
            ),
            *theme == app.theme,
        );
    }
    line(7, app.lang.tr(Text::ThemeKeys), false);
}
