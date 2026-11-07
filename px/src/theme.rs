// Тема из одного акцента: все оттенки выводятся из него.
// Набор цветов уходит в ~/.local/state/ysnap/palette.json — по нему tools/theme.py
// перекрашивает Qt, KDE, GTK, kitty, fastfetch, рамки niri и рисует обои.

use crate::gfx::C;

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub accent: u32,
}

pub const PRESETS: &[Preset] = &[
    Preset { id: "blood", name: "кровь", accent: 0xc4161c },
    Preset { id: "ash", name: "пепел", accent: 0xa8a29c },
    Preset { id: "sakura", name: "сакура", accent: 0xd9577c },
    Preset { id: "amber", name: "янтарь", accent: 0xd98e1f },
    Preset { id: "venom", name: "яд", accent: 0x5fae34 },
    Preset { id: "ice", name: "лёд", accent: 0x3a9fd0 },
    Preset { id: "dusk", name: "сумрак", accent: 0x8a5cf0 },
];

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub bg: C,
    pub panel: C,
    pub surface: C,
    pub surface_hi: C,
    pub code_bg: C,
    pub border: C,
    pub dim: C,
    pub red: C,
    pub blood: C,
    pub glow: C,
    pub ember: C,
    pub selection: C,
    pub text: C,
    pub text_dim: C,
    pub text_faint: C,
    pub on_accent: C,
    /// самый контрастный текст (наведение, выбранное): белый в тёмной теме, почти чёрный в светлой
    pub text_hi: C,
    /// жёсткая пиксельная тень под панелями и кнопками
    pub shade: C,
    pub ok: C,
    pub warn: C,
    pub glass: f32,
    pub light: bool,
}

pub const WHITE: C = C(0xffffffff);
pub const CLEAR: C = C(0);

