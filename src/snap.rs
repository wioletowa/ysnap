// Скриншоты: экран замораживается (grim), поверх — оверлей на каждом мониторе в пиксельном
// стиле: вуаль со строками развёртки, выделение «бегущими муравьями» с ручками, лупа
// с пиксельной сеткой и пипеткой, панель инструментов (перо, рамка, стрелка, маркер,
// мозаика, номерки), палитра из темы. Enter — готово: в буфер и/или в папку, уведомление
// с миниатюрой. Режимы: область, окно (по раскладке niri), экран. Таймер — 3/5/10 с.
//
//   ysnap [area|window|screen]   (Print, Alt+Print, Ctrl+Print в конфиге niri)

pub use crate::snapdraw::{draw, paint};
use crate::app::{App, Msg};
use crate::gfx::{Canvas, Img, C, R};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};
use smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Area,
    Window,
    Screen,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Pen,
    Rect,
    Arrow,
    Marker,
    Mosaic,
    Count,
    /// пипетка: щелчок копирует цвет точки как #rrggbb
    Pick,
}

pub const TOOLS: &[(Tool, &str, &str, &str)] = &[
    (Tool::Select, "cursor", "t:select", "выделение"),
    (Tool::Pen, "pen", "t:pen", "перо"),
    (Tool::Rect, "rect", "t:rect", "рамка"),
    (Tool::Arrow, "arrow", "t:arrow", "стрелка"),
    (Tool::Marker, "marker", "t:marker", "маркер"),
    (Tool::Mosaic, "mosaic", "t:mosaic", "мозаика"),
    (Tool::Count, "count", "t:count", "номерок"),
    (Tool::Pick, "drop", "t:pick", "пипетка"),
];
pub(crate) const COLOR_ACTS: [&str; 7] = ["c:0", "c:1", "c:2", "c:3", "c:4", "c:5", "c:6"];
/// меню «другие цвета»: набор и недавние (config::RECENT)
pub const MORE: [u32; 16] = [
    0xe02424, 0xf06a1d, 0xf5c518, 0x4caf50, 0x1fb5a8, 0x2f80ed, 0x7b4fe0, 0xd63aa0, //
    0xffffff, 0xc8c8c8, 0x8a8a8a, 0x4a4a4a, 0x0b0909, 0x8b5a2b, 0x9b0000, 0x00334d,
];
pub(crate) const MORE_ACTS: [&str; 16] = ["p:0", "p:1", "p:2", "p:3", "p:4", "p:5", "p:6", "p:7", "p:8", "p:9", "p:10", "p:11", "p:12", "p:13", "p:14", "p:15"];
pub(crate) const RECENT_ACTS: [&str; crate::config::RECENT] = ["r:0", "r:1", "r:2", "r:3", "r:4", "r:5", "r:6", "r:7"];

#[derive(Clone, Debug)]
pub struct Mark {
    pub tool: Tool,
    pub pts: Vec<(i32, i32)>,
    pub color: C,
    pub size: i32,
    pub n: u32,
    /// готовое лёгкое размытие (snapdraw::blur): ключ — область, сдвиг, масштаб, радиус
    pub cache: std::cell::RefCell<Option<(BlurKey, Vec<u32>)>>,
}

pub type BlurKey = (R, i32, i32, (i32, i32), u32, u32, i32);

#[derive(Clone, Copy, Debug)]
pub(crate) enum Drag {
    New(i32, i32),
    Move(i32, i32, R),
    /// ручка: 0..8 по часовой с левого верхнего угла, исходный прямоугольник
    Handle(u8, R),
    Draw,
}

pub struct Snap {
    pub imgs: HashMap<String, Rc<Img>>,
    pub mode: Mode,
    pub sel: Option<(String, R)>,
    pub(crate) drag: Option<Drag>,
    pub tool: Tool,
    /// кнопка цвета в панели; usize::MAX — цвет из меню
    pub color: usize,
    /// цвет новых меток
    pub cur: C,
    /// открыто меню «другие цвета»
    pub menu: bool,
    pub size: i32,
    pub marks: Vec<Mark>,
    pub redo: Vec<Mark>,
    pub ptr: Option<(String, i32, i32)>,
    pub flash: Option<Instant>,
    /// окна на видимых рабочих столах: монитор, рамка, заголовок
    pub wins: Vec<(String, R, String)>,
    pub busy: bool,
    pub hint: Option<(String, Instant)>,
    /// есть tesseract / zbarimg: кнопки «текст» и «QR» в панели
    pub ocr: bool,
    pub qr: bool,
    /// окно в фокусе до оверлея (кнопка «окно»: его снимок с рамкой и тенью)
    pub focus_win: Option<u64>,
}

