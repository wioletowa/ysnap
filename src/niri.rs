// IPC niri напрямую: строка JSON в $NIRI_SOCKET, ответ — строка {"Ok": {"<Запрос>": …}}. Доли
// миллисекунды против ~7 мс на запуск `niri msg`. Нет сокета или ответа — тот же запрос через
// `niri msg -j` (тот же JSON, что внутри «Ok»).

use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

/// запрос без аргументов: "FocusedOutput", "FocusedWindow", "Windows", "Workspaces"
pub fn request(req: &str) -> serde_json::Value {
    socket(req).unwrap_or_else(|| cli(req))
}

fn socket(req: &str) -> Option<serde_json::Value> {
    let path = std::env::var_os("NIRI_SOCKET")?;
    let mut s = std::os::unix::net::UnixStream::connect(path).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    s.write_all(format!("\"{req}\"\n").as_bytes()).ok()?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).ok()?;
    let mut v: serde_json::Value = serde_json::from_str(&line).ok()?;
    Some(v.get_mut("Ok")?.get_mut(req)?.take())
}

/// FocusedOutput → focused-output
fn kebab(req: &str) -> String {
    let mut arg = String::new();
    for (i, c) in req.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            arg.push('-');
        }
        arg.extend(c.to_lowercase());
    }
    arg
}

fn cli(req: &str) -> serde_json::Value {
    std::process::Command::new("niri").args(["msg", "-j", &kebab(req)]).output().ok().and_then(|o| serde_json::from_slice(&o.stdout).ok()).unwrap_or(serde_json::Value::Null)
}

/// всё, что оверлею нужно от niri, — четыре запроса разом: окна видимых рабочих столов (монитор,
/// рамка, заголовок), монитор и окно с фокусом
pub struct Info {
    pub wins: Vec<(String, crate::gfx::R, String)>,
    pub focus: String,
    pub focus_win: Option<u64>,
}

pub fn info() -> Info {
    let (ws, wins, out, win) = std::thread::scope(|sc| {
        let ws = sc.spawn(|| request("Workspaces"));
        let wins = sc.spawn(|| request("Windows"));
        let out = sc.spawn(|| request("FocusedOutput"));
        let win = request("FocusedWindow");
        let j = |h: std::thread::ScopedJoinHandle<'_, serde_json::Value>| h.join().unwrap_or(serde_json::Value::Null);
        (j(ws), j(wins), j(out), win)
    });
    Info { wins: crate::snap::niri_windows(&ws, &wins), focus: focus_name(&out), focus_win: win["id"].as_u64() }
}

pub fn focus_name(v: &serde_json::Value) -> String {
    v["name"].as_str().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn cli_names() {
        assert_eq!(super::kebab("FocusedOutput"), "focused-output");
        assert_eq!(super::kebab("Windows"), "windows");
    }
}
