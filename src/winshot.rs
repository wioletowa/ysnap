// Снимок окна «как в macOS»: окно (niri msg action screenshot-window — чистое, без того, что под
// ним), вокруг — его украшения из конфига niri (рамка, кольцо фокуса, скругление углов) и мягкая
// тень на прозрачном поле. Тень — из layout { shadow }, а если она там выключена — со значениями
// niri по умолчанию: без неё снимок не похож на окно. Всё считается в пикселях снимка (HiDPI).
//
// Конфиг читается маленьким разбором KDL (узлы, аргументы, свойства, блоки, комментарии,
// include): только то, что нужно — layout { border focus-ring shadow } и window-rule (match /
// exclude по app-id и title, geometry-corner-radius и те же три блока).

use crate::app::{App, Msg};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct Win {
    pub id: u64,
    pub app_id: String,
    pub title: String,
    /// логический размер окна (снимок — в физических пикселях)
    pub w: f64,
}


fn win_of(v: &serde_json::Value) -> Option<Win> {
    Some(Win {
        id: v["id"].as_u64()?,
        app_id: v["app_id"].as_str().unwrap_or("").to_string(),
        title: v["title"].as_str().unwrap_or("").to_string(),
        w: v["layout"]["window_size"][0].as_f64().unwrap_or(0.0),
    })
}

/// окно в фокусе niri
pub fn focused() -> Option<Win> {
    win_of(&crate::niri::request("FocusedWindow"))
}

fn by_id(id: u64) -> Option<Win> {
    crate::niri::request("Windows").as_array()?.iter().find(|w| w["id"].as_u64() == Some(id)).and_then(win_of)
}

// ── разбор KDL ─────────────────────────────────────────────────────────────
#[derive(Debug, Default, Clone)]
pub(crate) struct Node {
    pub name: String,
    pub args: Vec<String>,
    pub props: Vec<(String, String)>,
    pub kids: Vec<Node>,
}

impl Node {
    fn prop(&self, k: &str) -> Option<&str> {
        self.props.iter().rev().find(|p| p.0 == k).map(|p| p.1.as_str())
    }
    fn num(&self) -> Option<f32> {
        self.args.first()?.parse().ok()
    }
}

#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Open,
    Close,
    End,
}

fn tokens(src: &str) -> Vec<Tok> {
    let c: Vec<char> = src.chars().collect();
    let mut v = vec![];
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '/' if c.get(i + 1) == Some(&'/') => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            '/' if c.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            '\n' | ';' => {
                v.push(Tok::End);
                i += 1;
            }
            '{' => {
                v.push(Tok::Open);
                i += 1;
            }
            '}' => {
                v.push(Tok::Close);
                i += 1;
            }
            ch if ch.is_whitespace() => i += 1,
            _ => {
                // слово: голое или со строками в кавычках внутри (app-id="kitty")
                let mut w = String::new();
                while i < c.len() && !c[i].is_whitespace() && !matches!(c[i], '{' | '}' | ';') {
                    // сырая строка r#"…"# (регулярки в window-rule): без экранирования
                    let hashes = c[i + 1..].iter().take_while(|ch| **ch == '#').count();
                    if c[i] == 'r' && (w.is_empty() || w.ends_with('=')) && c.get(i + 1 + hashes) == Some(&'"') {
                        i += 2 + hashes;
                        let end: Vec<char> = std::iter::once('"').chain(std::iter::repeat_n('#', hashes)).collect();
                        while i < c.len() && !c[i..].starts_with(&end) {
                            w.push(c[i]);
                            i += 1;
                        }
                        i += end.len();
                        continue;
                    }
                    if c[i] == '"' {
                        i += 1;
                        while i < c.len() && c[i] != '"' {
                            if c[i] == '\\' && i + 1 < c.len() {
                                i += 1;
                            }
                            w.push(c[i]);
                            i += 1;
                        }
                        i += 1;
                    } else {
                        w.push(c[i]);
                        i += 1;
                    }
                }
                v.push(Tok::Word(w));
            }
        }
    }
    v
}