impl Snap {
    pub fn colors(t: &crate::theme::Theme) -> [C; 7] {
        [C::hex(0xe02424), t.red, C::hex(0xffffff), C::hex(0x0b0909), t.ok, t.warn, C::hex(0x3a9fd0)]
    }
    pub(crate) fn cur_sel(&self, out: &str) -> Option<R> {
        self.sel.as_ref().filter(|s| s.0 == out).map(|s| s.1)
    }
}

/// grim -t ppm: P6, ширина, высота, 255, RGB
pub(crate) fn parse_ppm(b: &[u8]) -> Option<Img> {
    let mut fields = vec![];
    let mut i = 0;
    while fields.len() < 4 && i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b.get(i) == Some(&b'#') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let s = i;
        while i < b.len() && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        fields.push(String::from_utf8_lossy(&b[s..i]).to_string());
    }
    i += 1;
    if fields.first().map(|f| f != "P6").unwrap_or(true) {
        return None;
    }
    let w: i32 = fields.get(1)?.parse().ok()?;
    let h: i32 = fields.get(2)?.parse().ok()?;
    let data = b.get(i..i + (w * h * 3) as usize)?;
    let px = data.chunks_exact(3).map(|p| 0xff000000 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32).collect();
    Some(Img { w, h, px })
}

/// монитор через grim (запасной путь снимка); cursor — с курсором (-c)
pub fn grab(out: &str, cursor: bool) -> Option<Img> {
    let mut c = std::process::Command::new("grim");
    if cursor {
        c.arg("-c");
    }
    let o = c.args(["-o", out, "-t", "ppm", "-"]).output().ok()?;
    if !o.status.success() {
        return None;
    }
    parse_ppm(&o.stdout)
}

/// окна видимых рабочих столов в координатах их мониторов
pub(crate) fn niri_windows(ws: &serde_json::Value, wins: &serde_json::Value) -> Vec<(String, R, String)> {
    let mut active: HashMap<u64, String> = HashMap::new();
    for w in ws.as_array().into_iter().flatten() {
        if w["is_active"].as_bool().unwrap_or(false) {
            if let (Some(id), Some(out)) = (w["id"].as_u64(), w["output"].as_str()) {
                active.insert(id, out.to_string());
            }
        }
    }
    let mut v = vec![];
    for w in wins.as_array().into_iter().flatten() {
        let Some(out) = w["workspace_id"].as_u64().and_then(|i| active.get(&i)) else { continue };
        let l = &w["layout"];
        let (Some(px), Some(py)) = (l["tile_pos_in_workspace_view"][0].as_f64(), l["tile_pos_in_workspace_view"][1].as_f64()) else { continue };
        let ox = l["window_offset_in_tile"][0].as_f64().unwrap_or(0.0);
        let oy = l["window_offset_in_tile"][1].as_f64().unwrap_or(0.0);
        let ww = l["window_size"][0].as_f64().unwrap_or(0.0);
        let wh = l["window_size"][1].as_f64().unwrap_or(0.0);
        let r = R::new((px + ox).round() as i32, (py + oy).round() as i32, ww as i32, wh as i32);
        // сверху — фокусное и плавающие
        let title = w["title"].as_str().unwrap_or("").to_string();
        if w["is_floating"].as_bool().unwrap_or(false) || w["is_focused"].as_bool().unwrap_or(false) {
            v.insert(0, (out.clone(), r, title));
        } else {
            v.push((out.clone(), r, title));
        }
    }
    v
}

/// конец прямой от a к b, повёрнутой к ближайшему из углов через 45°
pub(crate) fn snap45(a: (i32, i32), b: (i32, i32)) -> (i32, i32) {
    let (dx, dy) = ((b.0 - a.0) as f32, (b.1 - a.1) as f32);
    let step = std::f32::consts::FRAC_PI_4;
    let ang = (dy.atan2(dx) / step).round() * step;
    let len = (dx * dx + dy * dy).sqrt();
    // по диагонали — длина по проекции, чтобы конец шёл за курсором
    let len = if (ang / step).round() as i32 % 2 != 0 { (dx.abs() + dy.abs()) / 2.0 * std::f32::consts::SQRT_2 } else { len };
    (a.0 + (len * ang.cos()).round() as i32, a.1 + (len * ang.sin()).round() as i32)
}

pub(crate) fn norm(x0: i32, y0: i32, x1: i32, y1: i32) -> R {
    R::new(x0.min(x1), y0.min(y1), (x1 - x0).abs() + 1, (y1 - y0).abs() + 1)
}

