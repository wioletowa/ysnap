// Настройки: свой файл ~/.config/ysnap.json. Цвета берутся из gtk.css (gtk.rs); accent и light
// здесь — только когда там палитры нет. lang: "" — язык системы, иначе "ru", "en", "pt-BR".
// Лупу (пробел) и недавние цвета (меню цветов) программа записывает сама; save и screenshotPath
// дописывает при запуске, если их нет, — чтобы было видно, что настроить. Остальное не трогает.
//
//   save            "both" — в папку и в буфер, "folder" — только в папку, "clipboard" — только буфер
//   screenshotPath  куда сохранять: шаблон strftime, как screenshot-path у niri (%d %m %y %Y %H %M %S…)
//
//   { "save": "both", "screenshotPath": "~/Pictures/Screenshot/%d-%m-%y %H-%M-%S.png",
//     "shotLoupe": true, "shotNotify": true, "shotConfirm": "enter", "lang": "",
//     "recentColors": ["#e02424"] }

use crate::gfx::C;

/// недавних цветов в меню
pub const RECENT: usize = 8;

pub const DEFAULT_PATH: &str = "~/Pictures/Screenshot/%d-%m-%y %H-%M-%S.png";

/// куда идёт готовый снимок
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Save {
    Clipboard,
    Folder,
    Both,
}

impl Save {
    pub fn from_id(s: &str) -> Option<Save> {
        Some(match s.trim() {
            "clipboard" => Save::Clipboard,
            "folder" => Save::Folder,
            "both" => Save::Both,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            Save::Clipboard => "clipboard",
            Save::Folder => "folder",
            Save::Both => "both",
        }
    }
    pub fn copy(self) -> bool {
        self != Save::Folder
    }
    pub fn file(self) -> bool {
        self != Save::Clipboard
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub save: Save,
    /// шаблон пути файла (strftime, ~ — домашняя папка)
    pub path: String,
    pub shot_loupe: bool,
    pub shot_notify: bool,
    /// "enter" — выделил, потом enter; "instant" — выделил, и снимок готов
    pub shot_confirm: String,
    /// недавние цвета, новые — первыми
    pub recent: Vec<C>,
}

pub fn path() -> std::path::PathBuf {
    ysnap_px::paths::config_dir().join("ysnap.json")
}

pub fn json() -> serde_json::Value {
    std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(serde_json::Value::Null)
}

/// первый запуск: файла нет — создать со всеми ключами и подсказками («# ключ»: что это и какие
/// значения бывают; программа такие ключи не читает, а при записи сохраняет на месте)
pub fn create_template() {
    use serde_json::Value::{Array, Bool, String as S};
    let p = path();
    if p.exists() {
        return;
    }
    let hint = |ru: &str, en: &str| S(if crate::i18n::translated() { en } else { ru }.into());
    let mut o = serde_json::Map::new();
    let mut put = |k: &str, h: serde_json::Value, v: serde_json::Value| {
        o.insert(format!("# {k}"), h);
        o.insert(k.into(), v);
    };
    put("save", hint("куда снимок: both — в папку и в буфер, folder — только в папку, clipboard — только в буфер", "where a shot goes: both — folder and clipboard, folder — folder only, clipboard — clipboard only"), S("both".into()));
    put("screenshotPath", hint("путь файла: шаблон strftime, как screenshot-path у niri (%d %m %y %Y %H %M %S); ~ — домашняя папка", "file path: a strftime template like niri's screenshot-path (%d %m %y %Y %H %M %S); ~ is home"), S(DEFAULT_PATH.into()));
    put("shotConfirm", hint("enter — выделил, потом Enter; instant — отпустил мышь, и снимок готов", "enter — select, then press Enter; instant — the shot is taken when you release the mouse"), S("enter".into()));
    put("shotLoupe", hint("лупа у курсора (пробел переключает и запоминает)", "magnifier next to the cursor (space toggles and remembers it)"), Bool(true));
    put("shotNotify", hint("уведомление после снимка (notify-send)", "notification after a shot (notify-send)"), Bool(true));
    put("lang", hint("язык: пусто — как в системе, иначе ru, en, pt-BR", "language: empty — system, or ru, en, pt-BR"), S(String::new()));
    put("accent", hint("цвета берутся из ~/.config/gtk-4.0/gtk.css; accent (#rrggbb) и light — только если там палитры нет", "colours come from ~/.config/gtk-4.0/gtk.css; accent (#rrggbb) and light are used only when it has no palette"), S(String::new()));
    put("light", hint("светлая запасная тема (true/false)", "light fallback theme (true/false)"), Bool(false));
    put("recentColors", hint("недавние цвета меню «другие цвета» — заполняются сами", "recent colours of the «more colours» menu — filled in automatically"), Array(vec![]));
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = ysnap_px::paths::write_atomic(&p, &(serde_json::to_string_pretty(&serde_json::Value::Object(o)).unwrap_or_default() + "\n"));
}

/// дописать в файл save и screenshotPath, если их там нет (битый JSON save не трогает)
pub fn ensure(v: &serde_json::Value, set: &Settings) {
    if v.get("save").and_then(|x| x.as_str()).and_then(Save::from_id).is_none() {
        save("save", serde_json::Value::String(set.save.id().into()));
    }
    if v.get("screenshotPath").and_then(|x| x.as_str()).is_none() {
        save("screenshotPath", serde_json::Value::String(set.path.clone()));
    }
}

/// время по шаблону strftime (libc: те же коды, что у niri)
pub fn strftime(fmt: &str) -> String {
    let Ok(f) = std::ffi::CString::new(fmt) else { return fmt.to_string() };
    let mut buf = vec![0u8; 4096];
    // SAFETY: localtime_r в свой tm, strftime пишет не больше buf.len() байт
    let n = unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        libc::strftime(buf.as_mut_ptr() as *mut libc::c_char, buf.len(), f.as_ptr(), &tm)
    };
    if n == 0 && !fmt.is_empty() {
        return fmt.to_string();
    }
    String::from_utf8_lossy(&buf[..n]).to_string()
}

/// записать один ключ, сохранив остальные (файл мог поправить человек). Файл есть, но это не
/// объект JSON (опечатка, запятая в конце) — не трогаем: иначе он сотрётся до одного ключа
pub fn save(key: &str, v: serde_json::Value) {
    let p = path();
    let mut all = match std::fs::read_to_string(&p) {
        Ok(t) => match serde_json::from_str::<serde_json::Value>(&t) {
            Ok(v) if v.is_object() => v,
            _ => {
                eprintln!("ysnap: {} is not valid JSON, not saving {key}", p.display());
                return;
            }
        },
        Err(_) => serde_json::Value::Object(Default::default()),
    };
    if let Some(o) = all.as_object_mut() {
        o.insert(key.to_string(), v);
    }
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Err(e) = ysnap_px::paths::write_atomic(&p, &(serde_json::to_string_pretty(&all).unwrap_or_default() + "\n")) {
        eprintln!("ysnap: {}: {e}", p.display());
    }
}

impl Settings {
    pub fn from_json(v: &serde_json::Value) -> Settings {
        let b = |k: &str, d: bool| v.get(k).and_then(|x| x.as_bool()).unwrap_or(d);
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let recent = v.get("recentColors").and_then(|x| x.as_array()).into_iter().flatten().filter_map(|c| crate::theme::parse_hex(c.as_str()?)).map(C::hex).take(RECENT).collect();
        // старые ключи shotCopy / shotSave — если save ещё не задан
        let old = match (b("shotCopy", true), b("shotSave", true)) {
            (true, false) => Save::Clipboard,
            (false, _) => Save::Folder,
            _ => Save::Both,
        };
        let p = s("screenshotPath");
        Settings {
            save: Save::from_id(&s("save")).unwrap_or(old),
            path: if p.trim().is_empty() { DEFAULT_PATH.into() } else { p },
            shot_loupe: b("shotLoupe", true),
            shot_notify: b("shotNotify", true),
            shot_confirm: if s("shotConfirm") == "instant" { "instant".into() } else { "enter".into() },
            recent,
        }
    }

