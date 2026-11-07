// Шрифты: Tiny5 (пиксельный, чёткий на 16/24/32/48…), DotGothic16 (иероглифы), Monocraft
// (моно, тот же, что в терминале). Пиксельные рисуются без сглаживания — покрытие ≥ ½
// становится целым пикселем. Чего нет в пиксельных (эмодзи, редкие знаки) — из системного
// шрифта по fontconfig, уже со сглаживанием.

use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont};
use std::collections::HashMap;
use std::process::Command;

#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug)]
pub enum Kind {
    Px,
    Jp,
}

#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug)]
pub struct Face {
    pub kind: Kind,
    pub px: u16,
}

pub const PX: Face = Face { kind: Kind::Px, px: 16 };
pub const PX24: Face = Face { kind: Kind::Px, px: 24 };

pub fn px(size: u16) -> Face {
    Face { kind: Kind::Px, px: size }
}
pub fn jp(size: u16) -> Face {
    Face { kind: Kind::Jp, px: size }
}

pub struct Glyph {
    pub w: i32,
    pub h: i32,
    pub left: i32,
    pub top: i32,
    pub adv: i32,
    pub cov: Vec<u8>,
}

struct Src {
    font: FontRef<'static>,
    pixel: bool,
    /// множитель кегля: свой (не пиксельный) шрифт подгоняется по высоте прописных к Tiny5,
    /// чтобы вёрстка панелей не поехала
    k: f32,
}

/// пиксельный ли шрифт (по имени): такие рисуются без сглаживания и без подгонки кегля
pub fn is_pixel_family(f: &str) -> bool {
    let f = f.to_lowercase();
    ["pixel", "tiny5", "monocraft", "terminess", "dotgothic", "silkscreen", "press start", "unifont", "bitmap", "tamzen", "cozette"].iter().any(|p| f.contains(p))
}

/// высота прописной H в долях кегля
fn cap(font: &FontRef<'static>) -> Option<f32> {
    let o = font.outline(font.glyph_id('H'))?;
    Some((o.bounds.max.y - o.bounds.min.y).abs() / font.units_per_em()?)
}

pub struct Fonts {
    srcs: Vec<Src>,
    // индексы в srcs: порядок поиска глифа для каждого вида
    chain_px: Vec<usize>,
    chain_jp: Vec<usize>,
    cache: HashMap<(Face, char), Option<Glyph>>,
    adv_cache: HashMap<(Face, char), i32>,
    pub ellipsis: &'static str,
}

/// найденное fc-match за время жизни процесса: смена шрифта (ползунок «пиксельности» ходит
/// туда-обратно) не запускает его заново по семь раз в нити Wayland. Промахи не запоминаются:
/// только что поставленный шрифт найдётся со второй попытки
static FOUND: std::sync::Mutex<Vec<(String, Option<String>, (String, u32))>> = std::sync::Mutex::new(Vec::new());

fn fc_find(pattern: &str, want: Option<&str>) -> Option<(String, u32)> {
    if let Some(hit) = FOUND.lock().ok()?.iter().find(|(p, w, _)| (p.as_str(), w.as_deref()) == (pattern, want)).map(|x| x.2.clone()) {
        return Some(hit);
    }
    let r = fc_match(pattern, want)?;
    if let Ok(mut f) = FOUND.lock() {
        f.push((pattern.to_string(), want.map(str::to_string), r.clone()));
    }
    Some(r)
}