fn nodes(t: &[Tok], i: &mut usize) -> Vec<Node> {
    let mut out = vec![];
    let mut cur: Option<Node> = None;
    while *i < t.len() {
        match &t[*i] {
            Tok::Word(w) => {
                match &mut cur {
                    None => cur = Some(Node { name: w.clone(), ..Default::default() }),
                    Some(n) => match w.split_once('=') {
                        Some((k, v)) => n.props.push((k.to_string(), v.to_string())),
                        None => n.args.push(w.clone()),
                    },
                }
                *i += 1;
            }
            Tok::Open => {
                *i += 1;
                let kids = nodes(t, i);
                if let Some(n) = &mut cur {
                    n.kids = kids;
                }
            }
            Tok::Close => {
                *i += 1;
                break;
            }
            Tok::End => {
                *i += 1;
                out.extend(cur.take());
            }
        }
    }
    out.extend(cur.take());
    out
}

pub(crate) fn parse(src: &str) -> Vec<Node> {
    nodes(&tokens(src), &mut 0)
}

/// узлы файла с раскрытыми include (на их месте, как у niri: позже — сильнее)
fn load(p: &Path, depth: u32) -> Vec<Node> {
    let Ok(src) = std::fs::read_to_string(p) else { return vec![] };
    let mut out = vec![];
    for n in parse(&src) {
        if n.name == "include" {
            if let (Some(f), true) = (n.args.first(), depth < 8) {
                let f = ysnap_px::paths::expand(f);
                let f = if f.is_absolute() { f } else { p.parent().unwrap_or(Path::new(".")).join(f) };
                out.extend(load(&f, depth + 1));
            }
        } else {
            out.push(n);
        }
    }
    out
}

// ── украшения ──────────────────────────────────────────────────────────────
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub on: bool,
    pub width: f32,
    pub color: [u8; 4],
    /// active-gradient: от, до, угол (как у CSS: 0 — вверх, 90 — вправо, 180 — вниз); важнее цвета
    pub grad: Option<([u8; 4], [u8; 4], f32)>,
}

impl Line {
    /// цвет в точке контура r (x, y, w, h)
    fn at(&self, px: f32, py: f32, r: (f32, f32, f32, f32, f32)) -> [u8; 4] {
        let Some((a, b, ang)) = self.grad else { return self.color };
        let (x, y, w, h, _) = r;
        let t = ang.to_radians();
        let (dx, dy) = (t.sin(), -t.cos());
        let half = (w / 2.0 * dx).abs() + (h / 2.0 * dy).abs();
        let k = (((px - x - w / 2.0) * dx + (py - y - h / 2.0) * dy) / half.max(1e-3) * 0.5 + 0.5).clamp(0.0, 1.0);
        std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * k).round() as u8)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub softness: f32,
    pub spread: f32,
    pub offset: (f32, f32),
    pub color: [u8; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Deco {
    pub ring: Line,
    pub border: Line,
    pub radius: f32,
    pub shadow: Shadow,
}

/// значения niri по умолчанию
impl Default for Deco {
    fn default() -> Deco {
        Deco {
            ring: Line { on: true, width: 4.0, color: [0x7f, 0xc8, 0xff, 255], grad: None },
            border: Line { on: false, width: 4.0, color: [0xff, 0xc8, 0x7f, 255], grad: None },
            radius: 0.0,
            shadow: Shadow { softness: 30.0, spread: 5.0, offset: (0.0, 5.0), color: [0, 0, 0, 0x77] },
        }
    }
}

/// #rgb, #rgba, #rrggbb, #rrggbbaa
pub(crate) fn rgba(s: &str) -> Option<[u8; 4]> {
    let h = s.trim().strip_prefix('#')?;
    let h: String = if h.len() <= 4 { h.chars().flat_map(|c| [c, c]).collect() } else { h.to_string() };
    let b = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some([b(0)?, b(2)?, b(4)?, 255]),
        8 => Some([b(0)?, b(2)?, b(4)?, b(6)?]),
        _ => None,
    }
}

fn line(l: &mut Line, n: &Node) {
    for k in &n.kids {
        match k.name.as_str() {
            "on" => l.on = true,
            "off" => l.on = false,
            "width" => l.width = k.num().unwrap_or(l.width),
            "active-color" => l.color = k.args.first().and_then(|c| rgba(c)).unwrap_or(l.color),
            "active-gradient" => {
                if let (Some(a), Some(b)) = (k.prop("from").and_then(rgba), k.prop("to").and_then(rgba)) {
                    l.grad = Some((a, b, k.prop("angle").and_then(|v| v.parse().ok()).unwrap_or(180.0)));
                }
            }
            _ => {}
        }
    }
}

