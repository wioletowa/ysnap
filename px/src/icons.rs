// Иконки из темы (freedesktop: index.theme, Inherits, каталоги по размерам) и программы из
// .desktop-файлов. Иконка рисуется один раз нужного размера и живёт в кэше.

use crate::gfx::Img;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

struct ThemeDir {
    path: PathBuf,
    size: i32,
    scalable: bool,
}

pub struct Icons {
    dirs: Vec<ThemeDir>,
    found: HashMap<String, Option<PathBuf>>,
    imgs: HashMap<(String, i32), Option<std::rc::Rc<Img>>>,
    /// значки программ пиксельными в палитре: обводка и 4 тона от тёмного к светлому
    pixel: Option<[u32; 5]>,
    /// пиксельные копии чужих картинок (значки трея) по отпечатку картинки и палитры
    pixeled: HashMap<u64, std::rc::Rc<Img>>,
}

fn icon_roots() -> Vec<PathBuf> {
    let h = crate::paths::home();
    let mut v = vec![crate::paths::data_dir().join("icons"), h.join(".icons")];
    v.extend(crate::paths::data_dirs().into_iter().map(|d| d.join("icons")));
    v.push("/var/lib/flatpak/exports/share/icons".into());
    v
}

fn parse_ini(text: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut out: Vec<(String, HashMap<String, String>)> = vec![];
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') && l.ends_with(']') {
            out.push((l[1..l.len() - 1].to_string(), HashMap::new()));
        } else if let Some((k, v)) = l.split_once('=') {
            if let Some(last) = out.last_mut() {
                last.1.entry(k.trim().to_string()).or_insert_with(|| v.trim().to_string());
            }
        }
    }
    out
}

impl Icons {
    pub fn new(theme: &str) -> Icons {
        let mut ic = Icons { dirs: vec![], found: HashMap::new(), imgs: HashMap::new(), pixel: None, pixeled: HashMap::new() };
        let roots = icon_roots();
        let mut order: Vec<String> = vec![];
        let mut queue: Vec<String> = vec![];
        let first = if theme.is_empty() {
            ["Gruvbox-Plus-Ink", "Gruvbox-Plus-Dark", "Papirus-Dark", "Papirus", "breeze-dark", "Adwaita"]
                .iter()
                .find(|t| roots.iter().any(|r| r.join(t).join("index.theme").exists()))
                .map(|s| s.to_string())
                .unwrap_or_else(|| "hicolor".into())
        } else {
            theme.to_string()
        };
        queue.push(first);
        while let Some(t) = queue.pop() {
            if order.contains(&t) {
                continue;
            }
            order.push(t.clone());
            for r in &roots {
                let base = r.join(&t);
                let Ok(txt) = std::fs::read_to_string(base.join("index.theme")) else { continue };
                let ini = parse_ini(&txt);
                let head = ini.iter().find(|(n, _)| n == "Icon Theme");
                let mut inherits: Vec<String> = head.and_then(|(_, m)| m.get("Inherits")).map(|s| s.split(',').map(|x| x.trim().to_string()).collect()).unwrap_or_default();
                inherits.reverse();
                queue.extend(inherits);
                let dirs: Vec<String> = head.and_then(|(_, m)| m.get("Directories")).map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
                let mut td: Vec<ThemeDir> = vec![];
                for d in dirs {
                    let Some((_, m)) = ini.iter().find(|(n, _)| *n == d) else { continue };
                    let size = m.get("Size").and_then(|s| s.parse().ok()).unwrap_or(48);
                    let scale: i32 = m.get("Scale").and_then(|s| s.parse().ok()).unwrap_or(1);
                    if scale != 1 {
                        continue;
                    }
                    let ctx = m.get("Context").map(|s| s.as_str()).unwrap_or("");
                    if !matches!(ctx, "Applications" | "Apps" | "Status" | "Devices" | "Places" | "Actions" | "Panel" | "Categories" | "") {
                        continue;
                    }
                    let scalable = m.get("Type").map(|t| t == "Scalable").unwrap_or(false);
                    let path = base.join(&d);
                    if path.is_dir() {
                        td.push(ThemeDir { path, size, scalable });
                    }
                }
                // ближе к 16 и векторные — раньше
                td.sort_by_key(|d| if d.scalable { 1 } else { (d.size - 16).abs() * 2 + if d.size < 16 { 1 } else { 0 } });
                ic.dirs.extend(td);
            }
            if queue.is_empty() && !order.iter().any(|x| x == "hicolor") {
                queue.push("hicolor".into());
            }
        }
        ic.dirs.push(ThemeDir { path: "/usr/share/pixmaps".into(), size: 48, scalable: false });
        ic
    }