fn fc_match(pattern: &str, want: Option<&str>) -> Option<(String, u32)> {
    let out = Command::new("fc-match").args(["-f", "%{family}|%{file}|%{index}", pattern]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let mut it = s.split('|');
    let fam = it.next()?.to_lowercase();
    let file = it.next()?.to_string();
    let idx = it.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    if let Some(w) = want {
        if !fam.contains(&w.to_lowercase()) {
            return None;
        }
    }
    Some((file, idx))
}

/// уже отображённые файлы: отображение живёт до конца процесса, поэтому смена шрифта
/// (Fonts::with заново) берёт те же страницы, а не отображает файлы ещё раз
static MAPPED: std::sync::Mutex<Vec<(String, &'static [u8])>> = std::sync::Mutex::new(Vec::new());

/// шрифт отображается в память, а не читается: в RSS попадают только страницы нужных
/// глифов (CJK и эмодзи весят десятки мегабайт, а нужны из них единицы)
fn load(file: &str, idx: u32) -> Option<FontRef<'static>> {
    use std::os::unix::io::AsRawFd;
    let mut maps = MAPPED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, data)) = maps.iter().find(|(f, _)| f == file) {
        return FontRef::try_from_slice_and_index(data, idx).ok();
    }
    let f = std::fs::File::open(file).ok()?;
    let len = f.metadata().ok()?.len() as usize;
    if len == 0 {
        return None;
    }
    let data: &'static [u8] = unsafe {
        let p = libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, f.as_raw_fd(), 0);
        if p == libc::MAP_FAILED {
            return None;
        }
        std::slice::from_raw_parts(p as *const u8, len)
    };
    maps.push((file.to_string(), data));
    FontRef::try_from_slice_and_index(data, idx).ok()
}

impl Fonts {
    /// Tiny5 и запасные
    pub fn new() -> Fonts {
        Fonts::with("")
    }

    /// свой основной шрифт (семейство fontconfig); "" или Tiny5 — пиксельный по умолчанию
    pub fn with(family: &str) -> Fonts {
        let family = family.trim();
        let mut srcs: Vec<Src> = vec![];
        let add = |pat: &str, want: Option<&str>, pixel: bool, srcs: &mut Vec<Src>| -> Option<usize> {
            let (f, i) = fc_find(pat, want)?;
            let font = load(&f, i)?;
            srcs.push(Src { font, pixel, k: 1.0 });
            Some(srcs.len() - 1)
        };
        // общее семейство (sans, serif) — что fontconfig даст; имя найденного может быть любым
        let generic = matches!(family.to_lowercase().as_str(), "sans" | "sans-serif" | "serif" | "monospace");
        let custom = if family.is_empty() || family.eq_ignore_ascii_case("tiny5") { None } else { add(family, (!generic).then_some(family), !generic && is_pixel_family(family), &mut srcs) };
        let tiny = add("Tiny5", Some("tiny5"), true, &mut srcs);
        if let (Some(c), Some(t)) = (custom, tiny) {
            if !srcs[c].pixel {
                if let (Some(a), Some(b)) = (cap(&srcs[t].font), cap(&srcs[c].font)) {
                    srcs[c].k = (a / b).clamp(0.5, 1.5);
                }
            }
        }
        let dot = add("DotGothic16", Some("dotgothic"), true, &mut srcs);
        let mono = add("Monocraft", Some("monocraft"), true, &mut srcs)
            .or_else(|| add("Terminess Nerd Font Mono", Some("terminess"), true, &mut srcs));
        let sans = add("sans-serif:lang=ru", None, false, &mut srcs);
        let cjk = add("sans-serif:lang=ja", None, false, &mut srcs);
        let emoji = add("emoji", None, false, &mut srcs);
        let chain = |xs: &[Option<usize>]| -> Vec<usize> {
            let mut v: Vec<usize> = vec![];
            for x in xs.iter().flatten() {
                if !v.contains(x) {
                    v.push(*x);
                }
            }
            v
        };
        let chain_px = chain(&[custom, tiny, mono, dot, sans, cjk, emoji]);
        let chain_jp = chain(&[dot, tiny, cjk, sans]);
        let mut f = Fonts {
            srcs,
            chain_px,
            chain_jp,
            cache: HashMap::new(),
            adv_cache: HashMap::new(),
            ellipsis: "…",
        };
        if let Some(&first) = f.chain_px.first() {
            if f.srcs[first].font.glyph_id('…').0 == 0 {
                f.ellipsis = "..";
            }
        }
        f
    }

