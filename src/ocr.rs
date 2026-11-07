// Текст со снимка через tesseract. Снимок экрана — не то, на чём tesseract учился (тёмный текст на
// светлой бумаге, ~300 dpi): мелкий светлый текст на тёмном, сглаженный, на шуме и градиентах. Поэтому
// картинка сначала готовится: серая, тёмная тема переворачивается, увеличение ×3 (пиксельный шрифт
// без сглаживания — клетками), растяжка контраста, белые поля. Потом три прогона — eng+rus, eng и
// rus — параллельно, и по каждой строке выбирается её язык (чей прогон на ней увереннее): слово
// чужого алфавита, похожее на ошибку («еггог» в английской строке, латинская «B» в русской), берётся
// из прогона языка строки, если там чистое слово. Пути и имена со слешами меняются только по
// кускам и только при низкой уверенности. Пробелы между словами — по расстоянию (выравнивание
// кода), отступы строк — для моноширинного текста. Все файлы — в личной папке 0700, после — удаляются.
// i18n: inline pairs

use image::{GrayImage, Luma};
use std::path::Path;
use std::process::{Command, Stdio};

/// слово из TSV tesseract: строка (блок, абзац, строка), рамка, уверенность, текст
#[derive(Clone, Debug)]
struct Word {
    line: (u32, u32, u32),
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    conf: f32,
    text: String,
}

/// больше — tesseract считает долго, а мелкий текст и так крупный
const MAX_PX: u64 = 12_000_000;

/// подготовка: серое, тёмный текст на светлом, увеличение в up раз (если влезает в MAX_PX),
/// контрастнее, с белыми полями
fn prepare(img: &image::DynamicImage, up: u32) -> GrayImage {
    let g = img.to_luma8();
    let (w, h) = g.dimensions();
    let mut hist = [0u64; 256];
    for p in g.pixels() {
        hist[p[0] as usize] += 1;
    }
    let n = (w as u64 * h as u64).max(1);
    let at = |q: f64| {
        let mut acc = 0u64;
        for (i, c) in hist.iter().enumerate() {
            acc += c;
            if acc as f64 >= q * n as f64 {
                return i as i32;
            }
        }
        255
    };
    let (lo, med, hi) = (at(0.02), at(0.5), at(0.98));
    // пиксельный шрифт без сглаживания: почти нет полутонов между фоном и буквами — увеличивать клетками
    let span = (hi - lo).max(1) as f64;
    let (mut mid, mut ink) = (0u64, 0u64);
    for (i, c) in hist.iter().enumerate() {
        let d = (i as i32 - med).abs() as f64;
        if d > span * 0.2 {
            ink += c;
        }
        let f = (i as i32 - lo) as f64 / span;
        if f > 0.2 && f < 0.8 {
            mid += c;
        }
    }
    let hard = ink > 0 && (mid as f64) < ink as f64 * 0.12;
    let mut g = g;
    if med < 128 {
        image::imageops::invert(&mut g);
    }
    let s = (up as u64).min(((MAX_PX as f64 / n as f64).sqrt()) as u64).max(1) as u32;
    // резкий шрифт: клетками и чуть размыть (ступени путают распознавание); остальное — гладко
    let mut g = match (s > 1, hard) {
        (false, _) => g,
        (true, true) => image::imageops::blur(&image::imageops::resize(&g, w * s, h * s, image::imageops::FilterType::Nearest), 1.5),
        (true, false) => image::imageops::resize(&g, w * s, h * s, image::imageops::FilterType::CatmullRom),
    };
    // контраст: 1-й и 99-й процентиль — в чёрное и белое
    let mut hist = [0u64; 256];
    for p in g.pixels() {
        hist[p[0] as usize] += 1;
    }
    let n2 = g.pixels().len() as f64;
    let pct = |q: f64| {
        let mut acc = 0u64;
        for (i, c) in hist.iter().enumerate() {
            acc += c;
            if acc as f64 >= q * n2 {
                return i as f32;
            }
        }
        255.0
    };
    let (a, b) = (pct(0.01), pct(0.99));
    if b - a > 8.0 {
        for p in g.pixels_mut() {
            p[0] = (((p[0] as f32 - a) * 255.0 / (b - a)).clamp(0.0, 255.0)) as u8;
        }
    }
    let pad = 24;
    let mut out = GrayImage::from_pixel(g.width() + 2 * pad, g.height() + 2 * pad, Luma([255]));
    image::imageops::overlay(&mut out, &g, pad as i64, pad as i64);
    out
}

