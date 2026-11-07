// Немедленный UI: каждый кадр рисуем заново и записываем зоны (прямоугольник → действие).
// Клик ищет верхнюю зону под курсором и выполняет её действие; наведение — просто
// «курсор внутри прямоугольника» в момент рисования, без анимаций, а значит без мерцания.

use crate::font::{self, Face, Fonts};
use crate::gfx::{BoxStyle, Canvas, C, R};
use crate::icons::Icons;
use crate::theme::{Theme, CLEAR};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cur {
    Default,
    Pointer,
    Grab,
    Text,
    /// угол окна: тянуть — размер
    Resize,
}

#[derive(Clone, Debug)]
pub struct Zone<A> {
    pub r: R,
    pub act: A,
    pub cur: Cur,
}

/// однострочное поле ввода
#[derive(Default, Clone)]
pub struct Field {
    pub text: String,
    pub cur: usize,
    /// весь текст выделен (Ctrl+A): ввод заменяет его, стирание убирает целиком
    pub all: bool,
}

impl Field {
    pub fn set(&mut self, s: &str) {
        self.text = s.to_string();
        self.cur = self.text.chars().count();
        self.all = false;
    }
    /// выделить всё (пустое поле выделять нечего)
    pub fn select_all(&mut self) {
        self.all = !self.text.is_empty();
        self.cur = self.text.chars().count();
    }
    /// выделенный текст (для копирования)
    pub fn selected(&self) -> Option<&str> {
        self.all.then_some(self.text.as_str())
    }
    /// снять выделение вместе с текстом; true — было что снимать
    fn take_all(&mut self) -> bool {
        if self.all {
            self.clear();
            true
        } else {
            false
        }
    }
    fn byte(&self, ci: usize) -> usize {
        self.text.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(self.text.len())
    }
    pub fn insert(&mut self, s: &str) {
        self.take_all();
        let b = self.byte(self.cur);
        let s: String = s.chars().filter(|c| !c.is_control()).collect();
        self.text.insert_str(b, &s);
        self.cur += s.chars().count();
    }
    pub fn backspace(&mut self, word: bool) {
        if self.take_all() || self.cur == 0 {
            return;
        }
        let chars: Vec<char> = self.text.chars().collect();
        let mut from = self.cur - 1;
        if word {
            while from > 0 && chars[from] == ' ' {
                from -= 1;
            }
            while from > 0 && chars[from - 1] != ' ' {
                from -= 1;
            }
        }
        let (a, b) = (self.byte(from), self.byte(self.cur));
        self.text.replace_range(a..b, "");
        self.cur = from;
    }
    pub fn delete(&mut self) {
        if self.take_all() {
            return;
        }
        let n = self.text.chars().count();
        if self.cur < n {
            let (a, b) = (self.byte(self.cur), self.byte(self.cur + 1));
            self.text.replace_range(a..b, "");
        }
    }
    pub fn left(&mut self) {
        // как в любом поле: стрелка снимает выделение и встаёт к его краю
        if std::mem::take(&mut self.all) {
            self.cur = 0;
            return;
        }
        self.cur = self.cur.saturating_sub(1);
    }
    pub fn right(&mut self) {
        self.all = false;
        self.cur = (self.cur + 1).min(self.text.chars().count());
    }
    pub fn home(&mut self) {
        self.all = false;
        self.cur = 0;
    }
    pub fn end(&mut self) {
        self.all = false;
        self.cur = self.text.chars().count();
    }
    pub fn clear(&mut self) {
        self.text.clear();
        self.cur = 0;
        self.all = false;
    }

