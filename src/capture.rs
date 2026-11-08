// Снимок мониторов: wlr-screencopy прямо в нашем соединении с Wayland (без процесса grim и PPM
// туда-обратно), параллельно — запросы к niri в нити (niri::info). Оверлей открывается, когда
// готово и то и другое. Нет screencopy, монитор повёрнут (снимок пришлось бы поворачивать) или
// копия не вышла — те же мониторы через grim, как раньше.

use crate::app::{App, Msg};
use crate::gfx::Img;
use crate::snap::Mode;
use smithay_client_toolkit::shm::raw::RawPool;
use std::collections::HashMap;
use std::time::Duration;
use wayland_client::protocol::{wl_buffer, wl_output, wl_shm};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1 as frame, zwlr_screencopy_manager_v1 as manager};

/// копия одного монитора
pub struct Frame {
    obj: frame::ZwlrScreencopyFrameV1,
    /// формат, ширина, высота, шаг строки — из события buffer
    fmt: Option<(wl_shm::Format, i32, i32, i32)>,
    pool: Option<RawPool>,
    buf: Option<wl_buffer::WlBuffer>,
    invert: bool,
    /// Some — кончилась: снимок или None (не вышла)
    done: Option<Option<Img>>,
}

/// снимок в процессе: копии мониторов (или grim в нити) и ответ niri
pub struct Pending {
    mode: Mode,
    outs: Vec<String>,
    frames: HashMap<String, Frame>,
    imgs: Option<HashMap<String, Img>>,
    niri: Option<crate::niri::Info>,
}

/// форматы, которые читаем: ARGB/XRGB (как есть) и ABGR/XBGR (красный и синий местами)
fn readable(f: wl_shm::Format) -> bool {
    matches!(f, wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 | wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888)
}

/// пиксели буфера → Img (0xffRRGGBB), с переворотом по флагу y_invert
fn to_img(data: &[u8], fmt: wl_shm::Format, w: i32, h: i32, stride: i32, invert: bool) -> Option<Img> {
    let swap = matches!(fmt, wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888);
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let sy = if invert { h - 1 - y } else { y };
        let row = data.get((sy * stride) as usize..(sy * stride + w * 4) as usize)?;
        px.extend(row.chunks_exact(4).map(|b| {
            let p = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            if swap {
                0xff000000 | (p & 0xff) << 16 | (p & 0xff00) | (p >> 16) & 0xff
            } else {
                0xff000000 | p
            }
        }));
    }
    Some(Img { w, h, px })
}

impl App {
    /// начать: таймер (если есть) → снимок мониторов и вопросы к niri → оверлей (on_snap_ready)
    pub fn snap_start(&mut self, mode: Mode, delay: u32) {
        if self.st.snap.is_some() || self.waiting {
            return;
        }
        self.waiting = true;
        if delay == 0 {
            return self.begin_capture(mode);
        }
        let _ = self.handle.insert_source(calloop::timer::Timer::from_duration(Duration::from_secs(delay as u64)), move |_, _, app: &mut App| {
            app.begin_capture(mode);
            calloop::timer::TimeoutAction::Drop
        });
    }

