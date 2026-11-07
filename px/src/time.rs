// Местное время без chrono (libc localtime_r) и русские названия дней и месяцев: шелл, экран входа.

#[derive(Clone, Copy, Debug)]
pub struct Tm {
    pub year: i32,
    pub mon: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
    pub wday: u32,
}

/// секунды с 1970-го (метки кеша, возраст данных)
pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn local_now() -> Tm {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    local_tm(now)
}
pub fn local_tm(t: i64) -> Tm {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    Tm { year: tm.tm_year + 1900, mon: tm.tm_mon as u32 + 1, day: tm.tm_mday as u32, hour: tm.tm_hour as u32, min: tm.tm_min as u32, sec: tm.tm_sec as u32, wday: tm.tm_wday as u32 }
}

pub const RU_DAY: [&str; 7] = ["вс", "пн", "вт", "ср", "чт", "пт", "сб"];
pub const RU_DAY_FULL: [&str; 7] = ["воскресенье", "понедельник", "вторник", "среда", "четверг", "пятница", "суббота"];
pub const RU_MON: [&str; 12] = ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];
pub const RU_MON_NOM: [&str; 12] = ["январь", "февраль", "март", "апрель", "май", "июнь", "июль", "август", "сентябрь", "октябрь", "ноябрь", "декабрь"];