    pub fn find(&mut self, name: &str) -> Option<PathBuf> {
        if name.is_empty() {
            return None;
        }
        if let Some(p) = self.found.get(name) {
            return p.clone();
        }
        let r = if name.starts_with('/') {
            Path::new(name).exists().then(|| PathBuf::from(name))
        } else {
            let mut hit = None;
            'o: for d in &self.dirs {
                for ext in ["svg", "png", "xpm"] {
                    if ext == "xpm" {
                        continue;
                    }
                    let p = d.path.join(format!("{name}.{ext}"));
                    if p.exists() {
                        hit = Some(p);
                        break 'o;
                    }
                }
            }
            hit
        };
        self.found.insert(name.to_string(), r.clone());
        r
    }

    /// иконка по имени (или пути) нужного размера; нет — None
    pub fn get(&mut self, name: &str, size: i32) -> Option<std::rc::Rc<Img>> {
        let key = (name.to_string(), size);
        if let Some(i) = self.imgs.get(&key) {
            return i.clone();
        }
        let img = self.find(name).and_then(|p| load_file(&p, size)).map(std::rc::Rc::new);
        self.imgs.insert(key, img.clone());
        img
    }

    /// значок из файла (IconThemePath у программ трея): кеш по пути и времени изменения, чтобы
    /// не читать и не декодировать его на каждой перерисовке бара
    pub fn file(&mut self, p: &Path, size: i32) -> Option<std::rc::Rc<Img>> {
        let at = std::fs::metadata(p).ok()?.modified().ok()?;
        let key = (format!("\u{1}{}\u{1}{at:?}", p.display()), size);
        if let Some(i) = self.imgs.get(&key) {
            return i.clone();
        }
        let img = load_file(p, size).map(std::rc::Rc::new);
        self.imgs.insert(key, img.clone());
        img
    }

    /// включить/выключить пиксельные значки программ (тона палитры: обводка, тёмный → светлый)
    pub fn set_pixel(&mut self, ramp: Option<[u32; 5]>) {
        if self.pixel != ramp {
            self.pixel = ramp;
            self.imgs.retain(|k, _| !k.0.starts_with('\0'));
        }
    }

    /// готовая картинка (значок трея) пиксельной в тонах `ramp`; копии помнятся по отпечатку
    /// пикселей и палитры, чтобы бар не пересчитывал их каждую секунду
    pub fn pixeled(&mut self, img: &Img, ramp: &[u32; 5], size: i32) -> std::rc::Rc<Img> {
        let mut h = crate::fnv(crate::FNV0, &(size as u32).to_le_bytes());
        for p in ramp.iter().chain(img.px.iter()) {
            h = crate::fnv(h, &p.to_le_bytes());
        }
        if let Some(i) = self.pixeled.get(&h) {
            return i.clone();
        }
        // значки трея меняются (мигают, показывают счёт): старые копии не копить
        if self.pixeled.len() > 64 {
            self.pixeled.clear();
        }
        let out = std::rc::Rc::new(pixelate(img, ramp, size));
        self.pixeled.insert(h, out.clone());
        out
    }

    /// значок программы: как есть или пиксельный (сетка 16×16, тона палитры, дизеринг Байера)
    pub fn app(&mut self, name: &str, size: i32) -> Option<std::rc::Rc<Img>> {
        let Some(ramp) = self.pixel else { return self.app_raw(name, size) };
        let key = (format!("\0{name}"), size);
        if let Some(i) = self.imgs.get(&key) {
            return i.clone();
        }
        let lo = size.min(16);
        let img = self.app_raw(name, lo).map(|r| std::rc::Rc::new(pixelate(&r, &ramp, size)));
        self.imgs.insert(key, img.clone());
        img
    }

    /// иконка с запасными вариантами: имя, в нижнем регистре, «программа»
    fn app_raw(&mut self, name: &str, size: i32) -> Option<std::rc::Rc<Img>> {
        if let Some(i) = self.get(name, size) {
            return Some(i);
        }
        let low = name.to_lowercase();
        if low != name {
            if let Some(i) = self.get(&low, size) {
                return Some(i);
            }
        }
        self.get("application-x-executable", size)
    }

    pub fn drop_cache(&mut self) {
        self.imgs.clear();
    }
}

