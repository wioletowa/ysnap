// Оверлей: по поверхности layer-shell (overlay, на весь экран) на каждый монитор, клавиатура — на
// том, где фокус niri. Снимок экрана — в физических пикселях, холст — логический: застывший экран
// растягивает композитор (viewporter), как в шелле. Долгое (grim, кодирование, запись, wl-copy,
// tesseract) — в нитях, ответы приходят каналом. Процесс выходит, когда оверлей закрыт и работы нет.

use crate::config::Settings;
use crate::gfx::{Canvas, Img};
use crate::snap::{Mode, Snap};
use crate::ui::{Act, Cur, Ui, Zone};
use crate::Note;
use calloop::channel::{self, Sender};
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, LoopHandle};
use calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer, delegate_registry, delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{cursor_shape::CursorShapeManager, PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1};
use wayland_protocols::wp::fractional_scale::v1::client::{wp_fractional_scale_manager_v1 as frac_mgr, wp_fractional_scale_v1 as frac};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use ysnap_px::font::Fonts;
use ysnap_px::icons::Icons;
use ysnap_px::theme::Theme;

pub enum Msg {
    /// снимки мониторов через grim (capture.rs: запасной путь)
    Grabbed(HashMap<String, Img>),
    /// окна, монитор и окно с фокусом (niri::info)
    Niri(crate::niri::Info),
    SnapDone { path: Option<PathBuf>, copied: bool, err: Option<String>, notify: bool },
    SnapText(bool, Option<String>),
}

/// то, что рисует и читает оверлей (имена — как у шелла: snap.rs и snapdraw.rs почти без правок)
pub struct State {
    pub snap: Option<Snap>,
    pub started: Instant,
    pub set: Settings,
    pub t: Theme,
}

/// шрифты грузятся в своей нити, пока снимается экран: нужны только к первому кадру
pub struct Res {
    fonts: Option<Fonts>,
    loading: Option<std::thread::JoinHandle<Fonts>>,
}

impl Res {
    pub fn fonts(&mut self) -> &mut Fonts {
        if self.fonts.is_none() {
            let f = self.loading.take().and_then(|h| h.join().ok()).unwrap_or_else(|| Fonts::with(""));
            self.fonts = Some(f);
        }
        self.fonts.get_or_insert_with(|| Fonts::with(""))
    }
}

pub struct Surf {
    pub id: u32,
    layer: LayerSurface,
    pub output: String,
    pub w: i32,
    pub h: i32,
    scale: f64,
    frac: Option<frac::WpFractionalScaleV1>,
    vp: Option<wp_viewport::WpViewport>,
    pool: Option<SlotPool>,
    zones: Vec<Zone>,
    configured: bool,
    dirty: bool,
    frame_pending: bool,
    anim: bool,
}

pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
}

pub struct App {
    conn: Connection,
    pub(crate) qh: QueueHandle<App>,
    registry: RegistryState,
    seat: SeatState,
    pub(crate) outputs: OutputState,
    compositor: CompositorState,
    pub(crate) shm: Shm,
    layer_shell: LayerShell,
    frac_mgr: Option<frac_mgr::WpFractionalScaleManagerV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    cursor_mgr: Option<CursorShapeManager>,
    cursor_dev: Option<WpCursorShapeDeviceV1>,
    cursor_now: Option<Shape>,
    enter_serial: u32,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pub(crate) handle: LoopHandle<'static, App>,
    pub mods: Mods,
    pub surfs: HashMap<u32, Surf>,
    next_id: u32,
    ptr: Option<(u32, f64, f64)>,
    press: Option<(u32, Act)>,
    pub st: State,
    pub res: Res,
    icons: Icons,
    pub tx: Sender<Msg>,
    /// ждём снимков из нити (таймер, grim)
    pub waiting: bool,
    /// нити, которые ещё пишут, копируют или распознают
    pub jobs: u32,
    pub notes: Vec<Note>,
    /// `ysnap screen`: монитор в фокусе сразу, без оверлея
    pub instant: bool,
    pub(crate) screencopy: Option<wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1>,
    /// снимок в процессе (capture.rs)
    pub(crate) pending: Option<crate::capture::Pending>,
    /// YSNAP_TIMING: печатать, сколько мс от старта до снимка и первого кадра
    timing: Option<Instant>,
}

