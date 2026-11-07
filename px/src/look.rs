// Вид из JSON настроек (~/.config/ysnap.json): запасная палитра (accent, palette, light…) и язык.
// Основная палитра ysnap — из gtk.css (src/gtk.rs).

use crate::theme::{parse_hex, Theme, PRESETS};

#[derive(Clone)]
pub struct Look {
    pub theme: Theme,
    /// "" (по языку системы), "ru", "en", "pt-BR"
    pub lang: String,
    pub animations: bool,
    pub anim_speed: f32,
    pub icon_theme: String,
    pub blur: bool,
    /// значки программ пиксельными в палитре
    pub icon_pixel: bool,
    /// шрифты: шелла и отдельно файлов и просмотрщика ("" — как у шелла; у шелла "" — Tiny5)
    pub font_shell: String,
    pub font_files: String,
    pub font_view: String,
}

impl Look {
    /// вид из JSON в формате настроек шелла (экран входа читает копию, которую выкладывает шелл)
    pub fn from_json(v: &serde_json::Value, light: Option<bool>) -> Look {
        let f = |k: &str, d: f32| v.get(k).and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(d);
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let b = |k: &str, d: bool| v.get(k).and_then(|x| x.as_bool()).unwrap_or(d);
        let palette = s("palette");
        let accent = parse_hex(&s("accent")).unwrap_or_else(|| PRESETS.iter().find(|p| p.id == palette).unwrap_or(&PRESETS[0]).accent);
        crate::gfx::set_reveal(crate::gfx::Reveal::from_id(&s("reveal")), v.get("revealGrain").and_then(|x| x.as_i64()).unwrap_or(1) as i32);
        Look {
            theme: Theme::new(accent, f("tint", 1.0), f("glass", 0.55), f("lift", 0.0), f("inner", 1.0), light.unwrap_or_else(|| b("light", false))).apply(),
            lang: s("lang"),
            // лёгкий режим шелла (слабая или виртуальная машина) выключает анимации и здесь
            animations: b("animations", true) && !b("lite", false),
            anim_speed: f("animSpeed", 1.0),
            icon_theme: s("iconTheme"),
            blur: b("blurShell", true) && !b("lite", false),
            icon_pixel: b("iconPixel", true),
            font_shell: s("fontShell"),
            font_files: s("fontFiles"),
            font_view: s("fontView"),
        }
    }

    /// шрифт приложения: свой или как у шелла
    pub fn font(&self, app: &str) -> &str {
        let own = match app {
            "files" => &self.font_files,
            "view" => &self.font_view,
            _ => &self.font_shell,
        };
        if own.is_empty() {
            &self.font_shell
        } else {
            own
        }
    }

    /// длительность анимации с учётом скорости из настроек; 0 — анимации выключены
    pub fn dur(&self, ms: f32) -> f32 {
        if self.animations {
            ms * self.anim_speed.max(0.1)
        } else {
            0.0
        }
    }
}