    fn begin_capture(&mut self, mode: Mode) {
        let mut outs = self.output_names();
        let tx = self.tx.clone();
        // экран без оверлея: нужен только монитор в фокусе — ни остальных мониторов, ни окон
        let only_focus = mode == Mode::Screen && (self.instant || self.st.set.shot_confirm == "instant");
        let mut niri = None;
        if only_focus {
            let focus = crate::niri::focus_name(&crate::niri::request("FocusedOutput"));
            if outs.contains(&focus) {
                outs = vec![focus.clone()];
                niri = Some(crate::niri::Info { wins: vec![], focus, focus_win: None });
            }
        }
        if niri.is_none() {
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Niri(crate::niri::info()));
            });
        }
        self.pending = Some(Pending { mode, outs: outs.clone(), frames: HashMap::new(), imgs: None, niri });
        // повёрнутый монитор screencopy отдаёт как есть (без поворота) — такие через grim
        let rotated = self.outputs.outputs().filter_map(|o| self.outputs.info(&o)).any(|i| i.name.as_ref().is_some_and(|n| outs.contains(n)) && i.transform != wl_output::Transform::Normal);
        match (&self.screencopy, rotated) {
            (Some(m), false) => {
                let m = m.clone();
                for name in &outs {
                    let Some(o) = self.outputs.outputs().find(|o| self.outputs.info(o).and_then(|i| i.name).as_deref() == Some(name)) else { continue };
                    let obj = m.capture_output(0, &o, &self.qh, name.clone());
                    if let Some(p) = &mut self.pending {
                        p.frames.insert(name.clone(), Frame { obj, fmt: None, pool: None, buf: None, invert: false, done: None });
                    }
                }
                // ни один монитор не нашёлся по имени — иначе ждали бы копий вечно
                if self.pending.as_ref().is_some_and(|p| p.frames.is_empty()) {
                    self.grim(outs);
                }
            }
            _ => self.grim(outs),
        }
    }

    /// мониторы через grim в нити (запасной путь)
    fn grim(&mut self, outs: Vec<String>) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let imgs = std::thread::scope(|sc| {
                let hs: Vec<_> = outs.iter().map(|o| sc.spawn(move || crate::snap::grab(o).map(|i| (o.clone(), i)))).collect();
                hs.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
            });
            let _ = tx.send(Msg::Grabbed(imgs));
        });
    }

    pub fn on_grabbed(&mut self, imgs: HashMap<String, Img>) {
        if let Some(p) = &mut self.pending {
            p.imgs = Some(imgs);
        }
        self.capture_check();
    }

    pub fn on_niri(&mut self, info: crate::niri::Info) {
        if let Some(p) = &mut self.pending {
            p.niri = Some(info);
        }
        self.capture_check();
    }

    /// копии кончились — собрать (одна не вышла — все через grim); всё готово — оверлей
    fn capture_check(&mut self) {
        let Some(p) = &mut self.pending else { return };
        if p.imgs.is_none() && !p.frames.is_empty() && p.frames.values().all(|f| f.done.is_some()) {
            let frames = std::mem::take(&mut p.frames);
            let imgs: HashMap<String, Img> = frames.into_iter().filter_map(|(n, f)| f.done.flatten().map(|i| (n, i))).collect();
            if imgs.len() == p.outs.len() {
                p.imgs = Some(imgs);
            } else {
                let outs = p.outs.clone();
                return self.grim(outs);
            }
        }
        let ready = p.imgs.is_some() && p.niri.is_some();
        if !ready {
            return;
        }
        let Some(p) = self.pending.take() else { return };
        let info = p.niri.unwrap_or(crate::niri::Info { wins: vec![], focus: String::new(), focus_win: None });
        self.timing("captured");
        self.on_snap_ready(p.imgs.unwrap_or_default(), info.wins, p.mode, info.focus, info.focus_win);
    }

    fn frame_event(&mut self, name: &str, e: frame::Event) {
        let qh = self.qh.clone();
        let Some(f) = self.pending.as_mut().and_then(|p| p.frames.get_mut(name)) else { return };
        match e {
            frame::Event::Buffer { format: WEnum::Value(fmt), width, height, stride } if f.fmt.is_none() && readable(fmt) => {
                f.fmt = Some((fmt, width as i32, height as i32, stride as i32));
                // до 3-й версии buffer_done нет: копируем сразу
                if f.obj.version() < 3 {
                    copy(f, &self.shm, &qh);
                }
            }
            frame::Event::BufferDone => {
                if f.fmt.is_some() {
                    copy(f, &self.shm, &qh);
                } else {
                    f.done = Some(None);
                }
            }
            frame::Event::Flags { flags } => {
                f.invert = matches!(flags, WEnum::Value(v) if v.contains(frame::Flags::YInvert));
            }
            frame::Event::Ready { .. } => {
                let img = match (f.fmt, f.pool.as_mut()) {
                    (Some((fmt, w, h, stride)), Some(pool)) => to_img(pool.mmap(), fmt, w, h, stride, f.invert),
                    _ => None,
                };
                f.done = Some(img);
                finish(f);
            }
            frame::Event::Failed => {
                f.done = Some(None);
                finish(f);
            }
            _ => {}
        }
        if self.pending.as_ref().is_some_and(|p| p.frames.values().all(|f| f.done.is_some())) {
            self.capture_check();
        }
    }
}

/// буфер под копию и запрос copy
fn copy(f: &mut Frame, shm: &smithay_client_toolkit::shm::Shm, qh: &QueueHandle<App>) {
    let Some((fmt, w, h, stride)) = f.fmt else { return };
    if f.buf.is_some() {
        return;
    }
    let Ok(mut pool) = RawPool::new((stride * h) as usize, shm) else {
        f.done = Some(None);
        return;
    };
    let buf = pool.create_buffer(0, w, h, stride, fmt, (), qh);
    f.obj.copy(&buf);
    f.buf = Some(buf);
    f.pool = Some(pool);
}

fn finish(f: &mut Frame) {
    if let Some(b) = f.buf.take() {
        b.destroy();
    }
    f.pool = None;
    f.obj.destroy();
}

impl Dispatch<manager::ZwlrScreencopyManagerV1, ()> for App {
    fn event(_: &mut Self, _: &manager::ZwlrScreencopyManagerV1, _: manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<frame::ZwlrScreencopyFrameV1, String> for App {
    fn event(app: &mut Self, _: &frame::ZwlrScreencopyFrameV1, e: frame::Event, name: &String, _: &Connection, _: &QueueHandle<Self>) {
        app.frame_event(name, e);
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for App {
    fn event(_: &mut Self, _: &wl_buffer::WlBuffer, _: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_formats_and_flip() {
        // 2×2, шаг строки 12 (с хвостом): XRGB как есть, XBGR — R и B местами, переворот строк
        let mut d = vec![0u8; 24];
        d[0..4].copy_from_slice(&0x00112233u32.to_le_bytes());
        d[12..16].copy_from_slice(&0x00aabbccu32.to_le_bytes());
        let a = to_img(&d, wl_shm::Format::Xrgb8888, 2, 2, 12, false).unwrap();
        assert_eq!((a.px[0], a.px[2]), (0xff112233, 0xffaabbcc));
        let b = to_img(&d, wl_shm::Format::Xbgr8888, 2, 2, 12, true).unwrap();
        assert_eq!((b.px[0], b.px[2]), (0xffccbbaa, 0xff332211));
        assert!(to_img(&d, wl_shm::Format::Xrgb8888, 2, 3, 12, false).is_none());
    }
}