/// тень рисуется всегда (без неё снимок не похож на окно): on/off не важны, берём только значения
fn shadow(s: &mut Shadow, n: &Node) {
    for k in &n.kids {
        match k.name.as_str() {
            "softness" => s.softness = k.num().unwrap_or(s.softness),
            "spread" => s.spread = k.num().unwrap_or(s.spread),
            "offset" => {
                let f = |p: &str, d: f32| k.prop(p).and_then(|v| v.parse().ok()).unwrap_or(d);
                s.offset = (f("x", s.offset.0), f("y", s.offset.1));
            }
            "color" => s.color = k.args.first().and_then(|c| rgba(c)).unwrap_or(s.color),
            _ => {}
        }
    }
}

/// match / exclude: все свойства сходятся; незнакомое свойство (at-startup, is-floating…) — нет,
/// кроме is-focused=true и is-active=true (снимаем окно в фокусе)
fn matches(m: &Node, w: &Win) -> bool {
    m.props.iter().all(|(k, v)| match k.as_str() {
        "app-id" => crate::re::is_match(v, &w.app_id),
        "title" => crate::re::is_match(v, &w.title),
        "is-focused" | "is-active" => v == "true",
        _ => false,
    })
}

impl Deco {
    pub fn from_nodes(ns: &[Node], w: &Win) -> Deco {
        let mut d = Deco::default();
        let apply = |d: &mut Deco, n: &Node| match n.name.as_str() {
            "border" => line(&mut d.border, n),
            "focus-ring" => line(&mut d.ring, n),
            "shadow" => shadow(&mut d.shadow, n),
            _ => {}
        };
        for n in ns {
            match n.name.as_str() {
                "layout" => n.kids.iter().for_each(|k| apply(&mut d, k)),
                "window-rule" => {
                    let ms: Vec<&Node> = n.kids.iter().filter(|k| k.name == "match").collect();
                    let hit = (ms.is_empty() || ms.iter().any(|m| matches(m, w))) && !n.kids.iter().any(|k| k.name == "exclude" && matches(k, w));
                    if !hit {
                        continue;
                    }
                    for k in &n.kids {
                        if k.name == "geometry-corner-radius" {
                            // один радиус или четыре (берём наибольший)
                            d.radius = k.args.iter().filter_map(|a| a.parse::<f32>().ok()).fold(0.0, f32::max);
                        } else {
                            apply(&mut d, k);
                        }
                    }
                }
                _ => {}
            }
        }
        d
    }

    pub fn load(w: &Win) -> Deco {
        let p = std::env::var_os("NIRI_CONFIG").map(PathBuf::from).unwrap_or_else(|| ysnap_px::paths::config_dir().join("niri/config.kdl"));
        Deco::from_nodes(&load(&p, 0), w)
    }
}

// ── сборка картинки ────────────────────────────────────────────────────────
/// покрытие пикселя скруглённым прямоугольником (x, y, w, h, r) — по расстоянию до края, со
/// сглаживанием в один пиксель
fn cover(px: f32, py: f32, r: (f32, f32, f32, f32, f32)) -> f32 {
    let (x, y, w, h, rad) = r;
    let rad = rad.min(w / 2.0).min(h / 2.0).max(0.0);
    let qx = (px - (x + w / 2.0)).abs() - (w / 2.0 - rad);
    let qy = (py - (y + h / 2.0)).abs() - (h / 2.0 - rad);
    let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - rad;
    (0.5 - d).clamp(0.0, 1.0)
}