    /// стереть (пароль): байты затираются нулями, а не просто отпускаются
    pub fn wipe(&mut self) {
        wipe(&mut self.text);
        self.cur = 0;
        self.all = false;
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tone {
    Plain,
    Primary,
    Danger,
    Ghost,
}

pub struct Ui<'a, 'b, A> {
    pub cv: Canvas<'b>,
    pub f: &'a mut Fonts,
    pub ic: &'a mut Icons,
    pub t: Theme,
    pub zones: Vec<Zone<A>>,
    pub ptr: Option<(f64, f64)>,
    pub down: bool,
    /// нужен ещё кадр (идёт анимация)
    pub anim: bool,
    /// мигание курсора ввода
    pub blink: bool,
    /// перевод надписей приложения (у каждого своя таблица)
    pub tr: fn(&str) -> &str,
}

/// действие зоны: каждое приложение со своим набором; `block` глушит клики по фону рамки
pub trait Action: Clone {
    fn block() -> Self;
}

impl<'a, 'b, A: Action> Ui<'a, 'b, A> {
    pub fn hover(&self, r: R) -> bool {
        let r = R::new(r.x + self.cv.ox, r.y + self.cv.oy, r.w, r.h);
        self.ptr.map(|(x, y)| r.contains(x, y)).unwrap_or(false)
    }
    /// зона щелчка; её часть за обрезкой холста не ловит ничего: строка, прокрученная под шапку
    /// списка, не перехватывает щелчки шапки
    pub fn zone(&mut self, r: R, act: A, cur: Cur) {
        self.cv.reach = self.cv.reach.max(r.y + r.h);
        let r = R::new(r.x + self.cv.ox, r.y + self.cv.oy, r.w, r.h).inter(&self.cv.clip_rect());
        if r.w > 0 && r.h > 0 {
            self.zones.push(Zone { r, act, cur });
        }
    }

    pub fn text(&mut self, face: Face, x: i32, y: i32, s: &str, c: C) -> i32 {
        let s = (self.tr)(s);
        self.cv.text(self.f, face, x, y, s, c)
    }
    pub fn text_in(&mut self, face: Face, r: R, s: &str, c: C) -> i32 {
        let s = (self.tr)(s);
        self.cv.text_in(self.f, face, r, s, c)
    }
    pub fn text_center(&mut self, face: Face, r: R, s: &str, c: C) -> i32 {
        let s = (self.tr)(s);
        self.cv.text_center(self.f, face, r, s, c)
    }
    pub fn text_right(&mut self, face: Face, r: R, s: &str, c: C) -> i32 {
        let s = (self.tr)(s);
        self.cv.text_right(self.f, face, r, s, c)
    }
    pub fn wrap(&mut self, face: Face, r: R, s: &str, c: C, lines: usize) -> i32 {
        let s = (self.tr)(s);
        self.cv.text_wrap(self.f, face, r, s, c, lines)
    }
    pub fn w(&mut self, face: Face, s: &str) -> i32 {
        let s = (self.tr)(s);
        self.f.width(face, s)
    }

    pub fn app_icon(&mut self, name: &str, x: i32, y: i32, size: i32, k: f32) {
        if let Some(img) = self.ic.app(name, size) {
            self.cv.image(&img, x, y, k);
        }
    }

    /// кнопка с фаской (px.rs Button): светлая грань сверху-слева, тёмная снизу-справа,
    /// жёсткая тень; при нажатии вдавливается на пиксель
    #[allow(clippy::too_many_arguments)]
    pub fn button(&mut self, r: R, text: &str, icon: &str, tone: Tone, selected: bool, enabled: bool, act: A) -> bool {
        let t = self.t;
        let hov = enabled && self.hover(r);
        let down = hov && self.down;
        let h = if hov { 1.0 } else { 0.0 };
        let mut base = match tone {
            Tone::Primary => t.red,
            Tone::Danger => t.blood,
            Tone::Ghost => CLEAR,
            Tone::Plain => t.surface,
        };
        if selected {
            base = t.blood;
        }
        if !enabled {
            base = base.lerp(t.surface, 0.7);
        }
        let fill = if tone == Tone::Ghost && !selected {
            t.selection.alpha(0.6 * h)
        } else if tone == Tone::Primary && !selected {
            base.lerp(t.ember, h * 0.45)
        } else {
            base.lerp(t.surface_hi, h)
        };
        // на глубоком акценте (выбранное, «опасные») — белый в обеих темах
        let on_deep = if crate::theme::lightness(fill) > 0.6 { crate::gfx::C::hex(0x0b0909) } else { crate::theme::WHITE };
        let fg = if !enabled {
            t.text_faint
        } else if tone == Tone::Ghost && !selected {
            t.text_dim.lerp(t.text_hi, h)
        } else if tone == Tone::Plain && !selected {
            t.text
        } else if tone == Tone::Primary && !selected {
            t.on_accent
        } else {
            on_deep
        };
        let d = if down { 1 } else { 0 };
        if tone != Tone::Ghost || selected || hov {
            let solid = tone != Tone::Ghost || selected;
            let outline = if tone == Tone::Plain && !selected { t.border.lerp(t.red, h) } else { CLEAR };
            let st = BoxStyle::new(fill, outline).shadow(if solid { 2 } else { 0 }).bevel(solid).pressed(down);
            self.cv.pxbox(R::new(r.x + d, r.y + d, r.w, r.h), &st);
        }
        // содержимое по центру
        let face = font::PX;
        let tw = if text.is_empty() { 0 } else { self.w(face, text) };
        let iw = if icon.is_empty() { 0 } else { 14 };
        let gap = if tw > 0 && iw > 0 { 7 } else { 0 };
        let total = tw + iw + gap;
        let mut x = r.x + (r.w - total) / 2 + d;
        if iw > 0 {
            self.cv.icon(icon, x, r.y + (r.h - 14) / 2 + d, 2, fg);
            x += iw + gap;
        }
        if tw > 0 {
            let avail = r.right() - x - 4;
            self.text_in(face, R::new(x, r.y + d, avail.max(0), r.h), text, fg);
        }
        if enabled {
            self.zone(r, act, Cur::Pointer);
        }
        hov
    }

    /// маленькая кнопка-иконка без подложки
    pub fn ghost(&mut self, r: R, icon: &str, act: A) -> bool {
        self.button(r, "", icon, Tone::Ghost, false, true, act)
    }

    /// пиксельный флажок: квадрат с «лампочкой» внутри и подпись
    pub fn toggle(&mut self, r: R, text: &str, hint: &str, on: bool, act: A) {
        let t = self.t;
        let hov = self.hover(r);
        let b = R::new(r.x, r.y + 4, 16, 16);
        self.cv.pxbox(b, &BoxStyle::new(t.code_bg, if hov { t.red } else { t.border }));
        if on {
            self.cv.fill(R::new(b.x + 4, b.y + 4, 8, 8), t.ember);
        }
        self.text_in(font::PX, R::new(r.x + 26, r.y + 2, r.w - 26, 20), text, if hov { t.text_hi } else { t.text });
        if !hint.is_empty() {
            self.text_in(font::PX, R::new(r.x + 26, r.y + 22, r.w - 26, 16), hint, t.text_faint);
        }
        self.zone(r, act, Cur::Pointer);
    }

    /// ползунок из клеток: подпись, значение справа, шкала под ними
    #[allow(clippy::too_many_arguments)]
    pub fn slider(&mut self, r: R, text: &str, v: f32, vtext: &str, segments: i32, act: A) {
        let t = self.t;
        self.text_in(font::PX, R::new(r.x, r.y, r.w - 80, 20), text, t.text);
        self.text_right(font::PX, R::new(r.x, r.y, r.w, 20), vtext, t.red);
        let g = R::new(r.x, r.y + 24, r.w, 12);
        let hov = self.hover(R::new(r.x, r.y + 18, r.w, 22));
        self.cv.gauge(g, segments, v, if hov { t.ember } else { t.red }, if hov { t.surface_hi } else { t.surface.lerp(t.border, 0.3) }, 2);
        self.zone(R::new(r.x, r.y + 18, r.w, 22), act, Cur::Pointer);
    }

    /// поле ввода: тёмное, рамка, при фокусе — акцент, курсор-уголь
    pub fn field(&mut self, r: R, f: &Field, placeholder: &str, focused: bool, password: bool, act: A) {
        let t = self.t;
        self.cv.pxbox(r, &BoxStyle::new(t.code_bg, if focused { t.red } else { t.border }));
        let inner = R::new(r.x + 10, r.y, r.w - 20, r.h);
        let face = font::PX;
        let shown: String = if password { "■".repeat(f.text.chars().count()) } else { f.text.clone() };
        let before: String = shown.chars().take(f.cur).collect();
        // введённое — как есть, без перевода интерфейса (набранное «сон» не станет «sleep»)
        let bw = self.f.width(face, &before);
        // прокрутка: курсор всегда виден
        let shift = (bw - (inner.w - 10)).max(0);
        self.cv.push_clip(inner);
        if shown.is_empty() {
            // в фокусе подсказка начинается за курсором, иначе он закрывает её первую букву
            let off = if focused { 12 } else { 0 };
            self.text_in(face, R::new(inner.x + off, inner.y, inner.w - off, inner.h), placeholder, t.text_faint);
        } else {
            let y = inner.y + (inner.h - self.f.line(face)) / 2;
            if f.all {
                // выделение — подложка цвета выделения под всем текстом
                let tw = self.f.width(face, &shown);
                self.cv.fill(R::new(inner.x - shift - 1, y - 1, tw + 2, self.f.line(face) + 2), t.selection);
            }
            self.cv.text(self.f, face, inner.x - shift, y, &shown, if f.all { t.text_hi } else { t.text });
        }
        if focused && self.blink {
            let lh = self.f.line(face);
            self.cv.fill(R::new(inner.x - shift + bw, r.y + (r.h - lh) / 2, 8, lh), t.ember);
        }
        self.cv.pop_clip();
        self.zone(r, act, Cur::Text);
    }

    /// шапка панели: квадрат-маркер, заголовок, справа — кнопки
    pub fn header(&mut self, r: R, icon: &str, title: &str, sub: &str) {
        let t = self.t;
        self.cv.header(&t, r);
        let mut x = r.x + 10;
        if icon.is_empty() {
            self.cv.fill(R::new(x, r.cy() - 3, 6, 6), t.red);
            x += 14;
        } else {
            self.cv.icon(icon, x, r.cy() - 7, 2, t.red);
            x += 22;
        }
        let w = self.text_in(font::PX, R::new(x, r.y, r.w - x - 40, r.h - 1), title, t.text);
        if !sub.is_empty() {
            self.text_in(font::PX, R::new(x + w + 8, r.y, r.w / 2, r.h - 1), sub, t.text_faint);
        }
    }

    /// рамка окна шелла: стекло, контур, тень
    pub fn frame(&mut self, r: R, accent: bool) {
        let t = self.t;
        self.cv.pxbox(r, &BoxStyle::new(t.glass_bg(), if accent { t.red } else { t.border }).shadow(6));
        self.zone(r, A::block(), Cur::Default);
    }
}

/// затереть строку нулями и очистить (пароли не остаются в отпущенной памяти)
pub fn wipe(s: &mut String) {
    // SAFETY: нули — допустимый UTF-8
    unsafe { std::ptr::write_bytes(s.as_mut_vec().as_mut_ptr(), 0, s.len()) };
    s.clear();
}

/// текст из буфера обмена для поля ввода: не дольше секунды и не больше 64 КБ — зависшая
/// программа-владелец буфера не должна замораживать окно
pub fn paste() -> Option<String> {
    use std::io::Read;
    let mut ch = std::process::Command::new("wl-paste").args(["-n", "-t", "text"]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
    let mut out = ch.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = (&mut out).take(64 * 1024).read_to_end(&mut b);
        let _ = tx.send(b);
    });
    let r = rx.recv_timeout(std::time::Duration::from_secs(1)).ok();
    let _ = ch.kill();
    let _ = ch.wait();
    r.map(|b| String::from_utf8_lossy(&b).replace(['\n', '\r'], " "))
}