pub(crate) fn handles(r: R) -> [R; 8] {
    let s = 8;
    let (x0, x1, xm) = (r.x - s / 2, r.right() - s / 2, r.x + r.w / 2 - s / 2);
    let (y0, y1, ym) = (r.y - s / 2, r.bottom() - s / 2, r.y + r.h / 2 - s / 2);
    [R::new(x0, y0, s, s), R::new(xm, y0, s, s), R::new(x1, y0, s, s), R::new(x1, ym, s, s), R::new(x1, y1, s, s), R::new(xm, y1, s, s), R::new(x0, y1, s, s), R::new(x0, ym, s, s)]
}

// ── поведение ──────────────────────────────────────────────────────────────

pub(crate) fn copy_text(s: &str) {
    use std::io::Write;
    if let Ok(mut ch) = std::process::Command::new("wl-copy").stdin(std::process::Stdio::piped()).spawn() {
        if let Some(i) = ch.stdin.as_mut() {
            let _ = i.write_all(s.as_bytes());
        }
        drop(ch.stdin.take());
        let _ = ch.wait();
    }
}

/// снимок сделан (flash): оверлей сразу прозрачен и закрывается; курсор в углу — ждём, пока уйдёт,
/// не дольше стольких мс
const HOLD: u128 = 4000;

/// курсор у угла экрана (горячий угол niri — пиксель в углу; берём с запасом)
fn in_corner(x: i32, y: i32, w: i32, h: i32) -> bool {
    let near = |v: i32, max: i32| v <= 2 || v >= max - 3;
    near(x, w) && near(y, h)
}

/// PNG в файл (если путь есть) и/или в буфер; Some — ошибка записи
pub(crate) fn store(png: &[u8], path: Option<&std::path::Path>, copy: bool) -> Option<String> {
    let mut err = None;
    if let Some(p) = path {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Err(e) = std::fs::write(p, png) {
            err = Some(e.to_string());
        }
    }
    if copy {
        use std::io::Write;
        if let Ok(mut ch) = std::process::Command::new("wl-copy").args(["-t", "image/png"]).stdin(std::process::Stdio::piped()).spawn() {
            if let Some(i) = ch.stdin.as_mut() {
                let _ = i.write_all(png);
            }
            drop(ch.stdin.take());
            let _ = ch.wait();
        }
    }
    err
}

pub(crate) enum Picked {
    Path(PathBuf),
    Cancel,
    NoDialog,
}

/// «сохранить как…»: zenity, иначе kdialog
pub(crate) fn pick_save(dir: &std::path::Path, name: &str) -> Picked {
    let start = dir.join(name);
    let mut c = if crate::which("zenity") {
        let mut c = std::process::Command::new("zenity");
        c.args(["--file-selection", "--save", "--confirm-overwrite", "--file-filter=PNG | *.png"]).arg(format!("--filename={}", start.display()));
        c
    } else if crate::which("kdialog") {
        let mut c = std::process::Command::new("kdialog");
        c.arg("--getsavefilename").arg(&start).arg("*.png");
        c
    } else {
        return Picked::NoDialog;
    };
    let out = c.stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output();
    match out.ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|p| !p.is_empty()) {
        Some(p) => Picked::Path(PathBuf::from(p)),
        None => Picked::Cancel,
    }
}


impl App {
    pub fn on_snap_ready(&mut self, imgs: HashMap<String, Img>, wins: Vec<(String, R, String)>, mode: Mode, focus: String, focus_win: Option<u64>) {
        self.waiting = false;
        if imgs.is_empty() {
            self.toast_note("скриншот не вышел", "нужен grim (пакет grim)");
            return;
        }
        let outs: Vec<String> = imgs.keys().cloned().collect();
        let mut sn = Snap { imgs: imgs.into_iter().map(|(k, v)| (k, Rc::new(v))).collect(), mode, sel: None, drag: None, tool: Tool::Select, color: 0, cur: Snap::colors(&self.st.t)[0], menu: false, size: 4, marks: vec![], redo: vec![], ptr: None, flash: None, wins, busy: false, hint: None, ocr: crate::which("tesseract"), qr: crate::which("zbarimg"), focus_win };
        // фокус niri неизвестен — первый монитор
        let focus = if sn.imgs.contains_key(&focus) { focus } else { outs[0].clone() };
        if mode == Mode::Screen {
            let (w, h) = self.output_size(&focus);
            sn.sel = Some((focus.clone(), R::new(0, 0, w, h)));
        }
        self.st.snap = Some(sn);
        // мгновенный режим: снимок экрана — без оверлея, сразу в буфер и/или в папку
        if mode == Mode::Screen && (self.instant || self.st.set.shot_confirm == "instant") {
            self.snap_done();
            self.snap_close();
            return;
        }
        for o in outs {
            let kb = if o == focus { KeyboardInteractivity::Exclusive } else { KeyboardInteractivity::None };
            self.mk_snap(&o, kb);
        }
    }