/// «поверх» для прямой (не умноженной) альфы
fn over(dst: &mut [u8], c: [u8; 3], a: f32) {
    if a <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let oa = a + da * (1.0 - a);
    for i in 0..3 {
        let v = (c[i] as f32 * a + dst[i] as f32 * da * (1.0 - a)) / oa.max(1e-6);
        dst[i] = v.round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (oa * 255.0).round() as u8;
}

/// окно (физические пиксели), украшения в логических точках, k — пикселей на точку
pub fn compose(win: &image::RgbaImage, d: &Deco, k: f32) -> image::RgbaImage {
    let (ww, wh) = (win.width() as f32, win.height() as f32);
    let bw = if d.border.on { d.border.width * k } else { 0.0 };
    let rw = if d.ring.on { d.ring.width * k } else { 0.0 };
    let deco = bw + rw;
    let s = &d.shadow;
    let (soft, spread, ox, oy) = (s.softness * k, s.spread * k, s.offset.0 * k, s.offset.1 * k);
    let pad = ((soft + spread) * 1.5 + ox.abs().max(oy.abs())).max(16.0 * k).ceil() + deco.ceil();
    let (w, h) = ((ww + 2.0 * pad) as usize, (wh + 2.0 * pad) as usize);
    let r = d.radius * k;
    let rect = |grow: f32, dx: f32, dy: f32| (pad - grow + dx, pad - grow + dy, ww + 2.0 * grow, wh + 2.0 * grow, if r > 0.0 { r + grow } else { 0.0 });
    let mut out = image::RgbaImage::new(w as u32, h as u32);
    // тень: маска внешнего контура (с расширением и сдвигом), размытая
    let sr = rect(deco + spread, ox, oy);
    let mut a = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            a[y * w + x] = cover(x as f32 + 0.5, y as f32 + 0.5, sr);
        }
    }
    // сигма = мягкость / 2, как blur у CSS-тени
    crate::blur::box3(&mut a, w, h, crate::blur::radius(soft / 2.0));
    let sc = [s.color[0], s.color[1], s.color[2]];
    let sa = s.color[3] as f32 / 255.0;
    let (ring_r, border_r, win_r) = (rect(deco, 0.0, 0.0), rect(bw, 0.0, 0.0), rect(0.0, 0.0, 0.0));
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let p = out.get_pixel_mut(x as u32, y as u32);
            over(&mut p.0, sc, a[y * w + x] * sa);
            if rw > 0.0 {
                let c = d.ring.at(fx, fy, ring_r);
                over(&mut p.0, [c[0], c[1], c[2]], cover(fx, fy, ring_r) * c[3] as f32 / 255.0);
            }
            if bw > 0.0 {
                let c = d.border.at(fx, fy, border_r);
                over(&mut p.0, [c[0], c[1], c[2]], cover(fx, fy, border_r) * c[3] as f32 / 255.0);
            }
            let (wx, wy) = (x as i64 - pad as i64, y as i64 - pad as i64);
            if wx >= 0 && wy >= 0 && (wx as u32) < win.width() && (wy as u32) < win.height() {
                let q = win.get_pixel(wx as u32, wy as u32).0;
                over(&mut p.0, [q[0], q[1], q[2]], cover(fx, fy, win_r) * q[3] as f32 / 255.0);
            }
        }
    }
    out
}