/// один прогон tesseract: слова с рамками и уверенностью (TSV). psm: 3 — сам находит колонки и
/// блоки (интерфейс), 6 — один ровный блок (код, терминал: не теряет строки из одной «}»). Прогонов
/// несколько сразу — каждому один поток (OMP_THREAD_LIMIT), иначе они толкаются
fn run(png: &Path, out: &Path, lang: &str, psm: &str) -> Vec<Word> {
    let ok = Command::new("tesseract")
        .arg(png)
        .arg(out)
        .args(["--dpi", "300", "--psm", psm, "-l", lang, "-c", "preserve_interword_spaces=1", "tsv"])
        .env("OMP_THREAD_LIMIT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return vec![];
    }
    let tsv = std::fs::read_to_string(out.with_extension("tsv")).unwrap_or_default();
    parse_tsv(&tsv)
}

fn parse_tsv(tsv: &str) -> Vec<Word> {
    tsv.lines()
        .skip(1)
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() < 12 || c[0] != "5" || c[11].trim().is_empty() {
                return None;
            }
            let n = |i: usize| c[i].parse::<i32>().ok();
            Some(Word { line: (n(2)? as u32, n(3)? as u32, n(4)? as u32), x: n(6)?, y: n(7)?, w: n(8)?, h: n(9)?, conf: c[10].parse::<f32>().ok()?.max(0.0), text: c[11].trim().to_string() })
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Script {
    Lat,
    Cyr,
}

fn script(ch: char) -> Option<Script> {
    let l = ch.to_lowercase().next().unwrap_or(ch);
    if ('а'..='я').contains(&l) || l == 'ё' {
        Some(Script::Cyr)
    } else if ch.is_ascii_alphabetic() {
        Some(Script::Lat)
    } else {
        None
    }
}

fn count(s: &str, k: Script) -> usize {
    s.chars().filter(|c| script(*c) == Some(k)).count()
}

/// слово целиком из этого алфавита (знаки препинания по краям — можно)
fn clean(s: &str, k: Script) -> bool {
    let core = s.trim_matches(|c: char| ".,:;!?()[]{}«»\"'`“”‘’™®".contains(c));
    !core.is_empty() && core.chars().all(|c| script(c) == Some(k))
}

/// буквы, что tesseract путает между алфавитами (а/a, е/e, г/r, п/n, В/B, 3/З…)
fn ambiguous(s: &str, k: Script) -> bool {
    const LAT: &str = "abcehkmnoprtuxywgBCEHKMOPTXA346";
    const CYR: &str = "авсеЕкКмМнНоОрРсСтТуУхХгпиАВдбьшзчЗ";  // i18n-ok: распознаваемый текст, не надпись
    let set = if k == Script::Lat { LAT } else { CYR };
    let core: Vec<char> = s.chars().filter(|c| c.is_alphanumeric()).collect();
    !core.is_empty() && core.iter().all(|c| set.contains(*c))
}

fn iou(a: &Word, b: &Word) -> f32 {
    let (x0, y0) = (a.x.max(b.x), a.y.max(b.y));
    let (x1, y1) = ((a.x + a.w).min(b.x + b.w), (a.y + a.h).min(b.y + b.h));
    let i = ((x1 - x0).max(0) * (y1 - y0).max(0)) as f32;
    let u = (a.w * a.h + b.w * b.h) as f32 - i;
    if u > 0.0 {
        i / u
    } else {
        0.0
    }
}

const SEPS: &[char] = &['/', '-', '_', '.'];

/// куски слова и разделители между ними: «Изображения/Screenshots» → [Изображения, /, Screenshots]
fn pieces(s: &str) -> Vec<String> {
    let mut v = vec![String::new()];
    for c in s.chars() {
        if SEPS.contains(&c) {
            v.push(c.to_string());
            v.push(String::new());
        } else if let Some(l) = v.last_mut() {
            l.push(c);
        }
    }
    v
}

/// слово общего прогона → его вариант для строки языка dom (alt — то же место прогоном этого языка)
fn pick(base: &Word, alt: Option<&Word>, dom: Script) -> String {
    let other = if dom == Script::Lat { Script::Cyr } else { Script::Lat };
    let bp = pieces(&base.text);
    let ap = alt.map(|a| pieces(&a.text)).filter(|a| a.len() == bp.len());
    let whole = bp.len() == 1;
    let mut out = String::new();
    for (i, p) in bp.iter().enumerate() {
        if p.len() == 1 && p.starts_with(SEPS) {
            out.push_str(p);
            continue;
        }
        let a = ap.as_ref().map(|a| a[i].as_str());
        let short = whole && p.chars().filter(|c| c.is_alphabetic()).count() <= 2;
        // неуверенное слово чужого алфавита — или уверенное, но из одних двойников («еггог», «B»):
        // тогда и вариант языка строки должен быть уверенным (настоящее русское слово в английской
        // строке eng-прогон читает мусором с низкой уверенностью — оно останется)
        let alt_sure = alt.is_some_and(|a| a.conf >= 50.0);
        let wrong = count(p, other) > 0 && count(p, dom) == 0 && (base.conf < 70.0 || (whole && alt_sure && (ambiguous(p, other) || short)));
        let (pl, al) = (p.chars().count().max(1) as f32, a.map(|a| a.chars().count()).unwrap_or(0) as f32);
        let fits = base.conf < 40.0 || (0.6..=1.6).contains(&(al / pl));
        match a {
            Some(a) if wrong && fits && clean(a, dom) => out.push_str(a),
            _ => out.push_str(p),
        }
    }
    out
}

/// строки текста из слов. block — снимок прочитан ровным блоком (код, терминал): тогда у
/// моноширинного текста пробелы — по расстоянию между словами и отступы строк от левого края блока.
/// Интерфейс с колонками — просто слова через пробел (выравнивание колонок в буфере не нужно)
fn layout(lines: &[Vec<(Word, String)>], block: bool) -> String {
    // ширина знака: медиана по словам; моноширинный — если она у всех слов почти одна
    let mut cw: Vec<f32> = lines.iter().flatten().filter(|(w, t)| t.chars().count() >= 2 && w.w > 0).map(|(w, t)| w.w as f32 / t.chars().count() as f32).collect();
    cw.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let Some(&c) = cw.get(cw.len() / 2) else {
        return lines.iter().map(|l| l.iter().map(|w| w.1.as_str()).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n");
    };
    let spread = cw.iter().map(|x| (x - c).abs()).sum::<f32>() / cw.len() as f32 / c.max(1.0);
    let mono = block && cw.len() >= 3 && spread < 0.2;
    // левый край — у каждого блока свой
    let left = |b: u32| lines.iter().filter(|l| l.first().is_some_and(|w| w.0.line.0 == b)).filter_map(|l| l.first().map(|w| w.0.x)).min().unwrap_or(0);
    // пустая строка — там, где на снимке между строками заметный промежуток (абзац, другой блок)
    let mut hs: Vec<i32> = lines.iter().flatten().map(|w| w.0.h).collect();
    hs.sort_unstable();
    let lh = hs.get(hs.len() / 2).copied().unwrap_or(0).max(1);
    let mut out = String::new();
    let mut prev_bottom: Option<i32> = None;
    for l in lines {
        let Some(first) = l.first() else { continue };
        let top = l.iter().map(|w| w.0.y).min().unwrap_or(0);
        if prev_bottom.is_some_and(|b| top - b > lh * 3 / 2) {
            out.push('\n');
        }
        prev_bottom = Some(l.iter().map(|w| w.0.y + w.0.h).max().unwrap_or(top));
        if mono {
            let indent = ((first.0.x - left(first.0.line.0)) as f32 / c).round() as usize;
            out.push_str(&" ".repeat(indent.min(80)));
        }
        for (i, (w, t)) in l.iter().enumerate() {
            if i > 0 {
                let gap = (w.x - (l[i - 1].0.x + l[i - 1].0.w)) as f32;
                let n = if mono && gap > 1.5 * c { (gap / c).round() as usize } else { 1 };
                out.push_str(&" ".repeat(n.clamp(1, 40)));
            }
            out.push_str(t);
        }
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// языки tesseract, что стоят: из eng и rus
fn langs() -> Vec<&'static str> {
    let l = Command::new("tesseract").arg("--list-langs").stderr(Stdio::null()).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    ["eng", "rus"].into_iter().filter(|x| l.lines().any(|y| y.trim() == *x)).collect()
}

/// распознать картинку (PNG, JPEG, WebP — что угодно, что читает image); None — текста нет или
/// tesseract не справился. Работает долго (доли секунды — секунды): только в нити
pub fn recognize(img: &image::DynamicImage) -> Option<String> {
    let dir = ysnap_px::paths::private_tmp("ysnap-ocr").ok()?;
    let (words, block) = read(img, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    let mut lines: Vec<Vec<(Word, String)>> = vec![];
    for (w, t) in words {
        match lines.last_mut() {
            Some(l) if l.first().is_some_and(|f| f.0.line == w.line) => l.push((w, t)),
            _ => lines.push(vec![(w, t)]),
        }
    }
    let text = layout(&lines, block);
    (!text.trim().is_empty()).then_some(text)
}

/// сколько букв и цифр прочитано уверенно: мера, какой вариант подготовки удачнее (длинное слово
/// не перевешивает все остальные)
fn score(ws: &[Word]) -> usize {
    ws.iter().filter(|w| w.conf >= 70.0).map(|w| w.text.chars().filter(|c| c.is_alphanumeric()).count().min(12)).sum()
}

/// psm 3 нашёл колонки: блоки рядом по горизонтали (по высоте перекрываются, по ширине — нет)
fn columns(ws: &[Word]) -> bool {
    let mut bs: Vec<(u32, i32, i32, i32, i32)> = vec![];
    for w in ws {
        match bs.iter_mut().find(|b| b.0 == w.line.0) {
            Some(b) => {
                b.1 = b.1.min(w.x);
                b.2 = b.2.min(w.y);
                b.3 = b.3.max(w.x + w.w);
                b.4 = b.4.max(w.y + w.h);
            }
            None => bs.push((w.line.0, w.x, w.y, w.x + w.w, w.y + w.h)),
        }
    }
    bs.iter().enumerate().any(|(i, a)| {
        bs[i + 1..].iter().any(|b| {
            let oy = (a.4.min(b.4) - a.2.max(b.2)) as f32;
            let side = a.3 <= b.1 || b.3 <= a.1;
            side && oy > 0.5 * (a.4 - a.2).min(b.4 - b.2) as f32
        })
    })
}

/// слова со снимка. Как готовить снимок, заранее не угадать: пиксельный шрифт шелла лучше читается в
/// родном размере, мелкий сглаженный текст — увеличенным; колонки интерфейса — разметкой psm 3, код —
/// psm 6. Четыре варианта — параллельно, берётся тот, где уверенно прочитано больше; на нём —
/// прогоны отдельно eng и rus и выбор слов по языку строки (merge)
fn read(img: &image::DynamicImage, dir: &Path) -> (Vec<(Word, String)>, bool) {
    let have = langs();
    let both = if have.len() >= 2 { "eng+rus".to_string() } else { have.first().copied().unwrap_or("eng").to_string() };
    // увеличение нужно мелкому тексту в небольшом выделении; на весь экран оно стоит секунд, а
    // текст там обычного размера
    let n = img.width() as u64 * img.height() as u64;
    let ups: &[u32] = if n > 1_000_000 {
        &[1]
    } else if n > 400_000 {
        &[1, 2]
    } else {
        &[1, 3]
    };
    let pngs: Vec<(u32, std::path::PathBuf)> = ups
        .iter()
        .copied()
        .filter_map(|up| {
            let p = dir.join(format!("in{up}.png"));
            prepare(img, up).save(&p).ok().map(|_| (up, p))
        })
        .collect();
    let mut tries: Vec<(usize, u8, u32, &Path, &str, Vec<Word>)> = std::thread::scope(|s| {
        let hs: Vec<_> = pngs
            .iter()
            .flat_map(|(up, p)| ["3", "6"].into_iter().map(move |psm| (*up, p, psm)))
            .map(|(up, p, psm)| {
                let both = &both;
                s.spawn(move || {
                    let ws = run(p, &dir.join(format!("b{up}-{psm}")), both, psm);
                    // при равенстве — psm 6 (не теряет строк из одной «}»), но не там, где psm 3
                    // нашёл колонки (иначе строки двух колонок склеятся); и увеличенный
                    let prefer = if psm == "6" { 1 } else if columns(&ws) { 2 } else { 0 };
                    (score(&ws), prefer, up, p.as_path(), psm, ws)
                })
            })
            .collect();
        hs.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    tries.sort_by_key(|t| (t.0, t.1, t.2));
    let Some((_, _, _, png, psm, base)) = tries.pop() else { return (vec![], false) };
    let block = psm == "6";
    if have.len() < 2 {
        return (base.into_iter().map(|w| (w.clone(), w.text)).collect(), block);
    }
    let (eng, rus) = std::thread::scope(|s| {
        let e = s.spawn(|| run(png, &dir.join("eng"), "eng", psm));
        let r = run(png, &dir.join("rus"), "rus", psm);
        (e.join().unwrap_or_default(), r)
    });
    (merge(&base, &eng, &rus), block)
}

/// по строкам общего прогона: язык строки — чей прогон увереннее на её словах; слова, похожие на
/// ошибку чужого алфавита, — из прогона этого языка
fn merge(base: &[Word], eng: &[Word], rus: &[Word]) -> Vec<(Word, String)> {
    let near = |w: &Word, r: &'_ [Word]| r.iter().filter(|v| iou(w, v) > 0.4).cloned().collect::<Vec<Word>>();
    let mut out = vec![];
    let mut i = 0;
    while i < base.len() {
        let key = base[i].line;
        let j = base[i..].iter().position(|w| w.line != key).map(|p| i + p).unwrap_or(base.len());
        let line = &base[i..j];
        // сколько букв прогон прочёл уверенно (уверенность между языками несравнима: «еггог» по-русски
        // бывает увереннее «error» по-английски; зато мусор чужого языка уверенным почти не бывает)
        let score = |r: &[Word]| -> usize {
            let mut m: Vec<Word> = line.iter().flat_map(|w| near(w, r)).collect();
            m.sort_by_key(|v| (v.x, v.y));
            m.dedup_by_key(|v| (v.x, v.y));
            m.iter().filter(|v| v.conf >= 70.0).map(|v| v.text.chars().filter(|c| c.is_alphanumeric()).count().min(12)).sum()
        };
        let (se, sr) = (score(eng), score(rus));
        let dom = if se > sr + 2 {
            Some(Script::Lat)
        } else if sr > se + 2 {
            Some(Script::Cyr)
        } else {
            None
        };
        for w in line {
            let t = match dom {
                Some(d) => {
                    let alt = near(w, if d == Script::Cyr { rus } else { eng }).into_iter().next();
                    pick(w, alt.as_ref(), d)
                }
                None => w.text.clone(),
            };
            out.push((w.clone(), t));
        }
        i = j;
    }
    out
}

/// `ysnap ocr FILE`: текст с картинки в stdout
pub fn cli(args: &[String]) -> i32 {
    let Some(f) = args.first().filter(|a| !a.starts_with('-')) else {
        eprintln!(" ✕ ysnap ocr FILE");
        return 2;
    };
    if !crate::which("tesseract") {
        eprintln!(" ✕ {}", crate::tf!("нужен tesseract", "tesseract is needed", "tesseract é necessário"));
        return 1;
    }
    let Ok(img) = image::open(f) else {
        eprintln!(" ✕ {}", crate::tf!("не читается: {}", "cannot read: {}", "não foi possível ler: {}", f));
        return 2;
    };
    match recognize(&img) {
        Some(t) => {
            println!("{t}");
            0
        }
        None => {
            eprintln!(" · {}", crate::tf!("текст не распознан", "no text recognised", "nenhum texto reconhecido"));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(line: u32, x: i32, conf: f32, t: &str) -> Word {
        Word { line: (1, 1, line), x, y: line as i32 * 40, w: t.chars().count() as i32 * 20, h: 30, conf, text: t.into() }
    }

    #[test]
    fn line_language_fixes_homoglyphs() {
        // английская строка: «еггог» — кириллица из одних двойников, eng-прогон читает error
        let base = vec![w(1, 0, 89.0, "еггог"), w(1, 120, 90.0, "cannot"), w(1, 260, 91.0, "find")]; // i18n-ok
        let eng = vec![w(1, 0, 66.0, "error"), w(1, 120, 90.0, "cannot"), w(1, 260, 91.0, "find")];
        let rus = vec![w(1, 0, 89.0, "еггог"), w(1, 120, 40.0, "саппо*"), w(1, 260, 8.0, "Ф1па")]; // i18n-ok
        let m = merge(&base, &eng, &rus);
        assert_eq!(m.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(), ["error", "cannot", "find"]);
        // русская строка: латинская «B» и путь с низкой уверенностью — по кускам
        let base = vec![w(1, 0, 92.0, "сохранён"), w(1, 200, 93.0, "B"), w(1, 240, 22.0, "M3obpaxeHus/Screenshots")]; // i18n-ok
        let eng = vec![w(1, 0, 64.0, "COXpaHéH"), w(1, 200, 93.0, "B"), w(1, 240, 22.0, "M3obpaxeHus/Screenshots")];
        let rus = vec![w(1, 0, 92.0, "сохранён"), w(1, 200, 93.0, "в"), w(1, 240, 4.0, "Изображения/5сгееп5По{5")]; // i18n-ok
        let m = merge(&base, &eng, &rus);
        assert_eq!(m.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(), ["сохранён", "в", "Изображения/Screenshots"]); // i18n-ok
        // уверенный путь в русской строке не трогается
        let base = vec![w(1, 0, 92.0, "путь"), w(1, 120, 90.0, "/home/user")]; // i18n-ok
        let rus = vec![w(1, 0, 92.0, "путь"), w(1, 120, 60.0, "/поте/изег")]; // i18n-ok
        let eng = vec![w(1, 0, 20.0, "nyTb"), w(1, 120, 90.0, "/home/user")];
        assert_eq!(merge(&base, &eng, &rus)[1].1, "/home/user");
    }

    #[test]
    fn tsv_and_layout() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   5\t1\t1\t1\t1\t1\t10\t10\t40\t20\t95.5\tfn\n\
                   5\t1\t1\t1\t1\t2\t70\t10\t80\t20\t91\tmain()\n\
                   4\t1\t1\t1\t2\t0\t10\t40\t100\t20\t-1\t\n\
                   5\t1\t1\t1\t2\t1\t50\t40\t60\t20\t90\tlet\n";
        let ws = parse_tsv(tsv);
        assert_eq!(ws.len(), 3);
        assert_eq!(ws[1].text, "main()");
        assert!(pieces("a/b-c").len() == 5 && ambiguous("еггог", Script::Cyr) && ambiguous("сагдо", Script::Cyr) && !ambiguous("files", Script::Lat)); // i18n-ok
        // настоящее русское слово в английской строке: eng читает его неуверенно — остаётся
        let base = vec![w(1, 0, 90.0, "say"), w(1, 80, 85.0, "привет"), w(1, 220, 90.0, "to"), w(1, 280, 92.0, "everyone")]; // i18n-ok
        let eng = vec![w(1, 0, 90.0, "say"), w(1, 80, 31.0, "npuBeT"), w(1, 220, 90.0, "to"), w(1, 280, 92.0, "everyone")];
        let rus = vec![w(1, 0, 40.0, "зау"), w(1, 80, 85.0, "привет"), w(1, 220, 30.0, "Ю"), w(1, 280, 20.0, "емегуопе")]; // i18n-ok
        assert_eq!(merge(&base, &eng, &rus)[1].1, "привет"); // i18n-ok
    }
}