    pub fn snap_close(&mut self) {
        self.st.snap = None;
        self.destroy_all();
    }

    fn snap_hint(&mut self, s: &str) {
        if let Some(sn) = &mut self.st.snap {
            sn.hint = Some((s.to_string(), Instant::now()));
        }
    }

    /// уведомление — после выхода из цикла (main.rs: notify-send)
    pub fn toast_note(&mut self, summary: &str, body: &str) {
        self.notes.push(crate::Note { summary: crate::i18n::tr(summary).into(), body: crate::i18n::tr(body).into(), open: None });
    }

    /// мышь над оверлеем; true — событие наше
    pub fn snap_pointer(&mut self, sid: u32, kind: u8, x: f64, y: f64, button: u32, on_btn: bool) -> bool {
        let Some(out) = self.surfs.get(&sid).map(|s| s.output.clone()) else { return false };
        let (w, h) = self.surfs.get(&sid).map(|s| (s.w, s.h)).unwrap_or((0, 0));
        let shift = self.mods.shift;
        let Some(sn) = &mut self.st.snap else { return false };
        let (xi, yi) = ((x as i32).clamp(0, w - 1), (y as i32).clamp(0, h - 1));
        if sn.flash.is_some() || sn.busy {
            // снимок сделан: курсор нужен только snap_tick — ушёл ли он из угла
            sn.ptr = Some((out, xi, yi));
            return true;
        }
        let prev = sn.ptr.as_ref().map(|p| p.0.clone()).filter(|p| *p != out);
        sn.ptr = Some((out.clone(), xi, yi));
        self.dirty_out(&out);
        if let Some(p) = prev {
            self.dirty_out(&p);
        }
        let Some(sn) = &mut self.st.snap else { return false };
        match kind {
            // движение
            0 => {
                let Some(d) = sn.drag else { return false };
                match d {
                    Drag::New(x0, y0) => sn.sel = Some((out, norm(x0, y0, xi, yi))),
                    Drag::Move(dx, dy, r0) => {
                        let nx = (xi - dx).clamp(0, w - r0.w);
                        let ny = (yi - dy).clamp(0, h - r0.h);
                        sn.sel = Some((out, R::new(nx, ny, r0.w, r0.h)));
                    }
                    Drag::Handle(k, r0) => {
                        let (mut x0, mut y0, mut x1, mut y1) = (r0.x, r0.y, r0.right() - 1, r0.bottom() - 1);
                        match k {
                            0 => (x0, y0) = (xi, yi),
                            1 => y0 = yi,
                            2 => (x1, y0) = (xi, yi),
                            3 => x1 = xi,
                            4 => (x1, y1) = (xi, yi),
                            5 => y1 = yi,
                            6 => (x0, y1) = (xi, yi),
                            _ => x0 = xi,
                        }
                        sn.sel = Some((out, norm(x0, y0, x1, y1)));
                    }
                    Drag::Draw => {
                        if let Some(m) = sn.marks.last_mut() {
                            if m.tool == Tool::Pen && shift {
                                // Shift: прямая от начала штриха, по 45°
                                let a = m.pts[0];
                                m.pts = vec![a, snap45(a, (xi, yi))];
                            } else if m.tool == Tool::Pen {
                                if m.pts.last() != Some(&(xi, yi)) {
                                    m.pts.push((xi, yi));
                                }
                            } else if m.pts.len() >= 2 {
                                m.pts[1] = (xi, yi);
                            } else {
                                m.pts.push((xi, yi));
                            }
                        }
                    }
                }
                true
            }
            // нажатие
            1 => {
                if on_btn {
                    return false;
                }
                // щелчок мимо меню цветов закрывает его, и только
                if sn.menu {
                    sn.menu = false;
                    return true;
                }
                if button == 0x111 {
                    // ПКМ: снять выделение (метки остаются) / выйти
                    if sn.sel.is_some() && sn.mode != Mode::Screen {
                        sn.sel = None;
                    } else {
                        self.snap_close();
                    }
                    return true;
                }
                let sel = sn.cur_sel(&out);
                if sn.tool == Tool::Pick {
                    // цвет точки снимка: снимок в физических пикселях, масштаб бывает дробным (1.25)
                    if let Some(img) = sn.imgs.get(&out) {
                        let px = ((x * img.w as f64 / w.max(1) as f64) as i32).clamp(0, img.w - 1);
                        let py = ((y * img.h as f64 / h.max(1) as f64) as i32).clamp(0, img.h - 1);
                        let c = img.px[(py * img.w + px) as usize] & 0xffffff;
                        copy_text(&format!("#{c:06x}"));
                        self.st.set.remember(C::hex(c));
                        self.snap_hint(&crate::tf!("цвет #{:06x} скопирован", "colour #{:06x} copied", "cor #{:06x} copiada", c));
                    }
                    return true;
                }
                if let Some(r) = sel {
                    if let Some(k) = handles(r).iter().position(|hr| hr.inset(-3).contains(x, y)) {
                        sn.drag = Some(Drag::Handle(k as u8, r));
                        return true;
                    }
                    if sn.tool == Tool::Select && r.contains(x, y) {
                        sn.drag = Some(Drag::Move(xi - r.x, yi - r.y, r));
                        return true;
                    }
                }
                // инструмент рисования: где угодно на мониторе выделения, и за рамкой тоже (в файл
                // попадёт то, что внутри неё)
                if sn.tool != Tool::Select {
                    if sn.sel.as_ref().is_some_and(|s| s.0 == out) {
                        let n = sn.marks.iter().filter(|m| m.tool == Tool::Count).count() as u32 + 1;
                        sn.marks.push(Mark { tool: sn.tool, pts: vec![(xi, yi)], color: sn.cur, size: if sn.tool == Tool::Marker { 14 } else { sn.size }, n, cache: Default::default() });
                        sn.redo.clear();
                        // номерок тоже тянется: протяжка — стрелка от него к курсору
                        sn.drag = Some(Drag::Draw);
                    }
                    // выделение на другом мониторе: его метки здесь не рисуются — щелчок ничего не делает
                    if sn.sel.is_some() {
                        return true;
                    }
                }
                // новое выделение; метки (в координатах экрана) остаются
                match sn.mode {
                    Mode::Screen => sn.sel = Some((out.clone(), R::new(0, 0, w, h))),
                    _ => {
                        sn.sel = None;
                        sn.drag = Some(Drag::New(xi, yi));
                    }
                }
                true
            }
            // отпускание
            _ => {
                let Some(d) = sn.drag.take() else { return false };
                let picked = matches!(d, Drag::New(..));
                if let Drag::New(x0, y0) = d {
                    // щелчок без протяжки: окно под курсором (в любом режиме), иначе — экран
                    if (x0 - xi).abs() < 4 && (y0 - yi).abs() < 4 {
                        let win = sn.wins.iter().find(|(o, r, _)| *o == out && r.contains(x, y)).map(|(_, r, _)| r.inter(&R::new(0, 0, w, h)));
                        sn.sel = match (sn.mode, win) {
                            (Mode::Window, Some(r)) | (Mode::Area, Some(r)) => Some((out, r)),
                            _ => Some((out, R::new(0, 0, w, h))),
                        };
                    }
                }
                if let Drag::Draw = d {
                    // нулевая рамка/стрелка — убрать
                    if let Some(m) = sn.marks.last_mut() {
                        if m.tool != Tool::Pen && m.tool != Tool::Count && m.pts.len() < 2 {
                            sn.marks.pop();
                        } else if m.tool == Tool::Count && m.pts.len() >= 2 {
                            // протянул не дальше края номерка — просто номерок
                            let (a, b) = (m.pts[0], m.pts[1]);
                            let r = crate::snapdraw::count_side(m.size) / 2 + 4;
                            if (b.0 - a.0).pow(2) + (b.1 - a.1).pow(2) <= r * r {
                                m.pts.truncate(1);
                            }
                        }
                    }
                }
                // мгновенный режим: выделил (протянул или щёлкнул по окну) — снимок готов
                if picked && sn.sel.is_some() && self.st.set.shot_confirm == "instant" {
                    self.snap_done();
                }
                true
            }
        }
    }

