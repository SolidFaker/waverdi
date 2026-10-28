/// Horizontal pane proportions, adjustable by dragging the pane borders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Splits {
    pub tree_pct: u16,
    pub list_pct: u16,
    /// Height percentage of the instance/source row above nWave.
    pub top_pct: u16,
    /// Width percentage of the Value column inside the Signal List.
    pub value_pct: u16,
    /// Width percentage of the Hierarchy column inside the Instance pane.
    pub hier_pct: u16,
}

impl Default for Splits {
    fn default() -> Self {
        Self {
            tree_pct: 24,
            list_pct: 31,
            top_pct: 60,
            value_pct: 25,
            hier_pct: 55,
        }
    }
}

impl Splits {
    pub const MIN_PCT: u16 = 8;
    pub const MAX_PCT: u16 = 60;
    /// Minimum width left for the waveform pane, in percent.
    pub const MIN_WAVE_PCT: u16 = 10;
    /// Bounds of the horizontal split between the top row and nWave.
    pub const MIN_TOP_PCT: u16 = 20;
    pub const MAX_TOP_PCT: u16 = 80;
    /// Bounds of the Value column inside the Signal List.
    pub const MIN_VALUE_PCT: u16 = 8;
    pub const MAX_VALUE_PCT: u16 = 50;
    /// Bounds of the Hierarchy column inside the Instance pane.
    pub const MIN_HIER_PCT: u16 = 15;
    pub const MAX_HIER_PCT: u16 = 85;

    pub fn max_tree_pct(&self) -> u16 {
        (100 - self.list_pct - Self::MIN_WAVE_PCT).min(Self::MAX_PCT)
    }

    pub fn max_list_pct(&self) -> u16 {
        (100 - self.tree_pct - Self::MIN_WAVE_PCT).min(Self::MAX_PCT)
    }
}
