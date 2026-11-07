// ysnap-px: всё, из чего состоит пиксельный вид — холст (целые пиксели, фаски, байеровское
// проявление), шрифты, тема из одного акцента, иконки, немедленный UI. Взято из yami
// (https://github.com/Trendorin/yami) и урезано под ysnap.

pub mod font;
pub mod gfx;
pub mod i18n;
pub mod icons;
pub mod look;
pub mod paths;
pub mod theme;
pub mod time;
pub mod ui;

/// FNV-1a: короткий устойчивый хэш для имён в кеше и отпечатков (не криптография); начало — FNV0,
/// продолжение — прошлый h
pub const FNV0: u64 = 0xcbf2_9ce4_8422_2325;
pub fn fnv(mut h: u64, d: &[u8]) -> u64 {
    for b in d {
        h = (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