    pub fn snap_wheel(&mut self, d: i32) {
        if let Some(sn) = &mut self.st.snap {
            let sizes = [2, 4, 8];
            let i = sizes.iter().position(|&s| s == sn.size).unwrap_or(1) as i32;
            sn.size = sizes[(i - d).clamp(0, 2) as usize];
            self.dirty_all();
        }
    }

    pub fn snap_btn(&mut self, a: &str) {
        let Some(sn) = &mut self.st.snap else { return };
        if let Some(t) = TOOLS.iter().find(|t| t.2 == a) {
            sn.tool = t.0;
        } else if let Some(c) = a.strip_prefix("c:") {
            let colors = Snap::colors(&self.st.t);
            sn.color = c.parse::<usize>().unwrap_or(0).min(colors.len() - 1);
            sn.cur = colors[sn.color];
            sn.menu = false;
            if sn.tool == Tool::Select || sn.tool == Tool::Mosaic || sn.tool == Tool::Pick {
                sn.tool = Tool::Pen;
            }
        } else if let Some(c) = a.strip_prefix("p:").and_then(|i| MORE.get(i.parse::<usize>().ok()?)).map(|c| C::hex(*c)).or_else(|| a.strip_prefix("r:").and_then(|i| self.st.set.recent.get(i.parse::<usize>().ok()?).copied())) {
            // цвет из меню: в недавние; совпал с кнопкой панели — подсветить её
            sn.color = Snap::colors(&self.st.t).iter().position(|x| x.0 & 0xffffff == c.0 & 0xffffff).unwrap_or(usize::MAX);
            sn.cur = c;
            sn.menu = false;
            if sn.tool == Tool::Select || sn.tool == Tool::Mosaic || sn.tool == Tool::Pick {
                sn.tool = Tool::Pen;
            }
            self.st.set.remember(c);
        } else if let Some(s) = a.strip_prefix("s:") {
            sn.size = s.parse().unwrap_or(4);
        } else {
            match a {
                "more" => sn.menu = !sn.menu,
                "m:area" => sn.mode = Mode::Area,
                "m:window" => {
                    let id = sn.focus_win;
                    self.snap_close();
                    self.window_shot(id);
                    return;
                }
                "m:screen" => sn.mode = Mode::Screen,
                "undo" => {
                    if let Some(m) = sn.marks.pop() {
                        sn.redo.push(m);
                    }
                }
                "copy" => self.snap_finish(true, false, false),
                "save" => {
                    self.snap_done();
                }
                "saveas" => self.snap_finish(false, false, true),
                "ocr" | "qr" => self.snap_read(a == "qr"),
                "close" => self.snap_close(),
                _ => {}
            }
        }
        self.dirty_all();
    }