// ── снимок ─────────────────────────────────────────────────────────────────
/// окно id (или в фокусе) с украшениями — PNG
/// pointer — курсор на снимке (niri кладёт его, только если он над окном)
pub fn capture(id: Option<u64>, pointer: bool) -> Result<Vec<u8>, String> {
    use image::ImageEncoder as _;
    let win = id.and_then(by_id).or_else(focused).ok_or_else(|| crate::tf!("нет окна в фокусе", "no focused window", "nenhuma janela em foco"))?;
    // снимок окна — только в свой каталог 0700
    let dir = ysnap_px::paths::private_tmp("ysnap-win").map_err(|e| e.to_string())?;
    let file = dir.join("window.png");
    let st = std::process::Command::new("niri")
        .args(["msg", "action", "screenshot-window", "--id", &win.id.to_string(), "--write-to-disk", "true", "--show-pointer", if pointer { "true" } else { "false" }, "--path"])
        .arg(&file)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    // niri пишет файл после ответа: ждём, пока он прочтётся целиком
    let t0 = Instant::now();
    let img = loop {
        if let Some(i) = std::fs::read(&file).ok().and_then(|b| image::load_from_memory(&b).ok()) {
            break Some(i);
        }
        if !st.as_ref().is_ok_and(|s| s.success()) || t0.elapsed() > Duration::from_secs(3) {
            break None;
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let _ = std::fs::remove_dir_all(&dir);
    let img = img.ok_or_else(|| crate::tf!("niri не снял окно", "niri did not capture the window", "o niri não capturou a janela"))?.to_rgba8();
    let k = if win.w > 0.0 { img.width() as f32 / win.w as f32 } else { 1.0 };
    let out = compose(&img, &Deco::load(&win), k);
    let mut png = vec![];
    image::codecs::png::PngEncoder::new(&mut png).write_image(out.as_raw(), out.width(), out.height(), image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
    Ok(png)
}

impl App {
    /// снимок окна в буфер и/или в папку — по настройкам, как Enter в оверлее
    pub fn window_shot(&mut self, id: Option<u64>) {
        let (copy, save) = (self.st.set.save.copy(), self.st.set.save.file());
        let (dir, name) = self.st.set.target();
        let notify = self.st.set.shot_notify;
        let pointer = self.st.set.show_pointer;
        let tx = self.tx.clone();
        self.jobs += 1;
        std::thread::spawn(move || {
            let (path, err) = match capture(id, pointer) {
                Ok(png) => {
                    let path = save.then(|| dir.join(&name));
                    let err = crate::snap::store(&png, path.as_deref(), copy);
                    (path, err)
                }
                Err(e) => (None, Some(e)),
            };
            let _ = tx.send(Msg::SnapDone { copied: copy && err.is_none(), path: path.filter(|_| err.is_none()), err, notify });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(app: &str) -> Win {
        Win { id: 1, app_id: app.into(), title: "t".into(), w: 100.0 }
    }

    #[test]
    fn kdl_blocks_props_and_comments() {
        let ns = parse("layout {\n  border { on; width 2; active-color \"#595959\" } // x\n  /* off */ shadow { offset x=0 y=8 }\n}\nwindow-rule { match app-id=\"^kitty$\"\n geometry-corner-radius 12 }\n");
        assert_eq!(ns.len(), 2);
        assert_eq!(ns[0].kids[0].kids.len(), 3);
        assert_eq!(ns[0].kids[1].kids[0].prop("y"), Some("8"));
        assert_eq!(ns[1].kids[0].prop("app-id"), Some("^kitty$"));
    }

    #[test]
    fn rules_apply_in_order() {
        let ns = parse(
            "layout { focus-ring { off; }\n border { on; width 2; active-color \"#595959\"; } }\n\
             layout { border { active-color \"#b0afad\"; } }\n\
             window-rule { geometry-corner-radius 12; }\n\
             window-rule { match app-id=\"zen\"; geometry-corner-radius 4 4 20 4; }\n\
             window-rule { match at-startup=true; geometry-corner-radius 99; }\n",
        );
        let d = Deco::from_nodes(&ns, &win("kitty"));
        assert!(!d.ring.on && d.border.on);
        assert_eq!((d.border.width, d.border.color), (2.0, [0xb0, 0xaf, 0xad, 255]));
        assert_eq!(d.radius, 12.0);
        assert_eq!(Deco::from_nodes(&ns, &win("zen-browser")).radius, 20.0);
        assert_eq!(d.shadow, Deco::default().shadow);
    }

    #[test]
    fn colors_and_gradient() {
        let ns = parse("layout { border { on; active-gradient from=\"#ff0000\" to=\"#0000ff\" angle=90; } }\nwindow-rule { match app-id=r#\"^(zen|firefox)$\"#; geometry-corner-radius 9; }");
        let d = Deco::from_nodes(&ns, &win("firefox"));
        assert_eq!(d.radius, 9.0);
        let r = (0.0, 0.0, 100.0, 10.0, 0.0);
        assert_eq!(d.border.at(0.0, 5.0, r), [255, 0, 0, 255]);
        assert_eq!(d.border.at(100.0, 5.0, r), [0, 0, 255, 255]);
        assert_eq!(rgba("#0007"), Some([0, 0, 0, 0x77]));
        assert_eq!(rgba("#b0afad"), Some([0xb0, 0xaf, 0xad, 255]));
    }

    #[test]
    fn composed_image_is_padded_with_transparent_corners() {
        let w = image::RgbaImage::from_pixel(40, 30, image::Rgba([200, 10, 10, 255]));
        let d = Deco { radius: 8.0, ..Deco::default() };
        let out = compose(&w, &d, 1.0);
        assert!(out.width() > 40 && out.height() > 30);
        assert_eq!(out.get_pixel(0, 0).0[3], 0);
        let (cx, cy) = (out.width() / 2, out.height() / 2);
        assert_eq!(out.get_pixel(cx, cy).0, [200, 10, 10, 255]);
    }
}