    /// шрифты из файлов папки, без fontconfig: Tiny5.ttf и DotGothic16-Regular.ttf. Для экрана
    /// входа: семь запусков fc-match — доли секунды старта, а шрифтов из домашней папки у его
    /// пользователя нет. Нет Tiny5 — как Fonts::new()
    pub fn bundled(dir: &std::path::Path) -> Fonts {
        let file = |n: &str| load(&dir.join(n).to_string_lossy(), 0);
        let Some(tiny) = file("Tiny5.ttf") else { return Fonts::new() };
        let mut srcs = vec![Src { font: tiny, pixel: true, k: 1.0 }];
        if let Some(d) = file("DotGothic16-Regular.ttf") {
            srcs.push(Src { font: d, pixel: true, k: 1.0 });
        }
        let chain_px: Vec<usize> = (0..srcs.len()).collect();
        let chain_jp: Vec<usize> = chain_px.iter().rev().copied().collect();
        let ellipsis = if srcs[0].font.glyph_id('…').0 == 0 { ".." } else { "…" };
        Fonts { srcs, chain_px, chain_jp, cache: HashMap::new(), adv_cache: HashMap::new(), ellipsis }
    }

    fn chain(&self, k: Kind) -> &[usize] {
        match k {
            Kind::Px => &self.chain_px,
            Kind::Jp => &self.chain_jp,
        }
    }

    fn scale(font: &FontRef<'static>, em: f32) -> PxScale {
        let upem = font.units_per_em().unwrap_or(1000.0);
        PxScale::from(em * font.height_unscaled() / upem)
    }

    /// размер кегля для вида: Monocraft чёток, когда его пиксель — целый
    fn em(&self, face: Face) -> f32 {
        face.px as f32
    }

    fn primary(&self, face: Face) -> Option<&Src> {
        self.chain(face.kind).first().map(|&i| &self.srcs[i])
    }

    pub fn ascent(&self, face: Face) -> i32 {
        match self.primary(face) {
            Some(src) => src.font.as_scaled(Self::scale(&src.font, self.em(face) * src.k)).ascent().round() as i32,
            None => face.px as i32,
        }
    }
    pub fn line(&self, face: Face) -> i32 {
        match self.primary(face) {
            Some(src) => {
                let s = src.font.as_scaled(Self::scale(&src.font, self.em(face) * src.k));
                (s.ascent() - s.descent()).round() as i32
            }
            None => face.px as i32 + 4,
        }
    }

    fn pick(&self, face: Face, ch: char) -> Option<(usize, GlyphId)> {
        for &i in self.chain(face.kind) {
            let g = self.srcs[i].font.glyph_id(ch);
            if g.0 != 0 {
                return Some((i, g));
            }
        }
        None
    }

    pub fn advance(&mut self, face: Face, ch: char) -> i32 {
        if let Some(a) = self.adv_cache.get(&(face, ch)) {
            return *a;
        }
        let a = match self.pick(face, ch) {
            Some((i, g)) => {
                let src = &self.srcs[i];
                src.font.as_scaled(Self::scale(&src.font, self.em(face) * src.k)).h_advance(g).round() as i32
            }
            None => (face.px / 2) as i32,
        };
        self.adv_cache.insert((face, ch), a);
        a
    }

    pub fn glyph(&mut self, face: Face, ch: char) -> Option<&Glyph> {
        if !self.cache.contains_key(&(face, ch)) {
            let g = self.raster(face, ch);
            self.cache.insert((face, ch), g);
            if self.cache.len() > 6000 {
                // редкие знаки не копим бесконечно
                self.cache.retain(|k, _| (k.1 as u32) < 0x500);
            }
        }
        self.cache.get(&(face, ch)).and_then(|g| g.as_ref())
    }

