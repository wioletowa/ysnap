// Действия оверлея. Сам немедленный UI (кнопки, зоны) — общий, в ysnap-px.

pub use ysnap_px::ui::{Cur, Tone};

pub type Ui<'a, 'b> = ysnap_px::ui::Ui<'a, 'b, Act>;
pub type Zone = ysnap_px::ui::Zone<Act>;

#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// глушит клики (фон)
    Block,
    SnapBtn(&'static str),
}

impl ysnap_px::ui::Action for Act {
    fn block() -> Self {
        Act::Block
    }
}