const BAYER4: [[f32; 4]; 4] = [[0.0, 8.0, 2.0, 10.0], [12.0, 4.0, 14.0, 6.0], [3.0, 11.0, 1.0, 9.0], [15.0, 7.0, 13.0, 5.0]];

/// маленькая картинка → пиксельный значок size×size: жёсткая альфа, яркость растянута на
/// 4 тона палитры с дизерингом Байера, тёмная обводка по контуру, клетки по целому множителю
pub fn pixelate(src: &Img, ramp: &[u32; 5], size: i32) -> Img {
    let (w, h) = (src.w, src.h);
    let lum = |p: u32| {
        let a = (p >> 24) as f32;
        if a < 1.0 {
            return 0.0;
        }
        let c = |s: u32| ((p >> s) & 0xff) as f32 / a;
        0.299 * c(16) + 0.587 * c(8) + 0.114 * c(0)
    };
    let solid = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h && (src.px[(y * w + x) as usize] >> 24) >= 110;
    let (mut lo, mut hi) = (1.0f32, 0.0f32);
    for y in 0..h {
        for x in 0..w {
            if solid(x, y) {
                let l = lum(src.px[(y * w + x) as usize]);
                lo = lo.min(l);
                hi = hi.max(l);
            }
        }
    }
    let span = (hi - lo).max(0.2);
    let mut grid = vec![0u32; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            grid[(y * w + x) as usize] = if solid(x, y) {
                let l = (lum(src.px[(y * w + x) as usize]) - lo) / span;
                let d = BAYER4[(y & 3) as usize][(x & 3) as usize] / 16.0 - 0.47;
                let i = (l * 3.0 + d * 0.9).round().clamp(0.0, 3.0) as usize;
                ramp[1 + i] | 0xff000000
            } else if solid(x - 1, y) || solid(x + 1, y) || solid(x, y - 1) || solid(x, y + 1) {
                ramp[0] | 0xff000000
            } else {
                0
            };
        }
    }
    let cell = (size / w.max(h)).max(1);
    let (ox, oy) = ((size - w * cell) / 2, (size - h * cell) / 2);
    let mut px = vec![0u32; (size * size) as usize];
    for y in 0..h * cell {
        for x in 0..w * cell {
            let (dx, dy) = (x + ox, y + oy);
            if dx < size && dy < size {
                px[(dy * size + dx) as usize] = grid[((y / cell) * w + x / cell) as usize];
            }
        }
    }
    Img { w: size, h: size, px }
}

/// потолки для картинок со стороны (значки уведомлений, обложки, значки трея): файл и память декодера
pub const MAX_FILE: u64 = 16 << 20;
pub const MAX_DECODE: u64 = 64 << 20;

/// картинка из байтов с потолком памяти и размеров: «бомба» (100000×100000 в маленьком png) не
/// раздует процесс, декодер просто откажется
pub fn decode(data: &[u8], max_alloc: u64) -> Option<image::DynamicImage> {
    let mut rd = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()?;
    let mut lim = image::Limits::default();
    lim.max_alloc = Some(max_alloc);
    lim.max_image_width = Some(8192);
    lim.max_image_height = Some(8192);
    rd.limits(lim);
    rd.decode().ok()
}

/// кеш миниатюр: не открытые 120 дней уходят, и всего не больше 512 МБ (раз в сутки, из нити)
pub fn thumbs_tidy() {
    crate::paths::tidy(&crate::paths::cache_dir().join("ysnap/thumbs"), 120, 512 << 20);
}

