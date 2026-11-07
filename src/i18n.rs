// Язык — тот же механизм, что в шелле: надписи по-русски, английский по таблице при отрисовке,
// португальский — по английскому. Строки с подстановками — макросом tf!("…", "…", "…", args).
// Какой язык — решает ysnap_px::i18n (из настроек шелла или системы).

use std::collections::HashMap;
use std::sync::OnceLock;

pub use ysnap_px::i18n::{en, pt_br, set, translated};

/// перевод целой надписи; нет в таблице — как есть
pub fn tr(s: &str) -> &str {
    if !translated() {
        return s;
    }
    let english = table().get(s).copied().unwrap_or(s);
    if pt_br() {
        pt_table().get(english).copied().unwrap_or(english)
    } else {
        english
    }
}

#[macro_export]
macro_rules! tf {
    ($ru:literal, $en:literal, $pt:literal $(, $a:expr)* $(,)?) => {
        if $crate::i18n::pt_br() {
            format!($pt $(, $a)*)
        } else if $crate::i18n::en() {
            format!($en $(, $a)*)
        } else {
            format!($ru $(, $a)*)
        }
    };
}

fn table() -> &'static HashMap<&'static str, &'static str> {
    static T: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    T.get_or_init(|| PAIRS.iter().copied().collect())
}

fn pt_table() -> &'static HashMap<&'static str, &'static str> {
    static T: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    T.get_or_init(|| PT_BR.iter().copied().collect())
}

const PAIRS: &[(&str, &str)] = &[
    ("QR-код", "QR code"),
    ("QR-код в буфер", "QR code to clipboard"),
    ("QR-код не найден", "no QR code found"),
    ("в буфер · ctrl+c", "copy · ctrl+c"),
    ("в папке", "show in folder"),
    ("выделение", "select"),
    ("выделение · 1", "select · 1"),
    ("маркер", "marker"),
    ("маркер · 5", "marker · 5"),
    ("мозаика", "mosaic"),
    ("мозаика · 6", "mosaic · 6"),
    ("номерок", "counter"),
    ("номерок · 7", "counter · 7"),
    ("нужен grim (пакет grim)", "grim is needed (package grim)"),
    ("область", "area"),
    ("окно", "window"),
    ("открыть", "open"),
    ("отмена · esc", "cancel · esc"),
    ("отменить · ctrl+z", "undo · ctrl+z"),
    ("перо", "pen"),
    ("перо · 2", "pen · 2"),
    ("пипетка", "colour picker"),
    ("пипетка: цвет в буфер · 8", "colour picker: hex to clipboard · 8"),
    ("рамка", "frame"),
    ("рамка · 3", "frame · 3"),
    ("скриншот в буфере обмена", "screenshot copied"),
    ("скриншот не вышел", "screenshot failed"),
    ("скриншот не сохранился", "screenshot not saved"),
    ("скриншот сохранён", "screenshot saved"),
    ("скриншот сохранён и скопирован", "screenshot saved and copied"),
    ("сохранить · enter", "save · enter"),
    ("сохранить как… · ctrl+s", "save as… · ctrl+s"),
    ("стрелка", "arrow"),
    ("стрелка · 4", "arrow · 4"),
    ("текст в буфер (tesseract)", "text to clipboard (tesseract)"),
    ("текст не распознан", "no text recognised"),
    ("текст со снимка", "text from the screenshot"),
    ("толщина · колесо", "thickness · wheel"),
    ("тяни — выделить · enter — весь экран · esc — отмена", "drag to select · enter for the whole screen · esc to cancel"),
    ("щёлкни или enter — весь монитор · esc — отмена", "click or enter for the whole monitor · esc to cancel"),
    ("щёлкни по окну · enter — окно под курсором · esc — отмена", "click a window · enter for the one under the cursor · esc to cancel"),
    ("экран", "screen"),
    ("другие цвета", "more colours"),
    ("недавние", "recent"),
    ("пока пусто", "nothing yet"),
];

#[rustfmt::skip]
const PT_BR: &[(&str, &str)] = &[
    ("QR code", "Código QR"),
    ("QR code to clipboard", "Código QR para a área de transferência"),
    ("no QR code found", "nenhum código QR encontrado"),
    ("copy · ctrl+c", "copiar · ctrl+c"),
    ("show in folder", "mostrar na pasta"),
    ("select", "selecionar"),
    ("select · 1", "selecionar · 1"),
    ("marker", "marca-texto"),
    ("marker · 5", "marca-texto · 5"),
    ("mosaic", "mosaico"),
    ("mosaic · 6", "mosaico · 6"),
    ("counter", "contador"),
    ("counter · 7", "contador · 7"),
    ("grim is needed (package grim)", "grim é necessário (pacote grim)"),
    ("area", "área"),
    ("window", "janela"),
    ("open", "abrir"),
    ("cancel · esc", "cancelar · esc"),
    ("undo · ctrl+z", "desfazer · ctrl+z"),
    ("pen", "caneta"),
    ("pen · 2", "caneta · 2"),
    ("colour picker", "seletor de cores"),
    ("colour picker: hex to clipboard · 8", "seletor de cores: hexadecimal para a área de transferência · 8"),
    ("frame", "moldura"),
    ("frame · 3", "moldura · 3"),
    ("screenshot copied", "captura de tela copiada"),
    ("screenshot failed", "falha na captura de tela"),
    ("screenshot not saved", "captura de tela não salva"),
    ("screenshot saved", "captura de tela salva"),
    ("screenshot saved and copied", "captura de tela salva e copiada"),
    ("save · enter", "salvar · enter"),
    ("save as… · ctrl+s", "salvar como… · ctrl+s"),
    ("arrow", "seta"),
    ("arrow · 4", "seta · 4"),
    ("text to clipboard (tesseract)", "texto para a área de transferência (tesseract)"),
    ("no text recognised", "nenhum texto reconhecido"),
    ("text from the screenshot", "texto da captura de tela"),
    ("thickness · wheel", "espessura · roda do mouse"),
    ("drag to select · enter for the whole screen · esc to cancel", "arraste para selecionar · enter para a tela inteira · esc para cancelar"),
    ("click or enter for the whole monitor · esc to cancel", "clique ou enter para o monitor inteiro · esc para cancelar"),
    ("click a window · enter for the one under the cursor · esc to cancel", "clique numa janela · enter para a janela sob o cursor · esc para cancelar"),
    ("screen", "tela"),
    ("more colours", "mais cores"),
    ("recent", "recentes"),
    ("nothing yet", "nada ainda"),
];
