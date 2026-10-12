//! Export actual TestBackend cells as HTML for reviewing the dashboard layout.
//! Scripted protocol data only; no daemon or model is started.
use agend_tui::{App, i18n::Language, render_buffer, source::scripted::ScriptedSource};
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent},
    style::{Color, Modifier},
};
use unicode_width::UnicodeWidthStr;

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn color(value: Color, fallback: &str) -> String {
    match value {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => fallback.into(),
    }
}
fn main() {
    println!(
        "<!doctype html><meta charset=utf-8><title>AgEnD native cells</title><style>body{{background:#10141c;color:#ccd2e2;font:14px monospace}}section{{position:relative;margin:16px 0}}span{{position:absolute;white-space:pre;font:14px/22px Menlo,monospace}}h2{{font:14px monospace}}</style>"
    );
    for width in [140, 110] {
        let (source, _) = ScriptedSource::demo();
        let mut app = App::new(Box::new(source), Language::ZhTw);
        for attention in [false, true] {
            if attention {
                app.key(KeyEvent::from(KeyCode::Char('!')));
            }
            let (buffer, _) = render_buffer(&mut app, width, 34);
            println!(
                "<h2>{width} columns / {} / actual ratatui cells, scripted data</h2><section style='width:{}px;height:748px'>",
                if attention { "needs you" } else { "overview" },
                width * 9
            );
            for y in 0..34 {
                let mut x = 0;
                while x < width {
                    let cell = &buffer[(x, y)];
                    let n = cell.symbol().width().max(1) as u16;
                    println!(
                        "<span style='left:{}px;top:{}px;width:{}px;background:{};color:{};font-weight:{}'>{}</span>",
                        x * 9,
                        y * 22,
                        n * 9,
                        color(cell.bg, "#181c27"),
                        color(cell.fg, "#ccd2e2"),
                        if cell.modifier.contains(Modifier::BOLD) {
                            "bold"
                        } else {
                            "normal"
                        },
                        escape(cell.symbol())
                    );
                    x += n;
                }
            }
            println!("</section>");
        }
    }
}
