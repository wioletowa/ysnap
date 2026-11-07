// Отрисовка оверлея скриншотов: метки (общие для экрана и сохранённого файла), замороженный
// экран с выделением, лупа, верхняя панель и панель инструментов с подсказками.

use crate::app::State;
use crate::font::PX;
use crate::gfx::{BoxStyle, Canvas, Img, C, R};
use crate::snap::*;
use crate::ui::{Act, Cur, Tone, Ui};

// ── рисование меток (и на экране, и в файле) ───────────────────────────────
/// квадратная кисть size по линии Брезенхэма
pub(crate) fn line(cv: &mut Canvas, a: (i32, i32), b: (i32, i32), size: i32, c: C) {
    let (mut x, mut y) = a;
    let dx = (b.0 - x).abs();
    let dy = -(b.1 - y).abs();
    let sx = if x < b.0 { 1 } else { -1 };
    let sy = if y < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let h = size / 2;
    loop {
        cv.set(R::new(x - h, y - h, size, size), c);
        if x == b.0 && y == b.1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// метка m; координаты экрана → буфер: (p - o) · k. sb — пикселей снимка на точку экрана (масштаб
/// монитора, бывает дробным: 1.25); на экране k = 1, в файле k = sb
pub fn paint(cv: &mut Canvas, fonts: &mut crate::font::Fonts, m: &Mark, base: &Img, o: (i32, i32), k: f32, sb: f32, on_accent: C) {
    let sc = |v: i32| (v as f32 * k).round() as i32;
    let tr = |p: (i32, i32)| (sc(p.0 - o.0), sc(p.1 - o.1));
    let size = sc(m.size).max(1);
    match m.tool {
        Tool::Pen => {
            for w in m.pts.windows(2) {
                line(cv, tr(w[0]), tr(w[1]), size, m.color);
            }
            if m.pts.len() == 1 {
                line(cv, tr(m.pts[0]), tr(m.pts[0]), size, m.color);
            }
        }
        Tool::Rect | Tool::Marker | Tool::Mosaic => {
            let (Some(a), Some(b)) = (m.pts.first(), m.pts.last()) else { return };
            let (a, b) = (tr(*a), tr(*b));
            let r = norm(a.0, a.1, b.0, b.1);
            match m.tool {
                Tool::Rect => cv.outline(r, m.color, size),
                Tool::Marker => cv.fill(r, m.color.alpha(0.38)),
                // толщина 2 — лёгкое размытие (не прочесть, но мягко)
                _ if m.size <= 2 => blur(cv, m, r, base, o, k, sb, sc(6).max(2)),
                _ => {
                    // мозаика: средний цвет клетки исходника (база — в своём масштабе: экран × sb);
                    // толщина 4 — клетки как были, 8 — крупные
                    let cell = sc(if m.size >= 8 { 26 } else { 10 }).max(1);
                    let mut y = r.y;
                    while y < r.bottom() {
                        let mut x = r.x;
                        while x < r.right() {
                            let cw = cell.min(r.right() - x);
                            let ch = cell.min(r.bottom() - y);
                            // точка исходника: буфер → экран (÷k + o) → база (×sb)
                            let bx = (((x as f32 / k) + o.0 as f32) * sb).round().clamp(0.0, (base.w - 1) as f32) as i32;
                            let by = (((y as f32 / k) + o.1 as f32) * sb).round().clamp(0.0, (base.h - 1) as f32) as i32;
                            let (bw, bh) = ((cw as f32 / k * sb).round().max(1.0) as i32, (ch as f32 / k * sb).round().max(1.0) as i32);
                            let (mut rs, mut gs, mut bs, mut n) = (0u32, 0u32, 0u32, 0u32);
                            let mut yy = by;
                            while yy < (by + bh).min(base.h) {
                                let mut xx = bx;
                                while xx < (bx + bw).min(base.w) {
                                    let p = base.px[(yy * base.w + xx) as usize];
                                    rs += (p >> 16) & 255;
                                    gs += (p >> 8) & 255;
                                    bs += p & 255;
                                    n += 1;
                                    xx += 2;
                                }
                                yy += 2;
                            }
                            let n = n.max(1);
                            cv.set(R::new(x, y, cw, ch), C(0xff000000 | ((rs / n) << 16) | ((gs / n) << 8) | (bs / n)));
                            x += cell;
                        }
                        y += cell;
                    }
                }
            }
        }
        Tool::Arrow => {
            let (Some(a), Some(b)) = (m.pts.first(), m.pts.last()) else { return };
            let (a, b) = (tr(*a), tr(*b));
            line(cv, a, b, size, m.color);
            let ang = ((b.1 - a.1) as f32).atan2((b.0 - a.0) as f32);
            let len = (size as f32 * 4.0 + 10.0 * k).min((((b.0 - a.0).pow(2) + (b.1 - a.1).pow(2)) as f32).sqrt());
            for d in [2.6f32, -2.6] {
                let e = (b.0 + (len * (ang + d).cos()) as i32, b.1 + (len * (ang + d).sin()) as i32);
                line(cv, b, e, size, m.color);
            }
        }
        Tool::Count => {
            let Some(p) = m.pts.first() else { return };
            let p = tr(*p);
            let s = sc(count_side(m.size));
            // протянули: стрелка от края номерка к точке
            if let Some(q) = m.pts.get(1) {
                let q = tr(*q);
                let (dx, dy) = ((q.0 - p.0) as f32, (q.1 - p.1) as f32);
                let len = (dx * dx + dy * dy).sqrt().max(1.0);
                let a = (p.0 + (dx / len * s as f32 / 2.0) as i32, p.1 + (dy / len * s as f32 / 2.0) as i32);
                let lw = sc(match m.size { 2 => 2, 8 => 5, _ => 3 }).max(1);
                line(cv, a, q, lw, m.color);
                let ang = dy.atan2(dx);
                let head = (lw as f32 * 3.0 + 6.0 * k).min(len - s as f32 / 2.0).max(0.0);
                for d in [2.6f32, -2.6] {
                    let e = (q.0 + (head * (ang + d).cos()) as i32, q.1 + (head * (ang + d).sin()) as i32);
                    line(cv, q, e, lw, m.color);
                }
            }
            let r = R::new(p.0 - s / 2, p.1 - s / 2, s, s);
            cv.pxbox(r, &BoxStyle::new(m.color, C::hex(0x0b0909)).shadow(sc(2)));
            let fs = match m.size { 2 => 12.0, 8 => 26.0, _ => 16.0 };
            let face = crate::font::px((fs * k).round() as u16);
            let txt = m.n.to_string();
            let tc = if crate::theme::lightness(m.color) > 0.6 { C::hex(0x0b0909) } else { on_accent };
            cv.text_center(fonts, face, r, &txt, tc);
        }
        Tool::Select | Tool::Pick => {}
    }
}

/// сторона номерка (точки экрана) по толщине: 2 — мелкий, 4 — обычный, 8 — крупный
pub(crate) fn count_side(size: i32) -> i32 {
    match size {
        2 => 18,
        8 => 42,
        _ => 26,
    }
}

/// размытие области r буфера: пиксели исходника (буфер → экран ÷k + o → база ×sb), crate::blur.
/// Результат хранится в метке (m.cache) и пересчитывается, только когда меняются область, сдвиг
/// или масштаб: оверлей перерисовывается каждый кадр (бегущие муравьи), а размытие дорогое
#[allow(clippy::too_many_arguments)]
fn blur(cv: &mut Canvas, m: &Mark, r: R, base: &Img, o: (i32, i32), k: f32, sb: f32, rad: i32) {
    // область в пределах буфера (без обрезки: она меняется, а кеш — нет)
    let full = R::new(r.x + cv.ox, r.y + cv.oy, r.w, r.h).inter(&R::new(0, 0, cv.w, cv.h));
    if full.empty() {
        return;
    }
    let key = (full, cv.ox, cv.oy, o, k.to_bits(), sb.to_bits(), rad);
    let mut cache = m.cache.borrow_mut();
    if cache.as_ref().map(|c| c.0) != Some(key) {
        let (w, h) = (full.w as usize, full.h as usize);
        let mut ch = [vec![0f32; w * h], vec![0f32; w * h], vec![0f32; w * h]];
        for y in 0..h {
            for x in 0..w {
                let bx = ((((full.x - cv.ox) as f32 + x as f32 + 0.5) / k + o.0 as f32) * sb) as i32;
                let by = ((((full.y - cv.oy) as f32 + y as f32 + 0.5) / k + o.1 as f32) * sb) as i32;
                let p = base.px[(by.clamp(0, base.h - 1) * base.w + bx.clamp(0, base.w - 1)) as usize];
                ch[0][y * w + x] = ((p >> 16) & 255) as f32;
                ch[1][y * w + x] = ((p >> 8) & 255) as f32;
                ch[2][y * w + x] = (p & 255) as f32;
            }
        }
        for c in ch.iter_mut() {
            crate::blur::box3(c, w, h, rad as usize);
        }
        let px = (0..w * h).map(|i| 0xff000000 | (ch[0][i] as u32) << 16 | (ch[1][i] as u32) << 8 | ch[2][i] as u32).collect();
        *cache = Some((key, px));
    }
    let Some((_, px)) = cache.as_ref() else { return };
    let vis = full.inter(&cv.clip_rect());
    let bw = cv.w as usize;
    for y in vis.y..vis.bottom() {
        let src = (y - full.y) as usize * full.w as usize + (vis.x - full.x) as usize;
        let dst = y as usize * bw + vis.x as usize;
        cv.buf[dst..dst + vis.w as usize].copy_from_slice(&px[src..src + vis.w as usize]);
    }
}

// ── отрисовка оверлея ──────────────────────────────────────────────────────
pub fn draw(ui: &mut Ui, st: &State, out: &str, w: i32, h: i32) {
    let t = ui.t;
    let Some(sn) = &st.snap else { return };
    let Some(img) = sn.imgs.get(out) else { return };
    // снимок сделан (вспышки нет), курсор может быть в углу (snap_tick): оверлей прозрачен
    if sn.flash.is_some() {
        return;
    }
    if img.w == w && img.h == h {
        ui.cv.blit(img, 0, 0);
    } else {
        ui.cv.image_scaled(img, R::new(0, 0, w, h), 1.0);
    }
    let sel = sn.cur_sel(out);
    let veil = t.bg.alpha(0.52);
    match sel {
        Some(r) => {
            ui.cv.fill(R::new(0, 0, w, r.y), veil);
            ui.cv.fill(R::new(0, r.bottom(), w, h - r.bottom()), veil);
            ui.cv.fill(R::new(0, r.y, r.x, r.h), veil);
            ui.cv.fill(R::new(r.right(), r.y, w - r.right(), r.h), veil);
        }
        None => ui.cv.fill(R::new(0, 0, w, h), veil),
    }
    // строки развёртки по вуали
    for y in (0..h).step_by(4) {
        match sel {
            Some(r) if y >= r.y && y < r.bottom() => {
                ui.cv.fill(R::new(0, y, r.x, 1), C(0xff000000).alpha(0.16));
                ui.cv.fill(R::new(r.right(), y, w - r.right(), 1), C(0xff000000).alpha(0.16));
            }
            _ => ui.cv.fill(R::new(0, y, w, 1), C(0xff000000).alpha(0.16)),
        }
    }
    let ph = (st.started.elapsed().as_millis() / 70) as i32;
    // режим «окно»: окно под курсором
    if sn.mode == Mode::Window && sn.drag.is_none() {
        if let Some((_, px, py)) = sn.ptr.as_ref().filter(|p| p.0 == out) {
            if let Some((_, r, title)) = sn.wins.iter().find(|(o, r, _)| o == out && r.contains(*px as f64, *py as f64)) {
                if sel.map(|s| s != *r).unwrap_or(true) {
                    ui.cv.fill(*r, t.red.alpha(0.12));
                    ui.cv.ants(*r, t.ember, 4, ph);
                    let tw = ui.f.width(PX, title).min(r.w - 20) + 20;
                    let tr = R::new(r.x + 8, r.y + 8, tw, 24);
                    ui.cv.pxbox(tr, &BoxStyle::new(t.panel.alpha(0.95), t.red).shadow(2));
                    ui.cv.text_in(ui.f, PX, R::new(tr.x + 10, tr.y, tr.w - 14, tr.h), title, t.text);
                }
            }
        }
    }
    // метки
    if let Some(r) = sel {
        // метки — и за рамкой: рисовать можно где угодно, в файл попадёт то, что внутри
        for m in &sn.marks {
            paint(&mut ui.cv, ui.f, m, img, (0, 0), 1.0, img.w as f32 / w.max(1) as f32, t.on_accent);
        }
        ui.cv.ants(R::new(r.x - 1, r.y - 1, r.w + 2, r.h + 2), t.ember, 4, ph);
        ui.anim = true;
        if sn.tool == Tool::Select || sn.drag.is_some() {
            for hr in handles(r) {
                ui.cv.fill(hr, C::hex(0x0b0909));
                ui.cv.fill(hr.inset(1), t.ember);
            }
        }
        // размер
        let s = format!("{} × {}", r.w, r.h);
        let sw = ui.f.width(PX, &s) + 16;
        let ly = if r.y > 30 { r.y - 28 } else { r.y + 6 };
        let lr = R::new(r.x, ly, sw, 22);
        ui.cv.pxbox(lr, &BoxStyle::new(t.panel.alpha(0.95), t.border));
        ui.cv.text_center(ui.f, PX, lr, &s, t.text);
        toolbar(ui, sn, &st.set.recent, r, w, h);
    } else {
        top_bar(ui, sn, w);
    }
    // перекрестье и лупа
    if let Some((_, px, py)) = sn.ptr.as_ref().filter(|p| p.0 == out) {
        let (px, py) = (*px, *py);
        let on_bar = ui.zones.iter().any(|z| matches!(z.act, Act::SnapBtn(_)) && z.r.contains(px as f64, py as f64));
        if sel.is_none() && sn.mode == Mode::Area {
            for x in (0..w).step_by(6) {
                ui.cv.fill(R::new(x, py, 3, 1), t.ember.alpha(0.55));
            }
            for y in (0..h).step_by(6) {
                ui.cv.fill(R::new(px, y, 1, 3), t.ember.alpha(0.55));
            }
        }
        if st.set.shot_loupe && !on_bar && sn.drag.map(|d| !matches!(d, Drag::Move(..) | Drag::Draw)).unwrap_or(true) {
            loupe(ui, img, px, py, w, h);
        }
    }
    if let Some((s, t0)) = &sn.hint {
        if t0.elapsed().as_millis() < 1600 {
            let sw = ui.w(PX, s) + 32;
            let r = R::new((w - sw) / 2, h - 80, sw, 30);
            ui.cv.pxbox(r, &BoxStyle::new(t.panel.alpha(0.96), t.red).shadow(3));
            ui.text_center(PX, r, s, t.text);
            ui.anim = true;
        }
    }
    ui.zone(R::new(0, 0, w, h), Act::Block, Cur::Default);
    // зоны кнопок должны быть поверх «фона»: переставим фон в начало
    if let Some(bg) = ui.zones.pop() {
        ui.zones.insert(0, bg);
    }
}

pub(crate) fn loupe(ui: &mut Ui, img: &Img, px: i32, py: i32, w: i32, h: i32) {
    let t = ui.t;
    let n = 15; // клеток по стороне
    let z = 8; // размер клетки
    let side = n * z;
    let mut x = px + 28;
    let mut y = py + 28;
    if x + side + 8 > w {
        x = px - 28 - side;
    }
    if y + side + 56 > h {
        y = py - 28 - side - 46;
    }
    let fr = R::new(x - 4, y - 4, side + 8, side + 52);
    ui.cv.pxbox(fr, &BoxStyle::new(t.panel.alpha(0.98), t.red).shadow(4));
    // масштаб снимка к экрану (HiDPI)
    let k = img.w as f32 / w.max(1) as f32;
    let (cx, cy) = ((px as f32 * k) as i32, (py as f32 * k) as i32);
    for j in 0..n {
        for i in 0..n {
            let sx = cx + i - n / 2;
            let sy = cy + j - n / 2;
            let c = if sx >= 0 && sy >= 0 && sx < img.w && sy < img.h { C(img.px[(sy * img.w + sx) as usize] | 0xff000000) } else { C::hex(0x000000) };
            ui.cv.set(R::new(x + i * z, y + j * z, z, z), c);
        }
    }
    // сетка и центральная клетка
    for i in 1..n {
        ui.cv.fill(R::new(x + i * z, y, 1, side), C(0xff000000).alpha(0.22));
        ui.cv.fill(R::new(x, y + i * z, side, 1), C(0xff000000).alpha(0.22));
    }
    let c0 = R::new(x + (n / 2) * z, y + (n / 2) * z, z, z);
    ui.cv.outline(R::new(c0.x - 1, c0.y - 1, z + 2, z + 2), C::hex(0x0b0909), 1);
    ui.cv.outline(c0, C(0xffffffff), 1);
    let col = if cx >= 0 && cy >= 0 && cx < img.w && cy < img.h { img.px[(cy * img.w + cx) as usize] & 0xffffff } else { 0 };
    ui.cv.fill(R::new(x, y + side + 8, 14, 14), C::hex(col));
    ui.cv.outline(R::new(x, y + side + 8, 14, 14), t.border, 1);
    ui.cv.text_in(ui.f, PX, R::new(x + 20, y + side + 4, side - 20, 22), &format!("#{col:06x}"), t.text);
    ui.cv.text_in(ui.f, PX, R::new(x, y + side + 24, side, 20), &format!("{px}, {py}"), t.text_faint);
}

pub(crate) fn top_bar(ui: &mut Ui, sn: &Snap, w: i32) {
    let t = ui.t;
    let modes = [(Mode::Area, "область", "crop_", "m:area"), (Mode::Window, "окно", "window", "m:window"), (Mode::Screen, "экран", "screen", "m:screen")];
    let bw = 112;
    let tw = bw * 3 + 8 * 2 + 16;
    let r = R::new((w - tw) / 2, 18, tw, 44);
    ui.cv.pxbox(r, &BoxStyle::new(t.panel.alpha(0.96), t.border.lerp(t.red, 0.5)).shadow(4));
    ui.zone(r, Act::SnapBtn("none"), Cur::Default);
    for (i, (m, label, icon, act)) in modes.iter().enumerate() {
        let br = R::new(r.x + 8 + i as i32 * (bw + 8), r.y + 7, bw, 30);
        let icon = if *icon == "crop_" { "rect" } else { icon };
        ui.button(br, label, icon, Tone::Ghost, sn.mode == *m, true, Act::SnapBtn(act));
    }
    let hint = match sn.mode {
        Mode::Area => "тяни — выделить · enter — весь экран · esc — отмена",
        Mode::Window => "щёлкни по окну · enter — окно под курсором · esc — отмена",
        Mode::Screen => "щёлкни или enter — весь монитор · esc — отмена",
    };
    let hw = ui.w(PX, hint) + 24;
    let hr = R::new((w - hw) / 2, r.bottom() + 8, hw, 24);
    ui.cv.pxbox(hr, &BoxStyle::new(t.bg.alpha(0.85), C(0)));
    ui.text_center(PX, hr, hint, t.text_dim);
}

pub(crate) fn toolbar(ui: &mut Ui, sn: &Snap, recent: &[C], sel: R, w: i32, h: i32) {
    let t = ui.t;
    let bs = 30;
    let colors = Snap::colors(&t);
    let n_tools = TOOLS.len() as i32;
    let extra = sn.ocr as i32 + sn.qr as i32;
    let tw = 8 + n_tools * (bs + 2) + 10 + (colors.len() as i32 + 1) * 22 + 10 + 3 * (bs + 2) + 10 + (5 + extra) * (bs + 2) + 4;
    let th = 44;
    let mut x = sel.x + (sel.w - tw) / 2;
    x = x.clamp(6, (w - tw - 6).max(6));
    let mut y = sel.bottom() + 10;
    if y + th > h - 6 {
        y = sel.y - th - 34;
        if y < 6 {
            y = sel.bottom() - th - 10;
        }
    }
    let r = R::new(x, y, tw, th);
    ui.cv.pxbox(r, &BoxStyle::new(t.panel.alpha(0.97), t.border.lerp(t.red, 0.5)).shadow(4));
    ui.zone(r, Act::SnapBtn("none"), Cur::Default);
    let by = r.y + 7;
    let mut bx = r.x + 8;
    for (tool, icon, act, _) in TOOLS {
        ui.button(R::new(bx, by, bs, bs), "", icon, Tone::Ghost, sn.tool == *tool, true, Act::SnapBtn(act));
        bx += bs + 2;
    }
    bx += 4;
    ui.cv.fill(R::new(bx, by + 4, 1, bs - 8), t.border);
    bx += 6;
    for (i, c) in colors.iter().enumerate() {
        let cr = R::new(bx + 2, by + 6, 18, 18);
        let hov = ui.hover(cr);
        if sn.color == i {
            ui.cv.outline(R::new(cr.x - 3, cr.y - 3, cr.w + 6, cr.h + 6), t.ember, 2);
        } else if hov {
            ui.cv.outline(R::new(cr.x - 2, cr.y - 2, cr.w + 4, cr.h + 4), t.border, 1);
        }
        ui.cv.fill(cr, *c);
        ui.cv.outline(cr, C::hex(0x0b0909), 1);
        ui.zone(R::new(cr.x - 2, cr.y - 4, 22, 26), Act::SnapBtn(COLOR_ACTS[i]), Cur::Pointer);
        bx += 22;
    }
    // другие цвета: «+», а когда выбран цвет из меню — он сам
    let mr = R::new(bx + 2, by + 6, 18, 18);
    let custom = sn.color == usize::MAX;
    if custom || sn.menu {
        ui.cv.outline(R::new(mr.x - 3, mr.y - 3, mr.w + 6, mr.h + 6), t.ember, 2);
    } else if ui.hover(mr) {
        ui.cv.outline(R::new(mr.x - 2, mr.y - 2, mr.w + 4, mr.h + 4), t.border, 1);
    }
    if custom {
        ui.cv.fill(mr, sn.cur);
        ui.cv.outline(mr, C::hex(0x0b0909), 1);
    } else {
        ui.cv.icon("plus", mr.x + 2, mr.y + 2, 2, t.text_dim);
    }
    ui.zone(R::new(mr.x - 2, mr.y - 4, 22, 26), Act::SnapBtn("more"), Cur::Pointer);
    let more_x = mr.x;
    bx += 22;
    bx += 4;
    ui.cv.fill(R::new(bx, by + 4, 1, bs - 8), t.border);
    bx += 6;
    // толщина: три точки разного размера
    for (k, sz) in [(0, 2), (1, 4), (2, 8)] {
        let br = R::new(bx, by, bs, bs);
        let on = sn.size == [2, 4, 8][k];
        ui.button(br, "", "", Tone::Ghost, on, true, Act::SnapBtn(["s:2", "s:4", "s:8"][k]));
        let d = sz.max(2);
        ui.cv.fill(R::new(br.x + (bs - d) / 2, br.y + (bs - d) / 2, d, d), if on { C(0xffffffff) } else { t.text_dim });
        bx += bs + 2;
    }
    bx += 4;
    ui.cv.fill(R::new(bx, by + 4, 1, bs - 8), t.border);
    bx += 6;
    let mut acts: Vec<(&str, &str, bool)> = vec![("left", "undo", !sn.marks.is_empty())];
    // распознать: текст (tesseract) и QR (zbarimg) — только если они есть
    if sn.ocr {
        acts.push(("text", "ocr", true));
    }
    if sn.qr {
        acts.push(("qr", "qr", true));
    }
    acts.extend([("copy", "copy", true), ("down", "save", true), ("folder", "saveas", true), ("close", "close", true)]);
    for (icon, act, on) in acts {
        let tone = if act == "save" { Tone::Primary } else { Tone::Ghost };
        ui.button(R::new(bx, by, bs, bs), "", icon, tone, false, on && !sn.busy, Act::SnapBtn(act));
        bx += bs + 2;
    }
    if sn.menu {
        more_menu(ui, sn, recent, r, more_x, w);
    }
    // подсказка к кнопке под курсором
    if let Some((_, px, py)) = &sn.ptr {
        let tip = ui.zones.iter().rev().find(|z| z.r.contains(*px as f64, *py as f64)).and_then(|z| match &z.act {
            Act::SnapBtn(a) => tip_of(a),
            _ => None,
        });
        if let Some(tip) = tip {
            let tw2 = ui.w(PX, tip) + 18;
            let tr = R::new((*px - tw2 / 2).clamp(4, w - tw2 - 4), if r.y > 40 { r.y - 28 } else { r.bottom() + 6 }, tw2, 22);
            ui.cv.pxbox(tr, &BoxStyle::new(t.panel, t.red));
            ui.text_center(PX, tr, tip, t.text);
        }
    }
}

/// меню «другие цвета»: набор 8×2 и полоса недавних; над панелью, если сверху есть место
fn more_menu(ui: &mut Ui, sn: &Snap, recent: &[C], bar: R, x: i32, w: i32) {
    let t = ui.t;
    let (cell, gap, pad) = (22, 4, 10);
    let mw = pad * 2 + 8 * cell + 7 * gap;
    let mh = pad * 2 + 2 * cell + gap + 26 + cell;
    let mx = (x - mw / 2).clamp(6, (w - mw - 6).max(6));
    let my = if bar.y - mh - 8 > 6 { bar.y - mh - 8 } else { bar.bottom() + 8 };
    let r = R::new(mx, my, mw, mh);
    ui.cv.pxbox(r, &BoxStyle::new(t.panel.alpha(0.98), t.border.lerp(t.red, 0.5)).shadow(4));
    ui.zone(r, Act::SnapBtn("none"), Cur::Default);
    let swatch = |ui: &mut Ui, cr: R, c: C, act: &'static str| {
        let on = sn.cur.0 & 0xffffff == c.0 & 0xffffff;
        if on {
            ui.cv.outline(R::new(cr.x - 3, cr.y - 3, cr.w + 6, cr.h + 6), t.ember, 2);
        } else if ui.hover(cr) {
            ui.cv.outline(R::new(cr.x - 2, cr.y - 2, cr.w + 4, cr.h + 4), t.border, 1);
        }
        ui.cv.fill(cr, c);
        ui.cv.outline(cr, C::hex(0x0b0909), 1);
        ui.zone(cr, Act::SnapBtn(act), Cur::Pointer);
    };
    for (i, c) in MORE.iter().enumerate() {
        let (col, row) = (i as i32 % 8, i as i32 / 8);
        let cr = R::new(r.x + pad + col * (cell + gap) + 2, r.y + pad + row * (cell + gap) + 2, cell - 4, cell - 4);
        swatch(ui, cr, C::hex(*c), MORE_ACTS[i]);
    }
    let ly = r.y + pad + 2 * cell + gap + 2;
    ui.text_in(PX, R::new(r.x + pad, ly, mw - pad * 2, 22), "недавние", t.text_dim);
    let ry = ly + 26;
    if recent.is_empty() {
        ui.text_in(PX, R::new(r.x + pad, ry, mw - pad * 2, cell), "пока пусто", t.text_faint);
    }
    for (i, c) in recent.iter().take(RECENT_ACTS.len()).enumerate() {
        let cr = R::new(r.x + pad + i as i32 * (cell + gap) + 2, ry + 2, cell - 4, cell - 4);
        swatch(ui, cr, *c, RECENT_ACTS[i]);
    }
}

pub(crate) fn tip_of(a: &str) -> Option<&'static str> {
    Some(match a {
        "t:select" => "выделение · 1",
        "t:pen" => "перо · 2",
        "t:rect" => "рамка · 3",
        "t:arrow" => "стрелка · 4",
        "t:marker" => "маркер · 5",
        "t:mosaic" => "мозаика · 6",
        "t:count" => "номерок · 7",
        "t:pick" => "пипетка: цвет в буфер · 8",
        "ocr" => "текст в буфер (tesseract)",
        "qr" => "QR-код в буфер",
        "undo" => "отменить · ctrl+z",
        "copy" => "в буфер · ctrl+c",
        "save" => "сохранить · enter",
        "saveas" => "сохранить как… · ctrl+s",
        "close" => "отмена · esc",
        "s:2" | "s:4" | "s:8" => "толщина · колесо",
        "more" => "другие цвета",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::State;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::time::Instant;

    #[test]
    fn shift_pen_snaps_to_45_degrees() {
        assert_eq!(snap45((0, 0), (100, 7)), (100, 0));
        assert_eq!(snap45((0, 0), (-3, 80)), (0, 80));
        let (x, y) = snap45((0, 0), (90, 110));
        assert_eq!(x, y);
        assert_eq!(count_side(2) < count_side(4), count_side(4) < count_side(8));
    }

    /// `YSNAP_PREVIEW=файл.png cargo test preview -- --ignored`: оверлей с метками всех
    /// видов и открытым меню цветов, без Wayland и grim
    #[test]
    #[ignore]
    fn preview() {
        let out = std::env::var("YSNAP_PREVIEW").unwrap_or_else(|_| "preview.png".into());
        let (w, h) = (1280, 720);
        let t = crate::gtk::theme().unwrap_or_else(|| ysnap_px::look::Look::from_json(&serde_json::Value::Null, None).theme);
        let mut px = vec![0u32; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                // «текст»: полосы и мелкие штрихи, чтобы было видно размытие и мозаику
                let txt = (y / 14) % 2 == 0 && (x / 3 + y) % 5 < 3;
                px[(y * w + x) as usize] = if txt { 0xffe8e8e8 } else if (x / 160 + y / 120) % 2 == 0 { 0xff2a2f3a } else { 0xff1c2028 };
            }
        }
        let mut imgs = HashMap::new();
        imgs.insert(String::new(), Rc::new(Img { w, h, px }));
        let c = Snap::colors(&t);
        let mk = |tool, pts: Vec<(i32, i32)>, color, size, n| Mark { tool, pts, color, size, n, cache: Default::default() };
        let marks = vec![
            mk(Tool::Mosaic, vec![(220, 200), (380, 300)], c[0], 2, 0),
            mk(Tool::Mosaic, vec![(400, 200), (560, 300)], c[0], 4, 0),
            mk(Tool::Mosaic, vec![(580, 200), (740, 300)], c[0], 8, 0),
            mk(Tool::Pen, vec![(800, 380), snap45((800, 380), (900, 384))], c[0], 4, 0),
            mk(Tool::Pen, vec![(800, 380), snap45((800, 380), (870, 300))], c[0], 4, 0),
            mk(Tool::Pen, vec![(800, 380), snap45((800, 380), (803, 280))], c[0], 4, 0),
            mk(Tool::Count, vec![(260, 360)], c[0], 2, 1),
            mk(Tool::Count, vec![(330, 360), (420, 430)], c[1], 4, 2),
            mk(Tool::Count, vec![(500, 380), (640, 330)], c[6], 8, 3),
            // за рамкой: видно на экране, в файл не попадает
            mk(Tool::Arrow, vec![(1060, 120), (930, 220)], c[0], 4, 0),
            mk(Tool::Mosaic, vec![(40, 500), (180, 620)], c[0], 8, 0),
        ];
        let mut set = crate::config::Settings::from_json(&serde_json::Value::Null);
        set.recent = vec![C::hex(0x2f80ed), C::hex(0xf5c518), C::hex(0x00ff88)];
        let sn = Snap {
            imgs,
            mode: Mode::Area,
            sel: Some((String::new(), R::new(200, 170, 760, 300))),
            drag: None,
            tool: Tool::Count,
            color: usize::MAX,
            cur: C::hex(0x2f80ed),
            menu: true,
            size: 4,
            marks,
            redo: vec![],
            ptr: Some((String::new(), 1100, 640)),
            flash: None,
            wins: vec![],
            busy: false,
            hint: None,
            ocr: true,
            qr: false,
            focus_win: None,
        };
        let st = State { snap: Some(sn), started: Instant::now(), set, t };
        let mut fonts = ysnap_px::font::Fonts::with(&crate::gtk::font());
        let mut icons = ysnap_px::icons::Icons::new("");
        let mut buf = vec![0u32; (w * h) as usize];
        {
            let mut ui = Ui { cv: Canvas::new(&mut buf, w, h), f: &mut fonts, ic: &mut icons, t, zones: vec![], ptr: Some((1100.0, 640.0)), down: false, anim: false, blink: false, tr: crate::i18n::tr };
            draw(&mut ui, &st, "", w, h);
        }
        let rgba: Vec<u8> = buf.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255]).collect();
        image::save_buffer(&out, &rgba, w as u32, h as u32, image::ExtendedColorType::Rgba8).unwrap();
    }
}