    /// цвет — в начало недавних (без повторов) и в файл
    pub fn remember(&mut self, c: C) {
        let c = C::hex(c.0 & 0xffffff);
        self.recent.retain(|x| x.0 & 0xffffff != c.0 & 0xffffff);
        self.recent.insert(0, c);
        self.recent.truncate(RECENT);
        save("recentColors", self.recent.iter().map(|c| serde_json::Value::String(format!("#{:06x}", c.0 & 0xffffff))).collect());
    }

    /// папка и имя файла для снимка сейчас; файл уже есть — « (2)», « (3)»…
    pub fn target(&self) -> (std::path::PathBuf, String) {
        let full = ysnap_px::paths::expand(&strftime(&self.path));
        let dir = full.parent().map(|d| d.to_path_buf()).unwrap_or_else(ysnap_px::paths::pictures);
        let name = full.file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| strftime("%Y-%m-%d %H-%M-%S.png"));
        let (stem, ext) = match name.rsplit_once('.') {
            Some((a, b)) if !a.is_empty() => (a.to_string(), format!(".{b}")),
            _ => (name.clone(), String::new()),
        };
        let mut pick = name;
        for i in 2..1000 {
            if !dir.join(&pick).exists() {
                break;
            }
            pick = format!("{stem} ({i}){ext}");
        }
        (dir, pick)
    }

    pub fn toggle_loupe(&mut self) {
        self.shot_loupe = !self.shot_loupe;
        save("shotLoupe", serde_json::Value::Bool(self.shot_loupe));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_modes_and_old_keys() {
        let set = |j: &str| Settings::from_json(&serde_json::from_str(j).unwrap());
        assert_eq!(set("{}").save, Save::Both);
        assert_eq!(set(r#"{"save":"clipboard"}"#).save, Save::Clipboard);
        assert_eq!(set(r#"{"shotCopy":true,"shotSave":false}"#).save, Save::Clipboard);
        assert_eq!(set(r#"{"shotCopy":false}"#).save, Save::Folder);
        assert!(Save::Both.copy() && Save::Both.file() && !Save::Folder.copy() && !Save::Clipboard.file());
        assert_eq!(set("{}").path, DEFAULT_PATH);
    }

    #[test]
    fn path_template_and_unique_names() {
        assert_eq!(strftime("100%%"), "100%");
        assert_eq!(strftime("%Y").len(), 4);
        let dir = std::env::temp_dir().join(format!("ysnap-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut set = Settings::from_json(&serde_json::Value::Null);
        set.path = format!("{}/shot.png", dir.display());
        assert_eq!(set.target(), (dir.clone(), "shot.png".into()));
        std::fs::write(dir.join("shot.png"), b"").unwrap();
        assert_eq!(set.target().1, "shot (2).png");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