pub fn run(t0: Instant, t: Theme, set: Settings, fonts: std::thread::JoinHandle<Fonts>, mode: Mode, delay: u32) -> (i32, Vec<Note>) {
    let Ok(conn) = Connection::connect_to_env() else {
        eprintln!("ysnap: no Wayland display");
        return (1, vec![]);
    };
    let Ok((globals, mut queue)) = registry_queue_init::<App>(&conn) else { return (1, vec![]) };
    let qh = queue.handle();
    let Ok(mut ev) = EventLoop::<'static, App>::try_new() else { return (1, vec![]) };
    let handle = ev.handle();
    let (tx, rx) = channel::channel::<Msg>();
    let _ = handle.insert_source(rx, |e, _, app: &mut App| {
        if let channel::Event::Msg(m) = e {
            app.on_msg(m);
        }
    });
    let (Ok(compositor), Ok(shm), Ok(layer_shell)) = (CompositorState::bind(&globals, &qh), Shm::bind(&globals, &qh), LayerShell::bind(&globals, &qh)) else {
        eprintln!("ysnap: the compositor has no layer-shell");
        return (1, vec![]);
    };
    let mut app = App {
        conn: conn.clone(),
        qh: qh.clone(),
        registry: RegistryState::new(&globals),
        seat: SeatState::new(&globals, &qh),
        outputs: OutputState::new(&globals, &qh),
        compositor,
        shm,
        layer_shell,
        frac_mgr: globals.bind(&qh, 1..=1, ()).ok(),
        viewporter: globals.bind(&qh, 1..=1, ()).ok(),
        cursor_mgr: CursorShapeManager::bind(&globals, &qh).ok(),
        cursor_dev: None,
        cursor_now: None,
        enter_serial: 0,
        pointer: None,
        keyboard: None,
        handle: handle.clone(),
        mods: Mods { ctrl: false, shift: false },
        surfs: HashMap::new(),
        next_id: 1,
        ptr: None,
        press: None,
        st: State { snap: None, started: Instant::now(), set, t },
        res: Res { fonts: None, loading: Some(fonts) },
        icons: Icons::new(""),
        tx,
        waiting: false,
        jobs: 0,
        notes: vec![],
        instant: mode == Mode::Screen,
        screencopy: globals.bind(&qh, 1..=3, ()).ok(),
        pending: None,
        timing: std::env::var_os("YSNAP_TIMING").map(|_| t0),
    };
    // имена и размеры мониторов (wl_output и xdg_output) — до первого снимка
    for _ in 0..2 {
        if queue.roundtrip(&mut app).is_err() {
            return (1, vec![]);
        }
    }
    let _ = WaylandSource::new(conn.clone(), queue).insert(handle.clone());
    // вспышка и горячий угол (snap_tick)
    let _ = handle.insert_source(Timer::from_duration(Duration::from_millis(40)), |_, _, app: &mut App| {
        app.snap_tick();
        app.flush();
        TimeoutAction::ToDuration(Duration::from_millis(40))
    });
    // `ysnap window`: окно в фокусе сразу, без оверлея
    if mode == Mode::Window {
        app.window_shot(None);
    } else {
        app.snap_start(mode, delay);
    }
    loop {
        if ev.dispatch(Some(Duration::from_millis(500)), &mut app).is_err() {
            break;
        }
        app.flush();
        if !app.waiting && app.st.snap.is_none() && app.jobs == 0 {
            break;
        }
    }
    let _ = app.conn.flush();
    (0, std::mem::take(&mut app.notes))
}