    pub fn snap_key(&mut self, e: &KeyEvent) {
        let (ctrl, shift) = (self.mods.ctrl, self.mods.shift);
        let ptr_w = self.st.snap.as_ref().and_then(|s| s.ptr.as_ref()).map(|p| self.output_size(&p.0).0).unwrap_or(1);
        let Some(sn) = &mut self.st.snap else { return };
        // снимок уже сделан (вспышка) или ждёт диалога: второй пробел, enter или автоповтор
        // клавиши не делают второй снимок (#13)
        if sn.flash.is_some() || sn.busy {
            return;
        }
        let raw = e.raw_code;
        match e.keysym {
            Keysym::Escape => {
                self.snap_close();
                return;
            }
            // лупа у курсора: вкл/выкл (запоминается в конфиге)
            Keysym::space => self.st.set.toggle_loupe(),
            Keysym::Return | Keysym::KP_Enter => {
                if sn.sel.is_none() {
                    // окно под курсором или весь монитор
                    let p = sn.ptr.clone();
                    let out = p.as_ref().map(|p| p.0.clone()).unwrap_or_else(|| sn.imgs.keys().next().cloned().unwrap_or_default());
                    let win = p.as_ref().and_then(|(o, x, y)| sn.wins.iter().find(|(wo, r, _)| wo == o && r.contains(*x as f64, *y as f64))).map(|w| w.1);
                    let full = sn.imgs.get(&out).map(|i| R::new(0, 0, i.w, i.h)).unwrap_or_default();
                    sn.sel = Some((out, if sn.mode == Mode::Window { win.unwrap_or(full) } else { full }));
                }
                self.snap_done();
                return;
            }
            Keysym::Left | Keysym::Right | Keysym::Up | Keysym::Down => {
                if let Some((_, r)) = &mut sn.sel {
                    let d = if shift { 10 } else { 1 };
                    let (dx, dy) = match e.keysym {
                        Keysym::Left => (-d, 0),
                        Keysym::Right => (d, 0),
                        Keysym::Up => (0, -d),
                        _ => (0, d),
                    };
                    if ctrl {
                        r.w = (r.w + dx).max(4);
                        r.h = (r.h + dy).max(4);
                    } else {
                        r.x += dx;
                        r.y += dy;
                    }
                }
            }
            Keysym::Tab => {
                sn.mode = match sn.mode {
                    Mode::Area => Mode::Window,
                    Mode::Window => Mode::Screen,
                    Mode::Screen => Mode::Area,
                };
                let m = sn.mode;
                self.snap_hint(match m {
                    Mode::Area => "область",
                    Mode::Window => "окно",
                    Mode::Screen => "экран",
                });
            }
            _ if ctrl => match raw {
                44 => {
                    // ctrl+z / ctrl+shift+z
                    if shift {
                        if let Some(m) = sn.redo.pop() {
                            sn.marks.push(m);
                        }
                    } else if let Some(m) = sn.marks.pop() {
                        sn.redo.push(m);
                    }
                }
                21 => {
                    if let Some(m) = sn.redo.pop() {
                        sn.marks.push(m);
                    }
                }
                46 => {
                    if sn.sel.is_some() {
                        self.snap_finish(true, false, false);
                        return;
                    }
                    // без выделения — цвет под курсором
                    if let Some((o, x, y)) = sn.ptr.clone() {
                        if let Some(img) = sn.imgs.get(&o) {
                            let k = img.w as f32 / ptr_w.max(1) as f32;
                            let (cx, cy) = ((x as f32 * k) as i32, (y as f32 * k) as i32);
                            let c = img.px[(cy.clamp(0, img.h - 1) * img.w + cx.clamp(0, img.w - 1)) as usize] & 0xffffff;
                            copy_text(&format!("#{c:06x}"));
                            self.snap_hint(&crate::tf!("цвет #{:06x} скопирован", "colour #{:06x} copied", "cor #{:06x} copiada", c));
                        }
                    }
                }
                31 => {
                    self.snap_finish(false, false, true);
                    return;
                }
                _ => {}
            },
            _ => {
                // 1–8: инструменты
                if (2..=(TOOLS.len() as u32 + 1)).contains(&raw) {
                    sn.tool = TOOLS[(raw - 2) as usize].0;
                } else if raw == 33 && sn.sel.is_none() {
                    // f — весь монитор под курсором
                    sn.mode = Mode::Screen;
                } else if raw == 17 {
                    sn.mode = Mode::Window;
                }
            }
        }
        self.dirty_all();
    }

