// ysnap — пиксельная скриншотилка для niri: экран замораживается (wlr-screencopy, запасной путь —
// grim), поверх — оверлей на каждом мониторе: выделение, окно, экран, разметка, пипетка, текст
// (tesseract) и QR (zbarimg). Готовое — в буфер (wl-copy) и/или в папку, уведомление — notify-send.
// Без демона: снимок, вопросы к niri (сокет) и шрифты (в нити, с кешем) идут параллельно.
//
//   ysnap [--delay N]   область: оверлей (Print)
//   ysnap window        окно в фокусе с рамкой и тенью, сразу (как screenshot-window)
//   ysnap screen        монитор в фокусе, сразу (как screenshot-screen)
//   ysnap ocr FILE      текст с картинки в stdout
//   ysnap theme         какие цвета и шрифт взяты из системы
//
// Нужен wl-copy; по желанию — notify-send, tesseract, zbarimg, zenity или kdialog («сохранить
// как…»). Цвета и шрифт — системные (gtk.rs), настройки — ~/.config/ysnap.json (config.rs).
// YSNAP_TIMING=1 — печатать, через сколько мс готовы снимок и первый кадр.

mod app;
mod blur;
mod capture;
mod config;
mod gtk;
mod i18n;
mod niri;
mod ocr;
mod re;
mod snap;
mod snapdraw;
mod ui;
mod winshot;

pub use ysnap_px::{font, gfx, icons, theme};

use std::path::PathBuf;

/// уведомление после закрытия оверлея (notify-send); open — сохранённый файл (миниатюра)
pub struct Note {
    pub summary: String,
    pub body: String,
    pub open: Option<PathBuf>,
}

pub fn which(exe: &str) -> bool {
    ysnap_px::paths::find_exe(exe).is_some()
}

fn main() {
    let t0 = std::time::Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // первый запуск: конфиг с подсказками (на языке системы — свой язык ещё не прочитан)
    if !config::path().exists() {
        i18n::set("");
        config::create_template();
    }
    let v = config::json();
    let look = ysnap_px::look::Look::from_json(&v, None);
    i18n::set(&look.lang);
    let code = match args.first().map(String::as_str) {
        Some("--version") => {
            println!("ysnap {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("-h") | Some("--help") => {
            println!("ysnap [area|window|screen] [--delay N] | ocr FILE | theme");
            0
        }
        Some("ocr") => ocr::cli(&args[1..]),
        Some("theme") => {
            let t = gtk::theme();
            println!("font: {}", gtk::font());
            println!("colors: {}", if t.is_some() { "gtk.css" } else { "built-in (no palette in gtk.css)" });
            print!("{}", t.unwrap_or(look.theme).palette_json());
            0
        }
        _ => {
            let set = config::Settings::from_json(&v);
            config::ensure(&v, &set);
            shoot(t0, &args, look, set)
        }
    };
    std::process::exit(code);
}

fn shoot(t0: std::time::Instant, args: &[String], look: ysnap_px::look::Look, set: config::Settings) -> i32 {
    let mode = match args.iter().find(|a| !a.starts_with('-')).map(String::as_str) {
        Some("window") => snap::Mode::Window,
        Some("screen") => snap::Mode::Screen,
        _ => snap::Mode::Area,
    };
    let delay = args.iter().position(|a| a == "--delay").and_then(|i| args.get(i + 1)).and_then(|d| d.parse().ok()).unwrap_or(0);
    // один оверлей за раз: второй Print, пока открыт первый, ничего не делает
    let Some(lock) = single() else { return 0 };
    let theme = gtk::theme().unwrap_or(look.theme);
    // шрифты (fontconfig) — в своей нити, параллельно со снимком экрана
    let fonts = std::thread::spawn(|| ysnap_px::font::Fonts::with(&gtk::font()));
    let (code, notes) = app::run(t0, theme, set, fonts, mode, delay);
    drop(lock);
    for n in notes {
        notify(&n);
    }
    code
}

/// замок в XDG_RUNTIME_DIR: None — уже запущен
fn single() -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("ysnap.lock")).ok()?;
    // SAFETY: flock на своём дескрипторе
    let ok = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    ok.then_some(f)
}

/// notify-send без кнопок; у снимка в файле — миниатюра (сам файл)
fn notify(n: &Note) {
    use std::process::{Command, Stdio};
    if !which("notify-send") {
        eprintln!("ysnap: {} {}", n.summary, n.body);
        return;
    }
    let mut c = Command::new("notify-send");
    c.args(["-a", "ysnap"]);
    if let Some(p) = &n.open {
        c.arg("-i").arg(p);
    }
    let _ = c.arg("--").arg(&n.summary).arg(&n.body).stdin(Stdio::null()).stderr(Stdio::null()).status();
}