/// где лежит миниатюра (~/.cache/ysnap/thumbs): ключ — путь, время изменения и размер
pub fn thumb_path(path: &Path, mtime: i64, size: i32) -> PathBuf {
    let h = crate::fnv(crate::FNV0, format!("{}|{}|{}", path.display(), mtime, size).as_bytes());
    crate::paths::cache_dir().join("ysnap/thumbs").join(format!("{h:016x}.png"))
}

/// миниатюра картинки, не больше size по большей стороне. Кеш:
/// второй раз картинка не декодируется. Гигантские (больше 40 Мпикс) не декодируются вовсе
pub fn thumb(path: &Path, mtime: i64, size: i32) -> Option<image::RgbaImage> {
    let cp = thumb_path(path, mtime, size);
    if let Ok(img) = image::open(&cp) {
        // нужная миниатюра молодеет (раз в неделю, не на каждое чтение): уборка кеша
        // (paths::tidy) убирает по времени изменения только то, что давно не открывали
        if let Ok(m) = std::fs::metadata(&cp).and_then(|m| m.modified()) {
            if m.elapsed().map(|d| d.as_secs() > 7 * 86_400).unwrap_or(false) {
                let _ = std::fs::File::options().write(true).open(&cp).and_then(|f| f.set_modified(std::time::SystemTime::now()));
            }
        }
        return Some(img.into_rgba8());
    }
    let rd = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let (w, h) = rd.into_dimensions().ok()?;
    if (w as u64) * (h as u64) > 40_000_000 {
        return None;
    }
    let mut rd = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut lim = image::Limits::default();
    lim.max_alloc = Some(256 << 20);
    rd.limits(lim);
    let t = rd.decode().ok()?.thumbnail(size as u32, size as u32).to_rgba8();
    let _ = std::fs::create_dir_all(cp.parent()?);
    let _ = t.save(&cp);
    Some(t)
}

pub fn load_file(p: &Path, size: i32) -> Option<Img> {
    let data = crate::paths::read_capped(p, MAX_FILE)?;
    if p.extension().map(|e| e == "svg" || e == "svgz").unwrap_or(false) {
        return svg(&data, size);
    }
    Some(fit(decode(&data, MAX_DECODE)?.to_rgba8(), size))
}

pub fn svg(data: &[u8], size: i32) -> Option<Img> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).ok()?;
    let s = tree.size();
    let k = size as f32 / s.width().max(s.height());
    let mut pm = tiny_skia::Pixmap::new(size as u32, size as u32)?;
    let dx = (size as f32 - s.width() * k) / 2.0;
    let dy = (size as f32 - s.height() * k) / 2.0;
    resvg::render(&tree, tiny_skia::Transform::from_row(k, 0.0, 0.0, k, dx, dy), &mut pm.as_mut());
    let px = pm.data().chunks_exact(4).map(|c| (c[3] as u32) << 24 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32).collect();
    Some(Img { w: size, h: size, px })
}

/// вписать в квадрат size без сглаживания (пиксели остаются пикселями)
/// вписать в mw×mh с сохранением пропорций, без полей (превью скриншота)
pub fn fit_in(img: image::RgbaImage, mw: i32, mh: i32) -> Img {
    let (w, h) = (img.width().max(1) as f32, img.height().max(1) as f32);
    let k = (mw as f32 / w).min(mh as f32 / h).min(1.0);
    let (nw, nh) = (((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1));
    let r = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
    let px = r
        .pixels()
        .map(|p| {
            let [cr, cg, cb, ca] = p.0;
            let a = ca as u32;
            let m = |v: u8| (v as u32 * a + 127) / 255;
            a << 24 | m(cr) << 16 | m(cg) << 8 | m(cb)
        })
        .collect();
    Img { w: nw as i32, h: nh as i32, px }
}

pub fn fit(img: image::RgbaImage, size: i32) -> Img {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let k = size as f32 / w.max(h) as f32;
    let (nw, nh) = (((w as f32 * k).round() as i32).max(1), ((h as f32 * k).round() as i32).max(1));
    let filter = if k < 1.0 { image::imageops::FilterType::Triangle } else { image::imageops::FilterType::Nearest };
    let r = image::imageops::resize(&img, nw as u32, nh as u32, filter);
    let mut px = vec![0u32; (size * size) as usize];
    let (ox, oy) = ((size - nw) / 2, (size - nh) / 2);
    for (x, y, p) in r.enumerate_pixels() {
        let [cr, cg, cb, ca] = p.0;
        let a = ca as u32;
        let m = |v: u8| (v as u32 * a + 127) / 255;
        px[((y as i32 + oy) * size + x as i32 + ox) as usize] = a << 24 | m(cr) << 16 | m(cg) << 8 | m(cb);
    }
    Img { w: size, h: size, px }
}

/// картинка как есть (обои, обложки), в предумноженный ARGB
pub fn from_rgba(img: &image::RgbaImage) -> Img {
    let px = img
        .pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            let a32 = a as u32;
            let m = |v: u8| (v as u32 * a32 + 127) / 255;
            a32 << 24 | m(r) << 16 | m(g) << 8 | m(b)
        })
        .collect();
    Img { w: img.width() as i32, h: img.height() as i32, px }
}