impl App {
    fn on_msg(&mut self, m: Msg) {
        match m {
            Msg::Grabbed(imgs) => self.on_grabbed(imgs),
            Msg::Niri(info) => self.on_niri(info),
            Msg::SnapDone { path, copied, err, notify } => {
                self.jobs = self.jobs.saturating_sub(1);
                self.on_snap_done(path, copied, err, notify);
            }
            Msg::SnapText(qr, text) => {
                self.jobs = self.jobs.saturating_sub(1);
                self.on_snap_text(qr, text);
            }
        }
    }

    /// YSNAP_TIMING: этап и мс от старта процесса
    pub(crate) fn timing(&mut self, what: &str) {
        if let Some(t0) = self.timing {
            eprintln!("ysnap: {what} at {} ms", t0.elapsed().as_millis());
        }
    }

    /// имена мониторов (wl_output.name: «DP-1», как у grim и niri)
    pub fn output_names(&self) -> Vec<String> {
        self.outputs.outputs().filter_map(|o| self.outputs.info(&o).and_then(|i| i.name)).collect()
    }

    /// логический размер монитора: поверхность оверлея, иначе xdg_output
    pub fn output_size(&self, name: &str) -> (i32, i32) {
        if let Some(s) = self.surfs.values().find(|s| s.output == name && s.w > 0) {
            return (s.w, s.h);
        }
        self.outputs
            .outputs()
            .filter_map(|o| self.outputs.info(&o))
            .find(|i| i.name.as_deref() == Some(name))
            .and_then(|i| i.logical_size.or_else(|| i.modes.iter().find(|m| m.current).map(|m| m.dimensions)))
            .unwrap_or((1, 1))
    }

