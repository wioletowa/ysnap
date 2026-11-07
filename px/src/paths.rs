// Каталоги XDG и атомарная запись.

use std::path::{Path, PathBuf};

/// переменная XDG: пустая или относительная — как не заданная (так велит спецификация)
fn xdg(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute())
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}
pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config"))
}
pub fn state_dir() -> PathBuf {
    let d = xdg("XDG_STATE_HOME").unwrap_or_else(|| home().join(".local/state")).join("ysnap");
    let _ = std::fs::create_dir_all(&d);
    d
}
pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local/share"))
}
/// системные папки данных: XDG_DATA_DIRS (на NixOS там /run/current-system/sw/share и
/// share каждого пакета), без неё — /usr/local/share и /usr/share
pub fn data_dirs() -> Vec<PathBuf> {
    let v = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    std::env::split_paths(&v).filter(|p| p.is_absolute()).collect()
}

/// исполняемый файл в PATH
pub fn find_exe(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false))
}

pub fn cache_dir() -> PathBuf {
    xdg("XDG_CACHE_HOME").unwrap_or_else(|| home().join(".cache"))
}
/// папка XDG из ~/.config/user-dirs.dirs (PICTURES, DOWNLOAD…); равная домашней — как не заданная
pub fn xdg_dir(key: &str) -> Option<PathBuf> {
    let t = std::fs::read_to_string(config_dir().join("user-dirs.dirs")).ok()?;
    let home = home();
    for l in t.lines() {
        if let Some(v) = l.strip_prefix(&format!("XDG_{key}_DIR=")) {
            let v = v.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
            let p = PathBuf::from(v);
            if p != home {
                return Some(p);
            }
        }
    }
    None
}

/// «Изображения» (XDG_PICTURES_DIR), иначе ~/Pictures
pub fn pictures() -> PathBuf {
    xdg_dir("PICTURES").unwrap_or_else(|| home().join("Pictures"))
}

/// путь из настроек или поля: «~» и «~/…» — от домашней папки
pub fn expand(p: &str) -> PathBuf {
    let p = p.trim();
    if p == "~" {
        return home();
    }
    p.strip_prefix("~/").map(|r| home().join(r)).unwrap_or_else(|| PathBuf::from(p))
}

/// путь для показа: домашняя папка — «~»
pub fn pretty(p: &Path) -> String {
    let h = home();
    if p == h {
        return "~".into();
    }
    match p.strip_prefix(&h) {
        Ok(r) => format!("~/{}", r.display()),
        Err(_) => p.display().to_string(),
    }
}

/// «естественное» сравнение имён: file2 < file10, без учёта регистра
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    // сортировки папок зовут это тысячи раз: числа — срезами строки (цифры ASCII), без выделений
    let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
    let (mut i, mut j) = (0, 0);
    loop {
        match (a[i..].chars().next(), b[j..].chars().next()) {
            (None, None) => return a.cmp(b),
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (ei, ej) = (i + digits(&a[i..]), j + digits(&b[j..]));
                let (ta, tb) = (a[i..ei].trim_start_matches('0'), b[j..ej].trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then(ta.cmp(tb));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
                (i, j) = (ei, ej);
            }
            (Some(x), Some(y)) => {
                let (lx, ly) = (x.to_lowercase().next().unwrap_or(x), y.to_lowercase().next().unwrap_or(y));
                if lx != ly {
                    return lx.cmp(&ly);
                }
                (i, j) = (i + x.len_utf8(), j + y.len_utf8());
            }
        }
    }
}

/// новый личный каталог для временных файлов: в $XDG_RUNTIME_DIR (он только наш) или в /tmp, с
/// правами 0700 и всегда новый — заготовка другого пользователя в /tmp со ссылками внутри
/// перенаправила бы наши записи в чужие файлы, а снимок экрана в /tmp иначе читали бы все
pub fn private_tmp(name: &str) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let base = xdg("XDG_RUNTIME_DIR").filter(|p| p.is_dir()).unwrap_or_else(std::env::temp_dir);
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let d = base.join(format!("{name}-{}-{n:08x}", std::process::id()));
    std::fs::DirBuilder::new().mode(0o700).create(&d)?;
    Ok(d)
}

/// свой бинарь: после обновления старый файл удалён («… (deleted)») — берём новый по тому же пути
pub fn self_exe() -> PathBuf {
    let Some(p) = std::fs::read_link("/proc/self/exe").ok().or_else(|| std::env::current_exe().ok()) else { return PathBuf::from("ysnap") };
    let s = p.to_string_lossy();
    PathBuf::from(s.strip_suffix(" (deleted)").unwrap_or(&s))
}

/// процессы этого пользователя по имени программы (последняя часть argv[0]) и, если задан, аргументу
/// среди остальных; процессы других пользователей и зомби (cmdline пуст) не попадают
pub fn pids(exe: &str, arg: &str) -> Vec<u32> {
    use std::os::unix::fs::MetadataExt;
    let Ok(rd) = std::fs::read_dir("/proc") else { return vec![] };
    let me = unsafe { libc::getuid() };
    rd.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        // cmdline чужих процессов читается всем: владелец папки /proc/PID — их uid
        .filter(|pid| std::fs::metadata(format!("/proc/{pid}")).is_ok_and(|m| m.uid() == me))
        .filter(|pid| {
            let c = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            let mut parts = c.split(|b| *b == 0);
            let first = parts.next().unwrap_or_default();
            first.rsplit(|b| *b == b'/').next() == Some(exe.as_bytes()) && (arg.is_empty() || parts.any(|p| p == arg.as_bytes()))
        })
        .collect()
}