    /// собрать картинку: вырезать, нанести метки, закодировать в PNG
    fn snap_render(&mut self) -> Option<Vec<u8>> {
        use image::ImageEncoder as _;
        let sn = self.st.snap.as_ref()?;
        let (out, r) = sn.sel.clone()?;
        let img = sn.imgs.get(&out)?.clone();
        // снимок — в физических пикселях, выделение — в точках экрана; масштаб бывает дробным
        // (1.25, 1.5): целая часть отношения вырезала бы не тот кусок
        let (ow, _) = self.output_size(&out);
        let k = img.w as f32 / ow.max(1) as f32;
        let px = |v: i32| (v as f32 * k).round() as i32;
        let (x0, y0) = (px(r.x).clamp(0, img.w), px(r.y).clamp(0, img.h));
        let (x1, y1) = (px(r.right()).clamp(0, img.w), px(r.bottom()).clamp(0, img.h));
        let (w, h) = (x1 - x0, y1 - y0);
        if w <= 0 || h <= 0 {
            return None;
        }
        let mut buf = vec![0u32; (w * h) as usize];
        {
            let mut cv = Canvas::new(&mut buf, w, h);
            cv.blit(&img, -x0, -y0);
            for m in &sn.marks {
                paint(&mut cv, self.res.fonts(), m, &img, (r.x, r.y), k, k, self.st.t.on_accent);
            }
        }
        let mut rgba = Vec::with_capacity(buf.len() * 4);
        for p in &buf {
            rgba.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255]);
        }
        let mut png = vec![];
        image::codecs::png::PngEncoder::new(&mut png).write_image(&rgba, w as u32, h as u32, image::ExtendedColorType::Rgba8).ok()?;
        Some(png)
    }

    /// распознать выделенное: текст (ocr.rs: подготовка картинки и три прогона tesseract) или QR
    /// (zbarimg); результат — в буфер обмена и уведомлением. Оверлей закрывается сразу, работа — в нити.
    pub fn snap_read(&mut self, qr: bool) {
        let Some(png) = self.snap_render() else { return };
        self.snap_close();
        let tx = self.tx.clone();
        self.jobs += 1;
        std::thread::spawn(move || {
            if !qr {
                let text = image::load_from_memory(&png).ok().and_then(|i| crate::ocr::recognize(&i));
                if let Some(t) = &text {
                    copy_text(t);
                }
                let _ = tx.send(Msg::SnapText(false, text));
                return;
            }
            // снимок экрана — только в свой каталог 0700, не в общий /tmp
            let Ok(dir) = ysnap_px::paths::private_tmp("ysnap-read") else {
                let _ = tx.send(Msg::SnapText(qr, None));
                return;
            };
            let tmp = dir.join("shot.png");
            if std::fs::write(&tmp, &png).is_err() {
                let _ = std::fs::remove_dir_all(&dir);
                let _ = tx.send(Msg::SnapText(qr, None));
                return;
            }
            let out = std::process::Command::new("zbarimg").args(["--raw", "-q"]).arg(&tmp).output();
            let _ = std::fs::remove_dir_all(&dir);
            let text = out.ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|t| !t.is_empty());
            if let Some(t) = &text {
                copy_text(t);
            }
            let _ = tx.send(Msg::SnapText(qr, text));
        });
    }

    pub fn on_snap_text(&mut self, qr: bool, text: Option<String>) {
        let title = if qr { "QR-код" } else { "текст со снимка" };
        match text {
            Some(t) => {
                let short: String = t.chars().take(160).collect();
                self.toast_note(&crate::tf!("{} — в буфере", "{}: copied", "{}: copiado", crate::i18n::tr(title)), &short);
            }
            None => self.toast_note(title, if qr { "QR-код не найден" } else { "текст не распознан" }),
        }
    }

    /// готово по настройкам (save: буфер, папка или оба)
    fn snap_done(&mut self) {
        let s = self.st.set.save;
        self.snap_finish(s.copy(), s.file(), false);
    }

    /// готово: copy — в буфер, save — в папку, ask — спросить место диалогом (zenity или kdialog)
    pub fn snap_finish(&mut self, copy: bool, save: bool, ask: bool) {
        // один снимок на один оверлей: кнопка панели, клавиша и мгновенный режим сходятся сюда
        if self.st.snap.as_ref().is_some_and(|s| s.flash.is_some() || s.busy) {
            return;
        }
        let Some(png) = self.snap_render() else { return };
        let (dir, name) = self.st.set.target();
        let tx = self.tx.clone();
        if let Some(sn) = &mut self.st.snap {
            sn.busy = ask;
            if !ask {
                sn.flash = Some(Instant::now());
            }
        }
        if ask {
            // диалог должен быть виден: убираем оверлей сразу
            self.snap_close();
        }
        self.dirty_all();
        // вспышки нет: оверлей закрывается сразу (если курсор не в горячем углу)
        self.snap_tick();
        let notify = self.st.set.shot_notify;
        self.jobs += 1;
        std::thread::spawn(move || {
            let mut path: Option<PathBuf> = None;
            if ask {
                let _ = std::fs::create_dir_all(&dir);
                match pick_save(&dir, &name) {
                    // диалога нет — в папку снимков, как «сохранить»
                    Picked::NoDialog => path = Some(dir.join(&name)),
                    Picked::Path(p) => path = Some(p),
                    Picked::Cancel => {
                        let _ = tx.send(Msg::SnapDone { path: None, copied: false, err: None, notify: false });
                        return;
                    }
                }
            } else if save {
                path = Some(dir.join(&name));
            }
            let err = store(&png, path.as_deref(), copy);
            let _ = tx.send(Msg::SnapDone { path, copied: copy, err, notify });
        });
    }

    pub fn on_snap_done(&mut self, path: Option<PathBuf>, copied: bool, err: Option<String>, notify: bool) {
        if let Some(e) = err {
            self.toast_note("скриншот не сохранился", &e);
            return;
        }
        if !notify || (path.is_none() && !copied) {
            return;
        }
        let summary = match (&path, copied) {
            (Some(_), true) => crate::i18n::tr("скриншот сохранён и скопирован"),
            (Some(_), false) => crate::i18n::tr("скриншот сохранён"),
            _ => crate::i18n::tr("скриншот в буфере обмена"),
        };
        let body = path.as_deref().map(ysnap_px::paths::pretty).unwrap_or_default();
        self.notes.push(crate::Note { summary: summary.into(), body, open: path });
    }

    /// раз в тик: вспышка догорела — закрыть оверлей. Курсор в углу экрана — подождать, пока уйдёт
    /// (не дольше HOLD): там горячий угол niri, и оверлей, пока он под курсором, его глушит. Без
    /// оверлея первое же движение мыши в углу открыло бы обзор — так и было в мгновенном режиме,
    /// когда выделение тянут до угла экрана (#15)
    pub fn snap_tick(&mut self) {
        let Some(sn) = &self.st.snap else { return };
        let Some(ms) = sn.flash.map(|f| f.elapsed().as_millis()) else { return };
        let cornered = sn.ptr.as_ref().is_some_and(|(o, x, y)| {
            let (w, h) = self.output_size(o);
            in_corner(*x, *y, w, h)
        });
        if !cornered || ms > HOLD {
            self.snap_close();
        }
    }
}