fn hsv(c: C) -> (f32, f32, f32) {
    let (r, g, b) = c.rgbf();
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

fn from_hsv(h: f32, s: f32, v: f32) -> C {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match (i as i32).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    C::from_f(r, g, b)
}

/// Qt.lighter / Qt.darker: яркость в HSV, лишнее — из насыщенности
pub fn scale_v(c: C, k: f32) -> C {
    let (h, mut s, mut v) = hsv(c);
    v *= k;
    if v > 1.0 {
        s = (s - (v - 1.0)).max(0.0);
        v = 1.0;
    }
    from_hsv(h, s, v)
}

pub fn lightness(c: C) -> f32 {
    let (r, g, b) = c.rgbf();
    (r.max(g).max(b) + r.min(g).min(b)) / 2.0
}

pub fn parse_hex(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    u32::from_str_radix(h, 16).ok()
}

impl Theme {
    /// lift — насколько поднять тёмные фоны к светлому графиту; inner — плотность карточек и
    /// графиков внутри панелей (0.2 — почти прозрачные, 1 — сплошные); light — светлая тема
    pub fn new(accent: u32, tint: f32, glass: f32, lift: f32, inner: f32, light: bool) -> Theme {
        if light {
            return Theme::light(accent, tint, glass, lift, inner);
        }
        let a = C::hex(accent);
        let light = C::hex(0x4a4242).lerp(a, 0.12 * tint);
        let lift = lift.clamp(0.0, 1.0) * 0.55;
        let mix = |base: u32, k: f32| C::hex(base).lerp(a, k).lerp(light, lift);
        let inner = inner.clamp(0.2, 1.0);
        let bg = mix(0x080707, 0.035 * tint);
        let panel = mix(0x0e0c0c, 0.055 * tint);
        Theme {
            bg,
            panel,
            surface: mix(0x161313, 0.08 * tint).alpha(inner),
            surface_hi: mix(0x1f1b1b, 0.12 * tint).alpha(inner.max(0.5)),
            code_bg: mix(0x050404, 0.02 * tint).alpha(inner),
            border: panel.lerp(a, 0.3),
            dim: panel.lerp(a, 0.45),
            red: a,
            blood: scale_v(a, 1.0 / 1.6),
            glow: scale_v(a, 1.0 / 1.15),
            ember: scale_v(a, 1.3),
            selection: bg.lerp(a, 0.55),
            text: mix(0xebe5e4, 0.05),
            text_dim: mix(0x8c8282, 0.12),
            text_faint: mix(0x5c5252, 0.1),
            on_accent: if lightness(a) > 0.6 { C::hex(0x0b0909) } else { WHITE },
            text_hi: WHITE,
            shade: C(0xb4000000),
            ok: C::hex(0x78c878),
            warn: C::hex(0xffaa28),
            glass,
            light: false,
        }
    }

    /// светлая тема: бумага вместо графита. Акцент темнеет, пока не станет читаться на светлом
    /// фоне (пепел, янтарь, лёд иначе сливаются), «ярче» здесь значит «глубже»: наведение
    /// уводит цвет в темноту, а не в белое. lift — насколько приглушить белизну к серому.
    fn light(accent: u32, tint: f32, glass: f32, lift: f32, inner: f32) -> Theme {
        let src = C::hex(accent);
        let mut a = src;
        for _ in 0..12 {
            if lightness(a) <= 0.46 {
                break;
            }
            a = scale_v(a, 0.9);
        }
        let grey = C::hex(0xb9b2ae).lerp(src, 0.1 * tint);
        let lift = lift.clamp(0.0, 1.0) * 0.45;
        let mix = |base: u32, k: f32| C::hex(base).lerp(src, k).lerp(grey, lift);
        let inner = inner.clamp(0.2, 1.0);
        let bg = mix(0xf4f0ed, 0.04 * tint);
        let panel = mix(0xebe6e2, 0.06 * tint);
        let text = C::hex(0x1c1717).lerp(a, 0.06);
        Theme {
            bg,
            panel,
            surface: mix(0xddd6d1, 0.08 * tint).alpha(inner),
            surface_hi: mix(0xd0c8c2, 0.12 * tint).alpha(inner.max(0.5)),
            code_bg: mix(0xfbf9f7, 0.02 * tint).alpha(inner),
            border: panel.lerp(a, 0.38),
            dim: panel.lerp(a, 0.55),
            red: a,
            blood: scale_v(a, 1.0 / 1.35),
            glow: scale_v(a, 1.0 / 1.1),
            ember: scale_v(a, 0.82),
            selection: bg.lerp(src, 0.3),
            text,
            text_dim: C::hex(0x5e5555).lerp(a, 0.1),
            text_faint: C::hex(0x958c89).lerp(a, 0.08),
            on_accent: if lightness(a) > 0.6 { C::hex(0x0b0909) } else { WHITE },
            text_hi: C::hex(0x000000),
            shade: text.alpha(0.32),
            ok: C::hex(0x2e8b3a),
            warn: C::hex(0xb86a00),
            glass,
            light: true,
        }
    }

    /// сделать тему текущей для процесса: то, что холсты берут сами (тень)
    pub fn apply(&self) -> Theme {
        crate::gfx::set_shade(self.shade);
        *self
    }

    /// фон панелей с матовостью
    pub fn glass_bg(&self) -> C {
        self.bg.alpha(self.glass)
    }
    pub fn glass_panel(&self) -> C {
        self.panel.alpha((self.glass + 0.08).min(1.0))
    }

    /// тона для пиксельных значков программ: обводка, затем от тёмного к светлому
    pub fn icon_ramp(&self) -> [u32; 5] {
        // значки одинаковы в обеих темах: тёмная обводка, светлый блик
        if self.light {
            [self.text.0 | 0xff000000, self.blood.0, self.red.0, scale_v(self.red, 1.3).0, self.code_bg.0 | 0xff000000]
        } else {
            [self.code_bg.0, self.blood.0, self.red.0, self.ember.0, self.text.0]
        }
    }

    pub fn palette_json(&self) -> String {
        let pairs = [
            ("bg", self.bg),
            ("panel", self.panel),
            ("surface", self.surface),
            ("surfaceHi", self.surface_hi),
            ("codeBg", self.code_bg),
            ("border", self.border),
            ("dim", self.dim),
            ("accent", self.red),
            ("deep", self.blood),
            ("glow", self.glow),
            ("bright", self.ember),
            ("selection", self.selection),
            ("text", self.text),
            ("textDim", self.text_dim),
            ("textFaint", self.text_faint),
            ("onAccent", self.on_accent),
            ("textHi", self.text_hi),
            ("ok", self.ok),
            ("warn", self.warn),
        ];
        let mut body: Vec<String> = pairs.iter().map(|(k, c)| format!("  \"{k}\": \"{}\"", c.hex_str())).collect();
        body.push(format!("  \"mode\": \"{}\"", if self.light { "light" } else { "dark" }));
        format!("{{\n{}\n}}\n", body.join(",\n"))
    }
}
