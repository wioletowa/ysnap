// Вид из системы: цвета — из @define-color в gtk.css (GTK 4, потом 3; с @import, так Noctalia,
// matugen и прочие генераторы кладут палитру в соседний файл), шрифт — gtk-font-name из
// settings.ini, иначе gsettings. Ничего не нашлось — встроенная тема и sans-serif.

use crate::gfx::C;
use crate::theme::{lightness, parse_hex, Theme};
use std::collections::HashMap;
use std::path::Path;

/// @define-color имя значение — из файла и того, что он импортирует (глубина ограничена)
fn defines(p: &Path, depth: u32, out: &mut HashMap<String, String>) {
    let Ok(css) = std::fs::read_to_string(p) else { return };
    for l in css.lines().map(str::trim) {
        if let Some(rest) = l.strip_prefix("@import") {
            let name = rest.trim().trim_start_matches("url(").trim_end_matches(';').trim_end_matches(')').trim_matches(|c| c == '"' || c == '\'');
            let path = name.strip_prefix("file://").unwrap_or(name);
            let path = if Path::new(path).is_absolute() { Path::new(path).to_path_buf() } else { p.parent().unwrap_or(Path::new(".")).join(path) };
            if depth < 4 {
                defines(&path, depth + 1, out);
            }
        } else if let Some(rest) = l.strip_prefix("@define-color") {
            let mut it = rest.trim().trim_end_matches(';').splitn(2, char::is_whitespace);
            if let (Some(k), Some(v)) = (it.next(), it.next()) {
                out.insert(k.to_string(), v.trim().to_string());
            }
        }
    }
}

/// значение цвета: #rrggbb, #rgb или @ссылка на другой цвет
fn color(map: &HashMap<String, String>, keys: &[&str]) -> Option<C> {
    keys.iter().find_map(|k| {
        let mut v = map.get(*k)?.as_str();
        for _ in 0..8 {
            match v.strip_prefix('@') {
                Some(r) => v = map.get(r)?.as_str(),
                None => break,
            }
        }
        let h = v.trim_start_matches('#');
        let h = if h.len() == 3 { h.chars().flat_map(|c| [c, c]).collect() } else { h.chars().take(6).collect::<String>() };
        parse_hex(&h).map(C::hex)
    })
}

/// тема из gtk.css; None — палитры там нет
pub fn theme() -> Option<Theme> {
    let cfg = ysnap_px::paths::config_dir();
    let mut map = HashMap::new();
    for d in ["gtk-4.0", "gtk-3.0"] {
        defines(&cfg.join(d).join("gtk.css"), 0, &mut map);
        if !map.is_empty() {
            break;
        }
    }
    let accent = color(&map, &["accent_bg_color", "accent_color", "theme_selected_bg_color"])?;
    let bg = color(&map, &["window_bg_color", "theme_bg_color"])?;
    let fg = color(&map, &["window_fg_color", "theme_fg_color"]).unwrap_or(if lightness(bg) > 0.5 { C::hex(0x1c1717) } else { C::hex(0xebe5e4) });
    let panel = color(&map, &["popover_bg_color", "card_bg_color", "headerbar_bg_color"]).unwrap_or(bg.lerp(fg, 0.06));
    let light = lightness(bg) > 0.5;
    // остальное (тени, ok/warn, «глубокий» и «яркий» акцент) — как у встроенной темы из этого акцента
    let mut t = Theme::new(accent.0 & 0xffffff, 1.0, 0.55, 0.0, 1.0, light);
    t.bg = bg;
    t.panel = panel;
    t.surface = panel.lerp(fg, 0.06);
    t.surface_hi = panel.lerp(fg, 0.12);
    t.code_bg = bg.lerp(C::hex(if light { 0xffffff } else { 0x000000 }), 0.3);
    t.border = panel.lerp(accent, 0.3);
    t.dim = panel.lerp(accent, 0.45);
    t.red = accent;
    t.selection = bg.lerp(accent, 0.45);
    t.text = fg;
    t.text_dim = fg.lerp(bg, 0.4);
    t.text_faint = fg.lerp(bg, 0.6);
    if let Some(c) = color(&map, &["accent_fg_color", "theme_selected_fg_color"]) {
        t.on_accent = c;
    }
    if let Some(c) = color(&map, &["success_color", "success_bg_color"]) {
        t.ok = c;
    }
    if let Some(c) = color(&map, &["warning_color", "warning_bg_color"]) {
        t.warn = c;
    }
    Some(t.apply())
}

/// семейство шрифта интерфейса: «Sans 11» → «Sans»
pub fn font() -> String {
    let cfg = ysnap_px::paths::config_dir();
    let from_ini = ["gtk-4.0", "gtk-3.0"].iter().find_map(|d| {
        let s = std::fs::read_to_string(cfg.join(d).join("settings.ini")).ok()?;
        s.lines().find_map(|l| l.trim().strip_prefix("gtk-font-name").map(|v| v.trim_start_matches([' ', '=']).trim().to_string()))
    });
    let name = from_ini.filter(|s| !s.is_empty()).or_else(|| {
        let o = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "font-name"]).stderr(std::process::Stdio::null()).output().ok()?;
        Some(String::from_utf8_lossy(&o.stdout).trim().trim_matches('\'').to_string()).filter(|s| !s.is_empty())
    });
    family(name.as_deref().unwrap_or(""))
}

fn family(name: &str) -> String {
    let mut words: Vec<&str> = name.split_whitespace().collect();
    if words.last().is_some_and(|w| w.parse::<f32>().is_ok()) {
        words.pop();
    }
    let f = words.join(" ").trim_end_matches(',').to_string();
    if f.is_empty() {
        "sans-serif".into()
    } else {
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_family_drops_size() {
        assert_eq!(family("Sans 11"), "Sans");
        assert_eq!(family("Inter Display Bold 10.5"), "Inter Display Bold");
        assert_eq!(family(""), "sans-serif");
    }

    #[test]
    fn colors_follow_references_and_imports() {
        let dir = std::env::temp_dir().join(format!("ysnap-gtk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("gtk.css"), "@import url(\"pal.css\");\n").unwrap();
        std::fs::write(dir.join("pal.css"), "@define-color accent_bg_color #ffb3af;\n@define-color window_bg_color #1a1111;\n@define-color sel @accent_bg_color;\n@define-color short #abc;\n").unwrap();
        let mut m = HashMap::new();
        defines(&dir.join("gtk.css"), 0, &mut m);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(color(&m, &["sel"]).map(|c| c.0 & 0xffffff), Some(0xffb3af));
        assert_eq!(color(&m, &["nope", "window_bg_color"]).map(|c| c.0 & 0xffffff), Some(0x1a1111));
        assert_eq!(color(&m, &["short"]).map(|c| c.0 & 0xffffff), Some(0xaabbcc));
    }
}