    /// поверхность оверлея на мониторе по имени
    pub fn mk_snap(&mut self, name: &str, kb: KeyboardInteractivity) {
        let Some(o) = self.outputs.outputs().find(|o| self.outputs.info(o).and_then(|i| i.name).as_deref() == Some(name)) else { return };
        let surface = self.compositor.create_surface(&self.qh);
        let id = self.next_id;
        self.next_id += 1;
        let frac = self.frac_mgr.as_ref().map(|m| m.get_fractional_scale(&surface, &self.qh, id));
        let vp = self.viewporter.as_ref().map(|v| v.get_viewport(&surface, &self.qh, ()));
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Overlay, Some("ysnap"), Some(&o));
        layer.set_anchor(Anchor::all());
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(kb);
        layer.commit();
        self.surfs.insert(id, Surf { id, layer, output: name.to_string(), w: 0, h: 0, scale: 1.0, frac, vp, pool: None, zones: vec![], configured: false, dirty: true, frame_pending: false, anim: false });
    }

    pub fn destroy_all(&mut self) {
        for (_, s) in self.surfs.drain() {
            if let Some(f) = &s.frac {
                f.destroy();
            }
            if let Some(v) = &s.vp {
                v.destroy();
            }
        }
        self.ptr = None;
        self.press = None;
        let _ = self.conn.flush();
    }

    pub fn dirty_all(&mut self) {
        for s in self.surfs.values_mut() {
            s.dirty = true;
        }
    }

    /// перерисовать оверлей одного монитора
    pub fn dirty_out(&mut self, name: &str) {
        for s in self.surfs.values_mut().filter(|s| s.output == name) {
            s.dirty = true;
        }
    }

    fn flush(&mut self) {
        let ids: Vec<u32> = self.surfs.values().filter(|s| s.dirty && s.configured && !s.frame_pending).map(|s| s.id).collect();
        for id in ids {
            self.draw(id);
        }
        let _ = self.conn.flush();
    }

    fn draw(&mut self, id: u32) {
        let Some(s) = self.surfs.get_mut(&id) else { return };
        s.dirty = false;
        let (w, h) = (s.w, s.h);
        if w <= 0 || h <= 0 {
            return;
        }
        // застывший экран — не пиксель-арт: буфер логический, растягивает композитор
        let need = (w * h * 4) as usize;
        if s.pool.as_ref().map(|p| p.len() < need).unwrap_or(true) {
            s.pool = SlotPool::new(need * 2, &self.shm).ok();
        }
        let Some(pool) = s.pool.as_mut() else { return };
        let Ok((buffer, bytes)) = pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else { return };
        let buf: &mut [u32] = unsafe { std::slice::from_raw_parts_mut(bytes.as_mut_ptr() as *mut u32, bytes.len() / 4) };
        // буфер пула хранит старый кадр: снимок сделан — оверлей должен стать прозрачным
        buf.fill(0);
        let ptr = self.ptr.filter(|p| p.0 == id).map(|p| (p.1, p.2));
        let down = self.press.as_ref().is_some_and(|p| p.0 == id);
        {
            let mut ui = Ui { cv: Canvas::new(buf, w, h), f: self.res.fonts(), ic: &mut self.icons, t: self.st.t, zones: vec![], ptr, down, anim: false, blink: false, tr: crate::i18n::tr };
            crate::snap::draw(&mut ui, &self.st, &s.output, w, h);
            s.zones = std::mem::take(&mut ui.zones);
            s.anim = ui.anim;
        }
        let surf = s.layer.wl_surface().clone();
        if let Some(vp) = &s.vp {
            vp.set_destination(w, h);
        }
        surf.damage_buffer(0, 0, w, h);
        if s.anim {
            surf.frame(&self.qh, surf.clone());
            s.frame_pending = true;
        }
        let _ = buffer.attach_to(&surf);
        surf.commit();
        if self.timing.is_some() {
            self.timing("first frame");
            self.timing = None;
        }
    }

    fn zone_at(&self, id: u32, x: f64, y: f64) -> Option<Zone> {
        self.surfs.get(&id)?.zones.iter().rev().find(|z| z.r.contains(x, y)).cloned()
    }

    fn set_cursor(&mut self, c: Shape) {
        if self.cursor_now == Some(c) {
            return;
        }
        self.cursor_now = Some(c);
        if let Some(d) = &self.cursor_dev {
            d.set_shape(self.enter_serial, c);
        }
    }

    fn surf_id(&self, s: &wl_surface::WlSurface) -> Option<u32> {
        self.surfs.values().find(|o| o.layer.wl_surface() == s).map(|o| o.id)
    }

    /// как в шелле: протяжка, рисование и лупа — snap_pointer; кнопки панелей — зоны (нажал и
    /// отпустил на той же)
    fn on_pointer(&mut self, e: &PointerEvent) {
        let Some(id) = self.surf_id(&e.surface) else { return };
        let prev = self.ptr.map(|p| p.0);
        let (x, y) = e.position;
        let on_btn = self.zone_at(id, x, y).map(|z| matches!(z.act, Act::SnapBtn(_))).unwrap_or(false);
        let took = match e.kind {
            PointerEventKind::Motion { .. } => self.snap_pointer(id, 0, x, y, 0, on_btn),
            PointerEventKind::Press { button, .. } => self.snap_pointer(id, 1, x, y, button, on_btn),
            PointerEventKind::Release { button, .. } if self.press.is_none() => self.snap_pointer(id, 2, x, y, button, on_btn),
            PointerEventKind::Axis { vertical, .. } => {
                let d = if vertical.value120 != 0 { vertical.value120.signum() } else { vertical.discrete.signum() };
                if d != 0 {
                    self.snap_wheel(d);
                }
                true
            }
            _ => false,
        };
        if self.surfs.contains_key(&id) {
            self.ptr = Some((id, x, y));
        }
        match e.kind {
            PointerEventKind::Enter { serial } => {
                self.enter_serial = serial;
                self.cursor_now = None;
                self.set_cursor(Shape::Crosshair);
            }
            PointerEventKind::Leave { .. } => {
                self.ptr = None;
                self.press = None;
            }
            PointerEventKind::Motion { .. } => {
                let shape = match self.zone_at(id, x, y).map(|z| z.cur) {
                    Some(Cur::Pointer) => Shape::Pointer,
                    Some(Cur::Default) if on_btn => Shape::Default,
                    _ => Shape::Crosshair,
                };
                self.set_cursor(shape);
            }
            PointerEventKind::Press { button, .. } if !took && button == 0x110 => {
                self.press = self.zone_at(id, x, y).map(|z| (id, z.act));
            }
            PointerEventKind::Release { .. } => {
                if let Some((pid, a)) = self.press.take() {
                    if pid == id && self.zone_at(id, x, y).is_some_and(|z| z.act == a) {
                        if let Act::SnapBtn(b) = a {
                            self.snap_btn(b);
                        }
                    }
                }
            }
            _ => {}
        }
        // мышь меняет только свой монитор (и тот, откуда ушла: там гаснут лупа и перекрестье)
        for sid in [Some(id), prev].into_iter().flatten() {
            if let Some(s) = self.surfs.get_mut(&sid) {
                s.dirty = true;
            }
        }
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surf: &wl_surface::WlSurface, _: u32) {
        for s in self.surfs.values_mut().filter(|s| s.layer.wl_surface() == surf) {
            s.frame_pending = false;
            if s.anim {
                s.dirty = true;
            }
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, l: &LayerSurface) {
        self.surfs.retain(|_, s| &s.layer != l);
        // монитор пропал: без поверхностей оверлей закрыт
        if self.surfs.is_empty() {
            self.st.snap = None;
        }
    }
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, l: &LayerSurface, c: LayerSurfaceConfigure, _: u32) {
        for s in self.surfs.values_mut().filter(|s| &s.layer == l) {
            s.w = c.new_size.0 as i32;
            s.h = c.new_size.1 as i32;
            s.configured = true;
            s.dirty = true;
            s.frame_pending = false;
        }
        self.flush();
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            let h = self.handle.clone();
            self.keyboard = self
                .seat
                .get_keyboard_with_repeat(
                    qh,
                    &seat,
                    None,
                    h,
                    Box::new(|app: &mut App, _, e| {
                        app.snap_key(&e);
                        app.flush();
                    }),
                )
                .ok();
        }
        if cap == Capability::Pointer && self.pointer.is_none() {
            if let Ok(p) = self.seat.get_pointer(qh, &seat) {
                if let Some(m) = &self.cursor_mgr {
                    self.cursor_dev = Some(m.get_shape_device(&p, qh));
                }
                self.pointer = Some(p);
            }
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
        if cap == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
            self.cursor_dev = None;
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for App {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, e: KeyEvent) {
        self.snap_key(&e);
    }
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, e: KeyEvent) {
        // пробел — переключатель лупы: зажатый не должен щёлкать им туда-сюда
        if e.keysym != Keysym::space {
            self.snap_key(&e);
        }
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, m: Modifiers, _: RawModifiers, _: u32) {
        self.mods = Mods { ctrl: m.ctrl, shift: m.shift };
    }
}

impl PointerHandler for App {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for e in events {
            self.on_pointer(e);
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

impl Dispatch<frac_mgr::WpFractionalScaleManagerV1, ()> for App {
    fn event(_: &mut Self, _: &frac_mgr::WpFractionalScaleManagerV1, _: frac_mgr::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewporter::WpViewporter, ()> for App {
    fn event(_: &mut Self, _: &wp_viewporter::WpViewporter, _: wp_viewporter::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewport::WpViewport, ()> for App {
    fn event(_: &mut Self, _: &wp_viewport::WpViewport, _: wp_viewport::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<frac::WpFractionalScaleV1, u32> for App {
    fn event(app: &mut Self, _: &frac::WpFractionalScaleV1, e: frac::Event, id: &u32, _: &Connection, _: &QueueHandle<Self>) {
        if let frac::Event::PreferredScale { scale } = e {
            if let Some(s) = app.surfs.get_mut(id) {
                s.scale = scale as f64 / 120.0;
                s.dirty = true;
            }
        }
    }
}

delegate_compositor!(App);
delegate_output!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_keyboard!(App);
delegate_pointer!(App);
delegate_layer!(App);
delegate_registry!(App);
