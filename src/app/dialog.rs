#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialog {
    Open,
    Goto,
    Find,
    Keys,
    About,
}
