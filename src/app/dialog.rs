#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialog {
    Open,
    Goto,
    Find,
    FindValue,
    SplitBus,
    CreateBus,
    GroupName,
    Keys,
    About,
}