    fn raster(&mut self, face: Face, ch: char) -> Option<Glyph> {
        let adv = self.advance(face, ch);
        let (i, gid) = match self.pick(face, ch) {
            Some(x) => x,
            None => {
                // нет нигде — пустой квадратик нужной ширины
                return Some(Glyph { w: 0, h: 0, left: 0, top: 0, adv, cov: vec![] });
            }
        };
        let src = &self.srcs[i];
        let scale = Self::scale(&src.font, self.em(face) * src.k);
        let glyph = gid.with_scale_and_position(scale, ab_glyph::point(0.0, 0.0));
        let Some(og) = src.font.outline_glyph(glyph) else {
            return Some(Glyph { w: 0, h: 0, left: 0, top: 0, adv, cov: vec![] });
        };
        let b = og.px_bounds();
        let w = b.width().ceil() as i32;
        let h = b.height().ceil() as i32;
        let mut cov = vec![0u8; (w * h).max(0) as usize];
        let pixel = src.pixel;
        og.draw(|x, y, c| {
            let idx = (y as i32 * w + x as i32) as usize;
            if idx < cov.len() {
                cov[idx] = if pixel {
                    if c >= 0.5 { 255 } else { 0 }
                } else {
                    (c.clamp(0.0, 1.0) * 255.0) as u8
                };
            }
        });
        Some(Glyph { w, h, left: b.min.x.round() as i32, top: b.min.y.round() as i32, adv, cov })
    }

    pub fn width(&mut self, face: Face, s: &str) -> i32 {
        s.chars().filter(|c| *c != '\n').map(|c| self.advance(face, c)).sum()
    }

    /// строка, урезанная до max px с многоточием; влезает — как есть
    pub fn elide(&mut self, face: Face, s: &str, max: i32) -> String {
        if self.width(face, s) <= max {
            return s.to_string();
        }
        let e = self.ellipsis;
        let ew = self.width(face, e);
        let mut w = 0;
        let mut out = String::new();
        for ch in s.chars() {
            let a = self.advance(face, ch);
            if w + a + ew > max {
                break;
            }
            w += a;
            out.push(ch);
        }
        out.push_str(e);
        out
    }

    pub fn wrap(&mut self, face: Face, s: &str, max: i32, max_lines: usize) -> Vec<String> {
        let mut lines: Vec<String> = vec![];
        let space = self.advance(face, ' ');
        'outer: for para in s.split('\n') {
            let mut cur = String::new();
            let mut cw = 0;
            for word in para.split(' ') {
                let ww = self.width(face, word);
                if cw > 0 && cw + space + ww > max {
                    lines.push(std::mem::take(&mut cur));
                    cw = 0;
                    if lines.len() >= max_lines {
                        break 'outer;
                    }
                }
                if ww > max {
                    // длинное слово режем по буквам
                    for ch in word.chars() {
                        let a = self.advance(face, ch);
                        if cw + a > max && cw > 0 {
                            lines.push(std::mem::take(&mut cur));
                            cw = 0;
                            if lines.len() >= max_lines {
                                break 'outer;
                            }
                        }
                        cur.push(ch);
                        cw += a;
                    }
                    continue;
                }
                if cw > 0 {
                    cur.push(' ');
                    cw += space;
                }
                cur.push_str(word);
                cw += ww;
            }
            lines.push(cur);
            if lines.len() >= max_lines {
                break;
            }
        }
        if lines.len() >= max_lines {
            // последняя строка — с многоточием, если текст не влез
            let total: usize = lines.iter().map(|l| l.chars().count()).sum();
            if total + lines.len() < s.chars().count() {
                let last = lines.pop().unwrap_or_default();
                let el = self.elide(face, &(last + " "), max);
                lines.push(el);
            }
        }
        lines
    }

    pub fn wrap_height(&mut self, face: Face, s: &str, max: i32, max_lines: usize) -> i32 {
        self.wrap(face, s, max, max_lines).len() as i32 * (self.line(face) + 2)
    }
}
