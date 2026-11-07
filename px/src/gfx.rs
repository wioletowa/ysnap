// Пиксельная графика в shm-буфер: только целые пиксели, без сглаживания.
// Буфер — ARGB8888 с предумноженной альфой (как ждёт wl_shm), цвет C — обычный 0xAARRGGBB.

use crate::font::{Fonts, Face};
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct C(pub u32);

impl C {
    pub const fn hex(rgb: u32) -> C {
        C(0xff000000 | rgb)
    }
    pub fn from_f(r: f32, g: f32, b: f32) -> C {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        C(0xff000000 | q(r) << 16 | q(g) << 8 | q(b))
    }
    pub fn a(self) -> u32 {
        self.0 >> 24
    }
    pub fn rgbf(self) -> (f32, f32, f32) {
        (((self.0 >> 16) & 255) as f32 / 255.0, ((self.0 >> 8) & 255) as f32 / 255.0, (self.0 & 255) as f32 / 255.0)
    }
    pub fn alpha(self, a: f32) -> C {
        C(self.0 & 0xffffff | ((a.clamp(0.0, 1.0) * 255.0).round() as u32) << 24)
    }
    /// умножить прозрачность
    pub fn fade(self, k: f32) -> C {
        C(self.0 & 0xffffff | ((self.a() as f32 * k.clamp(0.0, 1.0)).round() as u32) << 24)
    }
    pub fn lerp(self, o: C, t: f32) -> C {
        let t = t.clamp(0.0, 1.0);
        let ch = |s: u32| {
            let a = ((self.0 >> s) & 255) as f32;
            let b = ((o.0 >> s) & 255) as f32;
            ((a + (b - a) * t).round() as u32) << s
        };
        C(ch(24) | ch(16) | ch(8) | ch(0))
    }
    pub fn hex_str(self) -> String {
        format!("#{:06x}", self.0 & 0xffffff)
    }
    /// предумноженное значение для буфера
    pub fn premul(self) -> u32 {
        let a = self.a();
        if a == 255 {
            return self.0;
        }
        let m = |s: u32| (((self.0 >> s) & 255) * a + 127) / 255;
        a << 24 | m(16) << 16 | m(8) << 8 | m(0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct R {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl R {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> R {
        R { x, y, w, h }
    }
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x as f64 && py >= self.y as f64 && px < (self.x + self.w) as f64 && py < (self.y + self.h) as f64
    }
    pub fn inset(&self, d: i32) -> R {
        R::new(self.x + d, self.y + d, (self.w - 2 * d).max(0), (self.h - 2 * d).max(0))
    }
    pub fn inter(&self, o: &R) -> R {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        R::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn cy(&self) -> i32 {
        self.y + self.h / 2
    }
    pub fn empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
}

/// Готовая картинка: предумноженные ARGB-пиксели
#[derive(Clone)]
pub struct Img {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u32>,
}

pub struct Canvas<'a> {
    pub buf: &'a mut [u32],
    pub w: i32,
    pub h: i32,
    pub ox: i32,
    pub oy: i32,
    clip: R,
    clips: Vec<R>,
    /// общая прозрачность (ступенчатое появление панелей)
    pub opacity: f32,
    /// где под поверхностью размывать фон: только сами стеклянные панели, без полей под тенью
    pub blur: Vec<R>,
    /// самый нижний край зон Ui с последнего сброса (до обрезки): так окно с прокруткой узнаёт,
    /// сколько содержимого нарисовала вкладка
    pub reach: i32,
    /// цвет жёсткой тени (из темы, set_shade)
    pub shade: C,
    /// тень у букв (виджеты прямо на обоях): каждая буква сначала этим цветом на «пиксель
    /// шрифта» ниже и правее
    pub text_shade: Option<C>,
}

/// цвет тени для всех новых холстов процесса: тема одна на процесс, а холстов много
static SHADE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0xb4000000);

pub fn set_shade(c: C) {
    SHADE.store(c.0, std::sync::atomic::Ordering::Relaxed);
}

/// узор проявления панелей (настройки → вид → эффекты)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reveal {
    /// матрица Байера 4×4 — исходный вид
    Dither,
    /// строки через одну, как развёртка старого экрана
    Scan,
    /// случайные клетки
    Noise,
    /// диагональ из угла со ступенчатым краем
    Wipe,
    /// Байер, но клетки вдвое крупнее
    Blocks,
}

impl Reveal {
    pub const ALL: [(&'static str, Reveal); 5] = [("dither", Reveal::Dither), ("scan", Reveal::Scan), ("noise", Reveal::Noise), ("wipe", Reveal::Wipe), ("blocks", Reveal::Blocks)];
    pub fn from_id(s: &str) -> Reveal {
        Reveal::ALL.iter().find(|(id, _)| *id == s).map(|x| x.1).unwrap_or(Reveal::Dither)
    }
}

static REVEAL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static GRAIN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);

/// узор и крупность клеток проявления для всего процесса; grain — множитель клетки (1–4)
pub fn set_reveal(r: Reveal, grain: i32) {
    let i = Reveal::ALL.iter().position(|x| x.1 == r).unwrap_or(0) as u8;
    REVEAL.store(i, std::sync::atomic::Ordering::Relaxed);
    GRAIN.store(grain.clamp(1, 4) as u8, std::sync::atomic::Ordering::Relaxed);
}

fn reveal() -> (Reveal, i32) {
    let i = REVEAL.load(std::sync::atomic::Ordering::Relaxed) as usize;
    let r = Reveal::ALL.get(i).map(|x| x.1).unwrap_or(Reveal::Dither);
    let g = GRAIN.load(std::sync::atomic::Ordering::Relaxed) as i32 * if r == Reveal::Blocks { 2 } else { 1 };
    (r, g)
}

const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
/// порядок строк при развёртке: через одну, потом в промежутки (как чересстрочный кадр)
const SCAN: [u8; 16] = [0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];

/// дешёвый хеш клетки: шум без генератора случайных чисел, одинаковый в каждом кадре
#[inline]
fn hash(x: i32, y: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9e37_79b1) ^ (y as u32).wrapping_mul(0x85eb_ca77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^ (h >> 12)
}

impl<'a> Canvas<'a> {
    pub fn new(buf: &'a mut [u32], w: i32, h: i32) -> Canvas<'a> {
        Canvas { buf, w, h, ox: 0, oy: 0, clip: R::new(0, 0, w, h), clips: vec![], opacity: 1.0, blur: vec![], reach: 0, shade: C(SHADE.load(std::sync::atomic::Ordering::Relaxed)), text_shade: None }
    }

    /// проявление панели: при k клетка видна, если её порог ниже k. Узор — из настроек
    /// (set_reveal): Байер, строки, шум, диагональ или крупные блоки
    pub fn dissolve(&mut self, k: f32, cell: i32) {
        let r = R::new(-self.ox, -self.oy, self.w, self.h);
        self.dissolve_rect(r, k, cell);
    }

    /// то же проявление, но только внутри r (появление отдельных значков, меню внутри окна)
    pub fn dissolve_rect(&mut self, r: R, k: f32, cell: i32) {
        if k >= 1.0 {
            return;
        }
        let r = R::new(r.x + self.ox, r.y + self.oy, r.w, r.h).inter(&R::new(0, 0, self.w, self.h));
        if r.empty() {
            return;
        }
        let (style, grain) = reveal();
        let cell = (cell * grain).max(1);
        // порог клетки в 0..256; клетка видна, если порог < lvl
        let lvl = (k.clamp(0.0, 1.0) * 256.0) as i32;
        // три способа, по замерам самый быстрый для своего узора (tools: шелл ~0.2–0.3 мс на
        // панель 780×560): Байер — поштучно по таблице, строки и диагональ — отрезками нулей
        // (сплошные куски), шум — маской без ветвлений
        if matches!(style, Reveal::Dither | Reveal::Blocks) {
            // четыре строки узора на всю ширину — заранее, чтобы в цикле не делить на клетку
            let l = lvl / 16;
            let pat: Vec<Vec<bool>> = (0..4).map(|by| (0..r.w).map(|i| BAYER[by][((i / cell) & 3) as usize] as i32 >= l).collect()).collect();
            let w = self.w;
            for y in r.y..r.bottom() {
                let hide = &pat[(((y - r.y) / cell) & 3) as usize];
                let row = (y * w + r.x) as usize;
                for (p, &h) in self.buf[row..row + r.w as usize].iter_mut().zip(hide) {
                    if h {
                        *p = 0;
                    }
                }
            }
            return;
        }
        let (cols, rows) = ((r.w + cell - 1) / cell, (r.h + cell - 1) / cell);
        let span = (cols + rows).max(1);
        let th = |cx: i32, cy: i32| match style {
            Reveal::Scan => SCAN[(cy & 15) as usize] as i32 * 16,
            Reveal::Noise => (hash(cx, cy) & 255) as i32,
            // диагональ сверху-слева, край — ступенями Байера
            _ => ((cx + cy) * 200 / span) + BAYER[(cy & 3) as usize][(cx & 3) as usize] as i32 * 3,
        };
        if style == Reveal::Noise {
            let mut keep = vec![0u32; r.w as usize];
            for y in r.y..r.bottom() {
                let ly = y - r.y;
                if ly % cell == 0 {
                    for cx in 0..cols {
                        let m = if th(cx, ly / cell) >= lvl { 0 } else { u32::MAX };
                        keep[(cx * cell) as usize..((cx + 1) * cell).min(r.w) as usize].fill(m);
                    }
                }
                let row = (y * self.w + r.x) as usize;
                for (p, &m) in self.buf[row..row + r.w as usize].iter_mut().zip(&keep) {
                    *p &= m;
                }
            }
            return;
        }
        let mut runs: Vec<(usize, usize)> = Vec::with_capacity(cols as usize);
        for y in r.y..r.bottom() {
            let ly = y - r.y;
            if ly % cell == 0 {
                runs.clear();
                for cx in 0..cols {
                    if th(cx, ly / cell) >= lvl {
                        let (a, b) = ((cx * cell) as usize, ((cx + 1) * cell).min(r.w) as usize);
                        match runs.last_mut() {
                            Some(last) if last.1 == a => last.1 = b,
                            _ => runs.push((a, b)),
                        }
                    }
                }
            }
            let row = (y * self.w + r.x) as usize;
            let line = &mut self.buf[row..row + r.w as usize];
            for &(a, b) in &runs {
                line[a..b].fill(0);
            }
        }
    }

    /// строки развёртки: каждая вторая строка темнее на долю k (только где что-то нарисовано;
    /// предумноженный цвет — множим каналы, альфу не трогаем)
    pub fn scanlines(&mut self, k: f32) {
        let m = ((1.0 - k.clamp(0.0, 1.0)) * 256.0) as u32;
        for y in (1..self.h).step_by(2) {
            let row = (y * self.w) as usize;
            for p in &mut self.buf[row..row + self.w as usize] {
                let v = *p;
                if v >> 24 == 0 {
                    continue;
                }
                let rb = (((v & 0x00ff_00ff) * m) >> 8) & 0x00ff_00ff;
                let g = (((v & 0x0000_ff00) * m) >> 8) & 0x0000_ff00;
                *p = (v & 0xff00_0000) | rb | g;
            }
        }
    }

    /// пунктир по контуру («бегущие муравьи»): phase сдвигает штрихи
    pub fn ants(&mut self, r: R, c: C, dash: i32, phase: i32) {
        let d = dash.max(1);
        let on = |i: i32| ((i + phase).rem_euclid(2 * d)) < d;
        for i in 0..r.w {
            if on(i) {
                self.fill(R::new(r.x + i, r.y, 1, 1), c);
            }
            if on(i + r.h) {
                self.fill(R::new(r.right() - 1 - i, r.bottom() - 1, 1, 1), c);
            }
        }
        for j in 0..r.h {
            if on(j + r.w) {
                self.fill(R::new(r.right() - 1, r.y + j, 1, 1), c);
            }
            if on(j) {
                self.fill(R::new(r.x, r.bottom() - 1 - j, 1, 1), c);
            }
        }
    }

    pub fn clear(&mut self, c: C) {
        let v = c.premul();
        self.buf.fill(v);
    }

    pub fn push_clip(&mut self, r: R) {
        self.clips.push(self.clip);
        let r = R::new(r.x + self.ox, r.y + self.oy, r.w, r.h);
        self.clip = self.clip.inter(&r);
    }
    /// текущая обрезка в координатах буфера (как у зон Ui)
    pub fn clip_rect(&self) -> R {
        self.clip
    }
    pub fn pop_clip(&mut self) {
        if let Some(c) = self.clips.pop() {
            self.clip = c;
        }
    }

    #[inline]
    fn blend_at(&mut self, i: usize, src: u32) {
        self.buf[i] = over(self.buf[i], src);
    }

    fn eff(&self, c: C) -> u32 {
        if self.opacity >= 1.0 {
            c.premul()
        } else {
            c.fade(self.opacity).premul()
        }
    }

    pub fn fill(&mut self, r: R, c: C) {
        if c.a() == 0 {
            return;
        }
        let r = R::new(r.x + self.ox, r.y + self.oy, r.w, r.h).inter(&self.clip);
        if r.empty() {
            return;
        }
        let src = self.eff(c);
        let w = self.w as usize;
        for y in r.y..r.y + r.h {
            let row = y as usize * w;
            if src >> 24 == 255 {
                self.buf[row + r.x as usize..row + (r.x + r.w) as usize].fill(src);
            } else {
                for x in r.x..r.x + r.w {
                    self.blend_at(row + x as usize, src);
                }
            }
        }
    }

    /// заменить пиксели (без смешивания) — «дырка» в прозрачность
    pub fn set(&mut self, r: R, c: C) {
        let r = R::new(r.x + self.ox, r.y + self.oy, r.w, r.h).inter(&self.clip);
        if r.empty() {
            return;
        }
        let src = c.premul();
        let w = self.w as usize;
        for y in r.y..r.y + r.h {
            let row = y as usize * w;
            self.buf[row + r.x as usize..row + (r.x + r.w) as usize].fill(src);
        }
    }

    /// пиксель с готовой (уже предумноженной) альфой — для текста и картинок
    #[inline]
    fn put(&mut self, x: i32, y: i32, src: u32) {
        let x = x + self.ox;
        let y = y + self.oy;
        if x < self.clip.x || y < self.clip.y || x >= self.clip.x + self.clip.w || y >= self.clip.y + self.clip.h {
            return;
        }
        let i = y as usize * self.w as usize + x as usize;
        self.blend_at(i, src);
    }

    pub fn outline(&mut self, r: R, c: C, s: i32) {
        self.fill(R::new(r.x, r.y, r.w, s), c);
        self.fill(R::new(r.x, r.bottom() - s, r.w, s), c);
        self.fill(R::new(r.x, r.y + s, s, r.h - 2 * s), c);
        self.fill(R::new(r.right() - s, r.y + s, s, r.h - 2 * s), c);
    }

    /// прямоугольник со срезанными углами (px.rs notched_fill)
    pub fn notched(&mut self, r: R, n: i32, c: C) {
        if r.w <= 2 * n || r.h <= 2 * n {
            self.fill(r, c);
            return;
        }
        self.fill(R::new(r.x + n, r.y, r.w - 2 * n, r.h), c);
        self.fill(R::new(r.x, r.y + n, n, r.h - 2 * n), c);
        self.fill(R::new(r.right() - n, r.y + n, n, r.h - 2 * n), c);
    }

    /// фаска a без фаски b: жёсткая тень только там, где её не закрывает панель. Под полупрозрачным
    /// стеклом тень иначе просвечивает второй рамкой со сдвигом (#16)
    pub fn notched_minus(&mut self, a: R, b: R, n: i32, c: C) {
        let span = |r: R, y: i32| -> Option<(i32, i32)> {
            if y < r.y || y >= r.bottom() {
                return None;
            }
            let cut = r.w > 2 * n && r.h > 2 * n && (y < r.y + n || y >= r.bottom() - n);
            Some(if cut { (r.x + n, r.right() - n) } else { (r.x, r.right()) })
        };
        // полосы, внутри которых обе строки не меняются
        let mut ys = [a.y, a.y + n, a.bottom() - n, a.bottom(), b.y, b.y + n, b.bottom() - n, b.bottom()].map(|y| y.clamp(a.y, a.bottom()));
        ys.sort_unstable();
        for w in ys.windows(2) {
            let (y0, y1) = (w[0], w[1]);
            let Some((a0, a1)) = span(a, y0).filter(|_| y1 > y0) else { continue };
            match span(b, y0) {
                Some((b0, b1)) if b0 < a1 && b1 > a0 => {
                    if b0 > a0 {
                        self.fill(R::new(a0, y0, b0 - a0, y1 - y0), c);
                    }
                    if b1 < a1 {
                        self.fill(R::new(b1, y0, a1 - b1, y1 - y0), c);
                    }
                }
                _ => self.fill(R::new(a0, y0, a1 - a0, y1 - y0), c),
            }
        }
    }

    /// размыть фон под r (углы с фаской n остаются чистыми)
    pub fn blur_rect(&mut self, r: R, n: i32) {
        let r = R::new(r.x + self.ox, r.y + self.oy, r.w, r.h);
        if self.blur.iter().any(|b| b.x <= r.x && b.y <= r.y && b.right() >= r.right() && b.bottom() >= r.bottom()) {
            return;
        }
        if n > 0 {
            self.blur.push(R::new(r.x, r.y + n, r.w, r.h - 2 * n));
            self.blur.push(R::new(r.x + n, r.y, r.w - 2 * n, r.h));
        } else {
            self.blur.push(r);
        }
    }

    pub fn pxbox(&mut self, r: R, st: &BoxStyle) {
        let n = st.notch;
        // стеклянная панель с тенью — верхний уровень, под ней размытие
        if st.shadow > 0 && st.fill.a() < 255 {
            self.blur_rect(r, n);
        }
        if st.shadow > 0 && !st.pressed {
            self.notched_minus(R::new(r.x + st.shadow, r.y + st.shadow, r.w, r.h), r, n, self.shade);
        }
        self.notched(r, n, st.fill);
        if st.bevel {
            let hi = st.fill.lerp(C(0xffffffff), if st.pressed { 0.0 } else { 0.22 });
            let lo = st.fill.lerp(C(0xff000000), if st.pressed { 0.1 } else { 0.45 });
            self.fill(R::new(r.x + n, r.y, r.w - 2 * n, 1), hi);
            self.fill(R::new(r.x, r.y + n, 1, r.h - 2 * n), hi);
            self.fill(R::new(r.x + n, r.bottom() - 1, r.w - 2 * n, 1), lo);
            self.fill(R::new(r.right() - 1, r.y + n, 1, r.h - 2 * n), lo);
        }
        let s = st.stroke;
        if s > 0 && st.outline.a() > 0 {
            let o = st.outline;
            self.fill(R::new(r.x + n, r.y, r.w - 2 * n, s), o);
            self.fill(R::new(r.x + n, r.bottom() - s, r.w - 2 * n, s), o);
            self.fill(R::new(r.x, r.y + n, s, r.h - 2 * n), o);
            self.fill(R::new(r.right() - s, r.y + n, s, r.h - 2 * n), o);
            if n > s {
                let k = n - s;
                self.fill(R::new(r.x + s, r.y + s, k, k), o);
                self.fill(R::new(r.right() - n, r.y + s, k, k), o);
                self.fill(R::new(r.x + s, r.bottom() - n, k, k), o);
                self.fill(R::new(r.right() - n, r.bottom() - n, k, k), o);
            }
        }
    }

    /// иконка 7×7 из битмапа клетками cell
    pub fn icon(&mut self, name: &str, x: i32, y: i32, cell: i32, c: C) {
        if let Some(rows) = icon7(name) {
            for (j, row) in rows.iter().enumerate() {
                for (i, ch) in row.bytes().enumerate() {
                    if ch == b'#' {
                        self.fill(R::new(x + i as i32 * cell, y + j as i32 * cell, cell, cell), c);
                    }
                }
            }
        }
    }
    /// батарейка клетками, как значки icon7: корпус 10×7 с фасками и «пипкой» (11 клеток в ширину),
    /// внутри 6 столбиков заряда; погасшие — цветом `empty`. frac > 0 зажигает хотя бы один
    pub fn battery(&mut self, x: i32, y: i32, cell: i32, frac: f32, body: C, fill: C, empty: C) {
        let c = |i: i32, j: i32, w: i32, h: i32| R::new(x + i * cell, y + j * cell, w * cell, h * cell);
        for r in [c(1, 0, 8, 1), c(1, 6, 8, 1), c(0, 1, 1, 5), c(9, 1, 1, 5), c(10, 2, 1, 3)] {
            self.fill(r, body);
        }
        let lit = if frac <= 0.0 { 0 } else { ((frac.min(1.0) * 6.0).ceil() as i32).max(1) };
        for k in 0..6 {
            self.fill(c(2 + k, 2, 1, 3), if k < lit { fill } else { empty });
        }
    }
    /// иконка по центру прямоугольника
    pub fn icon_in(&mut self, name: &str, r: R, cell: i32, c: C) {
        let s = 7 * cell;
        self.icon(name, r.x + (r.w - s) / 2, r.y + (r.h - s) / 2, cell, c);
    }

    pub fn image(&mut self, img: &Img, x: i32, y: i32, k: f32) {
        let k = k * self.opacity;
        for j in 0..img.h {
            for i in 0..img.w {
                let p = img.px[(j * img.w + i) as usize];
                if p >> 24 == 0 {
                    continue;
                }
                let p = if k >= 1.0 { p } else { scale_premul(p, k) };
                self.put(x + i, y + j, p);
            }
        }
    }

    /// непрозрачная картинка 1:1 — построчным копированием (замороженный экран)
    pub fn blit(&mut self, img: &Img, x: i32, y: i32) {
        let x = x + self.ox;
        let y = y + self.oy;
        let c = self.clip.inter(&R::new(x, y, img.w, img.h));
        if c.empty() {
            return;
        }
        let w = self.w as usize;
        for row in c.y..c.bottom() {
            let sy = (row - y) as usize;
            let sx = (c.x - x) as usize;
            let src = &img.px[sy * img.w as usize + sx..sy * img.w as usize + sx + c.w as usize];
            let d = row as usize * w + c.x as usize;
            self.buf[d..d + c.w as usize].copy_from_slice(src);
        }
    }

    /// картинка, растянутая без сглаживания в прямоугольник (обои, обложки)
    pub fn image_scaled(&mut self, img: &Img, r: R, k: f32) {
        if img.w == 0 || img.h == 0 || r.empty() {
            return;
        }
        let k = k * self.opacity;
        for j in 0..r.h {
            let sy = (j as i64 * img.h as i64 / r.h as i64) as i32;
            for i in 0..r.w {
                let sx = (i as i64 * img.w as i64 / r.w as i64) as i32;
                let p = img.px[(sy * img.w + sx) as usize];
                if p >> 24 == 0 {
                    continue;
                }
                let p = if k >= 1.0 { p } else { scale_premul(p, k) };
                self.put(r.x + i, r.y + j, p);
            }
        }
    }

    /// картинка ближайшим соседом на r без смешивания и прозрачности (обои на весь экран):
    /// строка картинки пересчитывается один раз, её повтор — копия готовой строки, при равной
    /// ширине — копия строки целиком. На 1080p это ~1 мс против ~10 у image_scaled
    pub fn image_fast(&mut self, img: &Img, r: R) {
        if img.w <= 0 || img.h <= 0 || r.empty() || img.px.len() < (img.w as usize) * (img.h as usize) {
            return;
        }
        let o = R::new(r.x + self.ox, r.y + self.oy, r.w, r.h);
        let d = o.inter(&self.clip);
        if d.empty() {
            return;
        }
        let (w, n, iw) = (self.w as usize, d.w as usize, img.w as usize);
        let same = img.w == o.w;
        let xs: Vec<usize> = if same { vec![] } else { (d.x..d.right()).map(|x| (((x - o.x) as i64 * img.w as i64 / o.w as i64) as usize).min(iw - 1)).collect() };
        let mut prev: Option<(i32, usize)> = None;
        for y in d.y..d.bottom() {
            let sy = (((y - o.y) as i64 * img.h as i64 / o.h as i64) as i32).clamp(0, img.h - 1);
            let row = y as usize * w + d.x as usize;
            if let Some((py, prow)) = prev.filter(|p| p.0 == sy) {
                self.buf.copy_within(prow..prow + n, row);
                prev = Some((py, row));
                continue;
            }
            let src = &img.px[sy as usize * iw..(sy as usize + 1) * iw];
            if same {
                let a = (d.x - o.x) as usize;
                self.buf[row..row + n].copy_from_slice(&src[a..a + n]);
            } else {
                for (p, &sx) in self.buf[row..row + n].iter_mut().zip(&xs) {
                    *p = src[sx];
                }
            }
            prev = Some((sy, row));
        }
    }

    // ── текст ───────────────────────────────────────────────────────────────
    /// строка от левого края; y — верх строки высотой face.line(). Возвращает ширину.
    pub fn text(&mut self, fonts: &mut Fonts, face: Face, x: i32, y: i32, s: &str, c: C) -> i32 {
        if let Some(sh) = self.text_shade.take() {
            // сдвиг — на «пиксель» шрифта: у Tiny5 это 1/8 кегля, у мелкого текста — 1 px
            let d = (fonts.line(face) / 16).max(1);
            self.text(fonts, face, x + d, y + d, s, sh);
            self.text_shade = Some(sh);
        }
        let col = self.eff(c);
        let mut pen = x;
        let base = y + fonts.ascent(face);
        for ch in s.chars() {
            if ch == '\n' {
                continue;
            }
            let g = fonts.glyph(face, ch);
            if let Some(g) = g {
                let gx = pen + g.left;
                let gy = base + g.top;
                for j in 0..g.h {
                    for i in 0..g.w {
                        let cov = g.cov[(j * g.w + i) as usize];
                        if cov == 0 {
                            continue;
                        }
                        let p = if cov == 255 { col } else { scale_premul(col, cov as f32 / 255.0) };
                        self.put(gx + i, gy + j, p);
                    }
                }
                pen += g.adv;
            }
        }
        pen - x
    }

    /// текст, отцентрированный по вертикали в r; обрезается многоточием по ширине r
    pub fn text_in(&mut self, fonts: &mut Fonts, face: Face, r: R, s: &str, c: C) -> i32 {
        let y = r.y + (r.h - fonts.line(face)) / 2;
        if fonts.width(face, s) <= r.w {
            self.text(fonts, face, r.x, y, s, c)
        } else {
            let e = fonts.elide(face, s, r.w);
            self.text(fonts, face, r.x, y, &e, c)
        }
    }
    pub fn text_center(&mut self, fonts: &mut Fonts, face: Face, r: R, s: &str, c: C) -> i32 {
        let w = fonts.width(face, s).min(r.w);
        self.text_in(fonts, face, R::new(r.x + (r.w - w) / 2, r.y, w, r.h), s, c)
    }
    pub fn text_right(&mut self, fonts: &mut Fonts, face: Face, r: R, s: &str, c: C) -> i32 {
        let w = fonts.width(face, s).min(r.w);
        self.text_in(fonts, face, R::new(r.right() - w, r.y, w, r.h), s, c)
    }
    /// перенос по словам; возвращает высоту
    pub fn text_wrap(&mut self, fonts: &mut Fonts, face: Face, r: R, s: &str, c: C, max_lines: usize) -> i32 {
        let lines = fonts.wrap(face, s, r.w, max_lines);
        let lh = fonts.line(face) + 2;
        for (i, l) in lines.iter().enumerate() {
            self.text(fonts, face, r.x, r.y + i as i32 * lh, l, c);
        }
        lines.len() as i32 * lh
    }

    // ── составные элементы ───────────────────────────────────────────────
    /// шапка окна: матовая полоса и линия снизу
    pub fn header(&mut self, t: &Theme, r: R) {
        self.blur_rect(r, 0);
        self.fill(r, t.glass_panel());
        self.fill(R::new(r.x, r.bottom() - 1, r.w, 1), t.border);
    }

    /// сегментная шкала с блеском сверху; пустые клетки видны
    pub fn gauge(&mut self, r: R, segments: i32, value: f32, c: C, empty: C, gap: i32) {
        let cw = r.w / segments.max(1);
        let v = value.clamp(0.0, 1.0);
        for i in 0..segments {
            let x = r.x + i * cw;
            let cell = R::new(x, r.y, cw - gap, r.h);
            self.fill(cell, empty);
            let amount = (v * segments as f32 - i as f32).clamp(0.0, 1.0);
            let fw = (cell.w as f32 * amount).round() as i32;
            if fw > 0 {
                self.fill(R::new(x, r.y, fw, r.h), c);
                self.fill(R::new(x, r.y, fw, 1), c.lerp(C(0xffffffff), 0.35));
            }
        }
    }

    /// пунктирный разделитель бара
    pub fn sep(&mut self, x: i32, cy: i32, c: C) {
        for i in 0..4 {
            self.fill(R::new(x, cy - 8 + i * 4, 2, 2), c);
        }
    }
}

/// src поверх d (оба предумножены)
#[inline]
pub fn over(d: u32, src: u32) -> u32 {
    let sa = src >> 24;
    if sa == 255 {
        return src;
    }
    if sa == 0 {
        return d;
    }
    let inv = 255 - sa;
    let ch = |s: u32| {
        let sv = (src >> s) & 255;
        let dv = (d >> s) & 255;
        ((sv + (dv * inv + 127) / 255).min(255)) << s
    };
    ch(24) | ch(16) | ch(8) | ch(0)
}

#[inline]
pub fn scale_premul(p: u32, k: f32) -> u32 {
    let m = |s: u32| ((((p >> s) & 255) as f32 * k).round() as u32).min(255) << s;
    m(24) | m(16) | m(8) | m(0)
}

#[derive(Clone, Copy)]
pub struct BoxStyle {
    pub fill: C,
    pub outline: C,
    pub notch: i32,
    pub stroke: i32,
    pub shadow: i32,
    pub bevel: bool,
    pub pressed: bool,
}

impl BoxStyle {
    pub fn new(fill: C, outline: C) -> BoxStyle {
        BoxStyle { fill, outline, notch: 2, stroke: 1, shadow: 0, bevel: false, pressed: false }
    }
    pub fn shadow(mut self, s: i32) -> Self {
        self.shadow = s;
        self
    }
    pub fn bevel(mut self, b: bool) -> Self {
        self.bevel = b;
        self
    }
    pub fn pressed(mut self, b: bool) -> Self {
        self.pressed = b;
        self
    }
}

/// иконки 7×7 (px.rs Icon)
pub fn icon7(name: &str) -> Option<&'static [&'static str; 7]> {
    Some(match name {
        "keys" => &[".......", "#######", "#.#.#.#", "#######", "##...##", "#######", "......."],
        "copy" => &["####...", "#..#...", "#.####.", "#.#..#.", "###..#.", "..#..#.", "..####."],
        "send" => &["#......", "###....", "#####..", "#######", "#####..", "###....", "#......"],
        "stop" => &[".......", ".#####.", ".#####.", ".#####.", ".#####.", ".#####.", "......."],
        "plus" => &["...#...", "...#...", "...#...", "#######", "...#...", "...#...", "...#..."],
        "gear" => &["#.###.#", ".#####.", "##...##", "##...##", "##...##", ".#####.", "#.###.#"],
        "close" => &["#.....#", ".#...#.", "..#.#..", "...#...", "..#.#..", ".#...#.", "#.....#"],
        "max" => &["#######", "#######", "#.....#", "#.....#", "#.....#", "#.....#", "#######"],
        "restore" => &["..#####", "..#...#", "#####.#", "#####.#", "#...###", "#...#..", "#####.."],
        "min" => &[".......", ".......", ".......", ".......", ".......", "#######", "#######"],
        "search" => &[".###...", "#...#..", "#...#..", "#...#..", ".####..", "....##.", ".....##"],
        "down" => &["...#...", "...#...", "...#...", "#..#..#", ".#.#.#.", "..###..", "...#..."],
        "up" => &["...#...", "..###..", ".#.#.#.", "#..#..#", "...#...", "...#...", "...#..."],
        "left" => &["...#...", "..#....", ".#.....", "#######", ".#.....", "..#....", "...#..."],
        "right" => &["...#...", "....#..", ".....#.", "#######", ".....#.", "....#..", "...#..."],
        "check" => &["......#", ".....##", "#...##.", "##.##..", ".###...", "..#....", "......."],
        "folder" => &["###....", "#..####", "#.....#", "#.....#", "#.....#", "#######", "......."],
        "trash" => &["..###..", "#######", ".#.#.#.", ".#.#.#.", ".#.#.#.", ".#.#.#.", "..###.."],
        "pin" => &["..###..", ".#####.", ".#####.", "..###..", "...#...", "...#...", "...#..."],
        "drop" => &["...#...", "..###..", ".#####.", ".##.##.", "##.####", "#######", ".#####."],
        "image" => &["#######", "#.....#", "#.#...#", "#...#.#", "#..####", "#.#####", "#######"],
        "power" => &["...#...", ".#.#.#.", "#..#..#", "#.....#", "#.....#", ".#...#.", "..###.."],
        "user" => &["..###..", "..###..", "..###..", ".......", ".#####.", "#######", "#######"],
        "bolt" => &["....##.", "...##..", "..##...", ".#####.", "...##..", "..##...", ".##...."],
        "leaf" => &["....###", "..#####", ".###..#", ".##..##", "#.#.##.", "#..##..", "#......"],
        "gauge" => &[".......", "..###..", ".#...#.", "#..#..#", "#.#...#", "#.....#", "#######"],
        "lock" => &["..###..", ".#...#.", ".#...#.", "#######", "###.###", "###.###", "#######"],
        "speaker" => &["...#...", "..##.#.", "####..#", "####..#", "####..#", "..##.#.", "...#..."],
        "mute" => &["...#...", "..##...", "####.#.", "####..#", "####.#.", "..##...", "...#..."],
        "mic" => &["..###..", "..###..", "..###..", "#.###.#", ".#####.", "...#...", "..###.."],
        "float" => &["####...", "#..#...", "#.#####", "###...#", "..#...#", "..#...#", "..#####"],
        "tile" => &["###.###", "#.#.#.#", "#.#.#.#", "#.#.#.#", "#.#.#.#", "#.#.#.#", "###.###"],
        "moon" => &["..###..", ".##....", "##.....", "##.....", "##.....", ".##...#", "..####."],
        "sun" => &["...#...", ".#...#.", "..###..", "#.###.#", "..###..", ".#...#.", "...#..."],
        "calc" => &["#######", "#.....#", "#######", "#.#.#.#", "#######", "#.#.#.#", "#######"],
        "qr" => &["###.###", "#.#.#.#", "###.###", ".......", "###.#.#", "#.#..#.", "###.#.#"],
        "term" => &[".......", "#......", ".#.....", "..#....", ".#.....", "#..###.", "......."],
        "reboot" => &["..####.", ".#...##", "#...###", "#......", "#.....#", ".#...#.", "..###.."],
        "exit" => &["####...", "#......", "#...#..", "#.#####", "#...#..", "#......", "####..."],
        "top" => &["#######", ".......", "...#...", "..###..", ".#.#.#.", "...#...", "...#..."],
        "shade" => &["#######", "#######", ".......", ".......", ".......", ".......", "......."],
        "sticky" => &["..####.", "..#..#.", "####.#.", "#..###.", "#..#...", "####...", "......."],
        "wifi" => &[".#####.", "#.....#", "..###..", ".#...#.", "...#...", "..#.#..", "...#..."],
        "net" => &["...#...", "..###..", ".#.#.#.", "...#...", "...#...", "...#...", "#######"],
        // погода (weather.rs): облако, солнце за облаком, дождь, снег, гроза, туман
        "cloud" => &[".......", "..##...", ".####..", ".#####.", "#######", "#######", "......."],
        "cloudsun" => &["#.#....", ".##....", "#..##..", "..####.", ".######", "#######", "......."],
        "rain" => &["..##...", ".####..", "#######", "#######", ".......", "#.#.#..", ".#.#.#."],
        "snow" => &["..##...", ".####..", "#######", ".......", "#..#..#", ".......", ".#..#.."],
        "storm" => &["..##...", ".####..", "#######", "...#...", "..##...", "...#...", "..#...."],
        "fog" => &[".......", "#####..", ".......", ".######", ".......", "#####..", "......."],
        // проводная сеть: узел и два компьютера (модуль `net` бара)
        "lan" => &["..###..", "..###..", "...#...", ".#####.", ".#...#.", "###.###", "###.###"],
        "bt" => &["...##..", "#..#.#.", ".#.#.#.", "..##...", ".#.#.#.", "#..#.#.", "...##.."],
        "play" => &[".#.....", ".##....", ".###...", ".####..", ".###...", ".##....", ".#....."],
        "pause" => &[".##.##.", ".##.##.", ".##.##.", ".##.##.", ".##.##.", ".##.##.", ".##.##."],
        "next" => &["#...#.#", "##..#.#", "###.#.#", "####..#", "###.#.#", "##..#.#", "#...#.#"],
        "prev" => &["#.#...#", "#.#..##", "#.#.###", "#..####", "#.#.###", "#.#..##", "#.#...#"],
        "bell" => &["...#...", "..###..", ".#####.", ".#####.", ".#####.", "#######", "...#..."],
        "bellx" => &["#..#...", ".####..", ".##.##.", ".###.#.", ".####.#", "#######", "...#..."],
        "cpu" => &[".#.#.#.", "#######", ".#...#.", "##.#.##", ".#...#.", "#######", ".#.#.#."],
        "temp" => &["..##...", "..#.#..", "..##...", "..#.#..", ".####..", "######.", ".####.."],
        "ram" => &["#######", "#.#.#.#", "#.#.#.#", "#######", ".#.#.#.", ".#.#.#.", "......."],
        "clock" => &["..###..", ".#.#.#.", "#..#..#", "#..##.#", "#.....#", ".#...#.", "..###.."],
        "cal" => &["#.#.#.#", "#######", "#.....#", "#.#.#.#", "#.....#", "#.#.#.#", "#######"],
        "grid" => &["##.##..", "##.##..", ".......", "##.##..", "##.##..", ".......", "......."],
        "drag" => &["#.#.#.#", ".......", "#.#.#.#", ".......", "#.#.#.#", ".......", "#.#.#.#"],
        "eye" => &[".......", "..###..", ".#...#.", "#..#..#", ".#...#.", "..###..", "......."],
        "music" => &["..####.", "..#..#.", "..#..#.", "..#..#.", "###.###", "###.###", "......."],
        "about" => &[".#####.", "#.....#", "#.#.#.#", "#.....#", ".#.#.#.", "..###..", "......."],
        "chart" => &["......#", ".....##", "...#.##", "..##.##", "#.##.##", "#.##.##", "#######"],
        "menu" => &["#######", ".......", "#######", ".......", "#######", ".......", "......."],
        "dots" => &[".......", ".......", ".......", "#..#..#", ".......", ".......", "......."],
        // скриншотер
        "camera" => &[".......", "..##...", "#######", "#..#..#", "#.#.#.#", "#..#..#", "#######"],
        "cursor" => &["#......", "##.....", "###....", "####...", "#####..", "##.....", "#.#...."],
        "pen" => &[".....##", "....###", "...###.", "..###..", ".###...", "##.....", "#......"],
        "rect" => &[".......", "#######", "#.....#", "#.....#", "#.....#", "#######", "......."],
        "arrow" => &["...####", ".....##", "....#.#", "...#..#", "..#....", ".#.....", "#......"],
        "marker" => &[".......", ".......", "#######", "#######", "#######", ".......", "......."],
        "mosaic" => &["##..##.", "##..##.", "..##..#", "..##..#", "##..##.", "##..##.", "..##..#"],
        "count" => &[".#####.", "##.#.##", "#..#..#", "##.#.##", "#..#..#", "##...##", ".#####."],
        "text" => &["#######", "#..#..#", "...#...", "...#...", "...#...", "...#...", "..###.."],
        "window" => &["#######", "#######", "#.....#", "#.....#", "#.....#", "#.....#", "#######"],
        "screen" => &["#######", "#.....#", "#.....#", "#.....#", "#######", "...#...", "..###.."],
        "timer" => &["..###..", "...#...", ".#####.", "#..#..#", "#..##.#", "#.....#", ".#####."],
        // просмотрщик
        "rotl" => &[".####..", "#....#.", "#.....#", "###...#", ".#....#", "......#", "..####."],
        "rotr" => &["..####.", ".#....#", "#.....#", "#...###", "#....#.", "#......", ".####.."],
        "fliph" => &["...#...", "#..#..#", "##.#.##", "####.##", "##.#.##", "#..#..#", "...#..."],
        "fit" => &["##...##", "#.....#", "..###..", "..###..", "..###..", "#.....#", "##...##"],
        "one" => &["...#...", "..##...", "...#...", "...#...", "...#...", "...#...", "..###.."],
        "full" => &["###.###", "#.....#", "#.....#", ".......", "#.....#", "#.....#", "###.###"],
        "zoomin" => &[".###...", "#.#.#..", "#####..", "#.#.#..", ".####..", "....##.", ".....##"],
        "zoomout" => &[".###...", "#...#..", "#####..", "#...#..", ".####..", "....##.", ".....##"],
        "crop" => &[".#.....", ".######", ".#...#.", ".#...#.", ".#...#.", "######.", ".....#."],
        "save" => &["######.", "#....##", "#.....#", "#######", "#.###.#", "#.###.#", "#######"],
        "slides" => &["#######", "#.....#", "#.#...#", "#.##..#", "#.#...#", "#.....#", "#######"],
        "redo" => &["...#...", "....#..", "#######", "....#..", "...#...", ".......", "......."],
        _ => return None,
    })
}

/// Размер буфера в физических пикселях для логического w×h при масштабе вывода `scale`
/// (1.0, 1.25, 2.0 — из wp_fractional_scale, preferred_scale / 120).
pub fn phys(w: i32, h: i32, scale: f64) -> (i32, i32) {
    (((w as f64) * scale).round().max(1.0) as i32, ((h as f64) * scale).round().max(1.0) as i32)
}

/// Пиксельно-точное увеличение (ближайший сосед) логического холста src (w×h) в буфер dst
/// (pw×ph): на 4K с масштабом 2 каждый пиксель шелла — ровно 2×2, без мыла от
/// билинейного растяжения композитором. Дробный масштаб даёт чуть неровные клетки, но чётко.
pub fn upscale(src: &[u32], w: i32, h: i32, dst: &mut [u32], pw: i32, ph: i32) {
    let (w, h, pw, ph) = (w as usize, h as usize, pw as usize, ph as usize);
    if w == 0 || h == 0 || src.len() < w * h || dst.len() < pw * ph {
        return;
    }
    if pw % w == 0 && ph % h == 0 && pw / w == ph / h {
        // целый масштаб: строку растянуть один раз, потом копировать
        let k = pw / w;
        for y in 0..h {
            let row = &src[y * w..y * w + w];
            let first = y * k * pw;
            {
                let out = &mut dst[first..first + pw];
                for (x, &p) in row.iter().enumerate() {
                    out[x * k..x * k + k].fill(p);
                }
            }
            for r in 1..k {
                dst.copy_within(first..first + pw, first + r * pw);
            }
        }
        return;
    }
    let xs: Vec<usize> = (0..pw).map(|x| (x * w / pw).min(w - 1)).collect();
    let mut last = usize::MAX;
    for y in 0..ph {
        let sy = (y * h / ph).min(h - 1);
        let out = y * pw;
        if sy == last {
            dst.copy_within(out - pw..out, out);
            continue;
        }
        last = sy;
        let row = &src[sy * w..sy * w + w];
        for (x, &sx) in xs.iter().enumerate() {
            dst[out + x] = row[sx];
        }
    }
}

/// Наложить логический холст src (w×h, предумноженный ARGB, прозрачное — 0) поверх физического
/// буфера dst (pw×ph) с увеличением ближайшим соседом: интерфейс плеера поверх кадра видео,
/// который уже нарисован в полном разрешении. Прозрачные клетки пропускаются целиком.
pub fn overlay_up(src: &[u32], w: i32, h: i32, dst: &mut [u32], pw: i32, ph: i32) {
    let (w, h, pw, ph) = (w as usize, h as usize, pw as usize, ph as usize);
    if w == 0 || h == 0 || src.len() < w * h || dst.len() < pw * ph {
        return;
    }
    let xs: Vec<usize> = (0..pw).map(|x| (x * w / pw).min(w - 1)).collect();
    for y in 0..ph {
        let row = &src[(y * h / ph).min(h - 1) * w..][..w];
        if row.iter().all(|&p| p >> 24 == 0) {
            continue;
        }
        let out = &mut dst[y * pw..y * pw + pw];
        for (x, &sx) in xs.iter().enumerate() {
            let s = row[sx];
            let a = s >> 24;
            if a == 255 {
                out[x] = s;
            } else if a > 0 {
                let d = out[x];
                let inv = 255 - a;
                let ch = |sh: u32| (((s >> sh) & 255) + (((d >> sh) & 255) * inv + 127) / 255).min(255) << sh;
                out[x] = 0xff00_0000 | ch(16) | ch(8) | ch(0);
            }
        }
    }
}