// ── программы ───────────────────────────────────────────────────────────────
#[derive(Clone, Debug, Default)]
pub struct App {
    pub id: String,
    pub name: String,
    pub generic: String,
    pub comment: String,
    pub keywords: String,
    pub icon: String,
    pub exec: String,
    pub terminal: bool,
    pub wm_class: String,
    /// MimeType из .desktop: что программа умеет открывать
    pub mime: Vec<String>,
    /// в Exec есть %f/%F/%u/%U — принимает файлы
    pub takes_files: bool,
}

fn app_dirs() -> Vec<PathBuf> {
    let mut v = vec![crate::paths::data_dir().join("applications")];
    v.extend(crate::paths::data_dirs().into_iter().map(|d| d.join("applications")));
    v.push("/var/lib/flatpak/exports/share/applications".into());
    v.push(crate::paths::data_dir().join("flatpak/exports/share/applications"));
    v
}

fn localized(m: &HashMap<String, String>, key: &str) -> String {
    let lang = std::env::var("LANG").unwrap_or_default();
    let short: String = lang.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    for k in [format!("{key}[{}]", lang.split('.').next().unwrap_or("")), format!("{key}[{short}]"), key.to_string()] {
        if let Some(v) = m.get(&k) {
            return v.clone();
        }
    }
    String::new()
}

pub fn load_apps() -> Vec<App> {
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut out = vec![];
    for d in app_dirs() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        // подкаталоги (kde4/…) — на один уровень
        let subs: Vec<PathBuf> = files.iter().filter(|p| p.is_dir()).cloned().collect();
        for s in subs {
            if let Ok(rd) = std::fs::read_dir(&s) {
                files.extend(rd.flatten().map(|e| e.path()));
            }
        }
        for p in files {
            if p.extension().map(|e| e != "desktop").unwrap_or(true) {
                continue;
            }
            let id = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if seen.contains_key(&id) {
                continue;
            }
            seen.insert(id.clone(), ());
            let Ok(txt) = std::fs::read_to_string(&p) else { continue };
            let ini = parse_ini(&txt);
            let Some((_, m)) = ini.iter().find(|(n, _)| n == "Desktop Entry") else { continue };
            if m.get("Type").map(|t| t != "Application").unwrap_or(false) {
                continue;
            }
            let yes = |k: &str| m.get(k).map(|v| v == "true").unwrap_or(false);
            if yes("NoDisplay") || yes("Hidden") {
                continue;
            }
            if let Some(only) = m.get("OnlyShowIn") {
                if !only.split(';').any(|x| x == "niri" || x.is_empty()) {
                    continue;
                }
            }
            out.push(App {
                id,
                name: localized(m, "Name"),
                generic: localized(m, "GenericName"),
                comment: localized(m, "Comment"),
                keywords: localized(m, "Keywords").replace(';', " "),
                icon: m.get("Icon").cloned().unwrap_or_default(),
                exec: m.get("Exec").cloned().unwrap_or_default(),
                terminal: yes("Terminal"),
                wm_class: m.get("StartupWMClass").cloned().unwrap_or_default(),
                mime: m.get("MimeType").map(|v| v.split(';').filter(|x| !x.is_empty()).map(|x| x.to_string()).collect()).unwrap_or_default(),
                takes_files: m.get("Exec").map(|e| ["%f", "%F", "%u", "%U"].iter().any(|c| e.contains(c))).unwrap_or(false),
            });
        }
    }
    // ключ — один раз на программу, а не две строки на каждое сравнение
    out.sort_by_cached_key(|a| a.name.to_lowercase());
    out
}