/// файл, путь к которому дал чужой процесс (уведомление, плеер), читаем с потолком: /dev/zero,
/// fifo или гигантский файл не съедят память и не подвесят нить. Не обычный файл — None.
pub fn read_capped(p: &Path, max: u64) -> Option<Vec<u8>> {
    read_capped_with(p, max, 0)
}

/// то же, но сам файл не может быть ссылкой (O_NOFOLLOW): файлы из папки, которую пишет другой
/// пользователь (вид для экрана входа), — ссылка на чужой файл не прочтётся с нашими правами
pub fn read_capped_nofollow(p: &Path, max: u64) -> Option<Vec<u8>> {
    read_capped_with(p, max, libc::O_NOFOLLOW)
}

fn read_capped_with(p: &Path, max: u64, flags: i32) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let m = if flags & libc::O_NOFOLLOW != 0 { std::fs::symlink_metadata(p) } else { std::fs::metadata(p) };
    if !m.ok()?.is_file() {
        return None;
    }
    // O_NONBLOCK: если файл подменили на fifo между проверкой и открытием, open не повиснет
    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | flags).open(p).ok()?;
    let m = f.metadata().ok()?;
    if !m.is_file() || m.len() > max {
        return None;
    }
    let mut v = Vec::with_capacity(m.len() as usize);
    f.take(max).read_to_end(&mut v).ok()?;
    Some(v)
}

/// уборка кеша (миниатюры, позиции плеера): не чаще раза в сутки (метка `.tidy` в самой папке)
/// удаляет файлы старше `days` дней по времени изменения, а если папка всё равно больше
/// `max_bytes` — самые старые, пока не влезет. Вложенные папки не трогаются. Зовётся из нити
pub fn tidy(dir: &Path, days: u64, max_bytes: u64) {
    use std::time::{Duration, SystemTime};
    let now = SystemTime::now();
    let stamp = dir.join(".tidy");
    let fresh = std::fs::metadata(&stamp).and_then(|m| m.modified()).map(|t| now.duration_since(t).unwrap_or_default() < Duration::from_secs(86_400)).unwrap_or(false);
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    if fresh {
        return;
    }
    let _ = std::fs::write(&stamp, b"");
    let old = now - Duration::from_secs(days * 86_400);
    let mut kept: Vec<(SystemTime, u64, PathBuf)> = vec![];
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() || e.file_name() == ".tidy" {
            continue;
        }
        let t = m.modified().unwrap_or(now);
        if t < old {
            let _ = std::fs::remove_file(e.path());
        } else {
            kept.push((t, m.len(), e.path()));
        }
    }
    let mut total: u64 = kept.iter().map(|k| k.1).sum();
    if total <= max_bytes {
        return;
    }
    kept.sort_by_key(|k| k.0);
    for (_, len, p) in kept {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
}

/// запись через временный файл: читатель никогда не увидит половину
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path).map(|t| t == text).unwrap_or(false) {
        return Ok(());
    }
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension("tmp~");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Разбор JSON-настроек, прощающий ошибки: ключ с негодным значением берётся по умолчанию, а не
/// весь файл (правят руками по ключу — одна опечатка не сбрасывает остальное). Неизвестные
/// ключи пропускаются. Не JSON вовсе — значения по умолчанию.
pub fn lenient_json<T: serde::de::DeserializeOwned + serde::Serialize + Default>(text: &str) -> T {
    if let Ok(v) = serde_json::from_str::<T>(text) {
        return v;
    }
    let Some(user) = serde_json::from_str::<serde_json::Value>(text).ok().and_then(|v| v.as_object().cloned()) else { return T::default() };
    let mut base = serde_json::to_value(T::default()).unwrap_or_default();
    for (k, x) in user {
        let mut one = base.clone();
        one[&k] = x.clone();
        if serde_json::from_value::<T>(one).is_ok() {
            base[&k] = x;
        } else {
            eprintln!("bad value for {k}, default used");
        }
    }
    serde_json::from_value(base).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{natural, private_tmp, tidy};
    use std::cmp::Ordering::*;

    #[test]
    fn natural_order() {
        let mut v = ["ep10.mkv", "Ep2.mkv", "ep1.mkv", "ep02b.mkv", "ёж 3", "ёж 20", "a"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(v, ["a", "ep1.mkv", "Ep2.mkv", "ep02b.mkv", "ep10.mkv", "ёж 3", "ёж 20"]);
        assert_eq!(natural("file007", "file7"), "file007".cmp("file7"));
        assert_eq!(natural("x99999999999999999999999", "x100000000000000000000000"), Less);
        assert_eq!(natural("A", "a"), Less);
    }

    #[test]
    fn tidy_ages_and_caps() {
        use std::time::{Duration, SystemTime};
        let d = private_tmp("tidy-test").unwrap();
        let put = |name: &str, len: usize, age_days: u64| {
            let p = d.join(name);
            std::fs::write(&p, vec![0u8; len]).unwrap();
            let f = std::fs::File::options().write(true).open(&p).unwrap();
            f.set_modified(SystemTime::now() - Duration::from_secs(age_days * 86_400)).unwrap();
        };
        put("old", 10, 200);
        put("mid", 100, 50);
        put("new", 100, 1);
        std::fs::create_dir(d.join("sub")).unwrap();
        tidy(&d, 120, 150);
        // старше 120 дней — прочь; потом самые старые, пока не влезет в 150 байт; папки не трогаются
        assert!(!d.join("old").exists());
        assert!(!d.join("mid").exists());
        assert!(d.join("new").exists() && d.join("sub").is_dir());
        // второй раз за сутки — ничего не делает
        put("again", 10, 300);
        tidy(&d, 120, 150);
        assert!(d.join("again").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
