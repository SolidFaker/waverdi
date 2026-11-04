#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialog {
    Open,
    Goto,
    Find,
    FindValue,
    Follow,
    Markers,
    SplitBus,
    CreateBus,
    GroupName,
    Settings,
    BindKey,
    ColorPicker,
    Keys,
    About,
    AddSignals,
}