/// иконка окна по app_id: .desktop с тем же именем, StartupWMClass, последний кусок id
pub fn icon_for(apps: &[App], app_id: &str) -> String {
    if app_id.is_empty() {
        return String::new();
    }
    if let Some(n) = app_id.strip_prefix("steam_app_") {
        return format!("steam_icon_{n}");
    }
    let low = app_id.to_lowercase();
    let tail = low.rsplit('.').next().unwrap_or(&low).to_string();
    let hit = apps
        .iter()
        .find(|a| a.id.to_lowercase() == low)
        .or_else(|| apps.iter().find(|a| a.wm_class.to_lowercase() == low))
        .or_else(|| apps.iter().find(|a| a.id.to_lowercase().rsplit('.').next() == Some(&tail)))
        .or_else(|| apps.iter().find(|a| a.name.to_lowercase() == low));
    match hit {
        Some(a) if !a.icon.is_empty() => a.icon.clone(),
        _ => app_id.to_string(),
    }
}

/// Exec из .desktop → слова: кавычки и \\-экранирование внутри них (спецификация Desktop Entry).
/// Без оболочки: программа запускается напрямую, имена файлов не разбираются как команды.
fn exec_words(exec: &str) -> Vec<String> {
    let mut v = vec![];
    let mut cur = String::new();
    let mut q = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => q = !q,
            '\\' if q => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            ' ' if !q => {
                if !cur.is_empty() {
                    v.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        v.push(cur);
    }
    v
}

/// команда из Exec без %-кодов
pub fn exec_argv(exec: &str) -> Vec<String> {
    exec_words(exec).into_iter().filter(|a| !(a.len() == 2 && a.starts_with('%'))).map(|a| a.replace("%%", "%")).collect()
}

/// команда из Exec с файлами: %f/%u — по одному (вернёт несколько команд), %F/%U — все разом;
/// кодов нет — файлы в конец
pub fn exec_with(exec: &str, files: &[PathBuf]) -> Vec<Vec<String>> {
    let raw = exec_words(exec);
    let uri = |p: &PathBuf| file_uri(p);
    let path = |p: &PathBuf| p.to_string_lossy().to_string();
    let multi = raw.iter().any(|a| a == "%F" || a == "%U");
    let single = raw.iter().any(|a| a == "%f" || a == "%u");
    let build = |set: &[PathBuf]| -> Vec<String> {
        let mut out = vec![];
        for a in &raw {
            match a.as_str() {
                "%F" => out.extend(set.iter().map(path)),
                "%U" => out.extend(set.iter().map(uri)),
                "%f" => out.extend(set.first().map(path)),
                "%u" => out.extend(set.first().map(uri)),
                x if x.len() == 2 && x.starts_with('%') => {}
                x => out.push(x.replace("%%", "%")),
            }
        }
        out
    };
    if multi || files.is_empty() {
        vec![build(files)]
    } else if single {
        files.iter().map(|f| build(std::slice::from_ref(f))).collect()
    } else {
        let mut v = build(&[]);
        v.extend(files.iter().map(path));
        vec![v]
    }
}

/// file:// с процентным кодированием
pub fn file_uri(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::from("file://");
    // байты пути как есть: имя не в UTF-8 не превращается в «�»
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// обратно из file:// (другие схемы — None)
pub fn uri_path(u: &str) -> Option<PathBuf> {
    let rest = u.trim().strip_prefix("file://")?;
    let rest = if rest.starts_with('/') { rest } else { rest.split_once('/').map(|(_, r)| r).unwrap_or("") };
    let b = rest.as_bytes();
    let mut out = Vec::with_capacity(b.len() + 1);
    if !rest.starts_with('/') {
        out.push(b'/');
    }
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}
