use super::{App, Dialog};
use crate::waveform::{Change, Radix, SigKind, Signal, Ticks, Value};
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CtxItem {
    Radix(Radix),
    WaveDigital,
    WaveAnalog,
    SplitBus,
    CreateBus,
    Remove,
    AddToWaveform,
    SelectAllSource,
    NewGroup,
    RenameGroup,
    ExpandGroup,
    CollapseGroup,
    ExpandAll,
    CollapseAll,
    RemoveGroup,
}

/// One row of a context menu: either a leaf item or a submenu.
#[derive(Clone, Copy)]
pub enum CtxEntry {
    Item(&'static str, CtxItem),
    Submenu(&'static str, &'static [CtxEntry]),
}

const RADIX_MENU: &[CtxEntry] = &[
    CtxEntry::Item("Hex", CtxItem::Radix(Radix::Hex)),
    CtxEntry::Item("Binary", CtxItem::Radix(Radix::Bin)),
    CtxEntry::Item("Octal", CtxItem::Radix(Radix::Oct)),
    CtxEntry::Item("Decimal", CtxItem::Radix(Radix::Dec)),
    CtxEntry::Item("ASCII", CtxItem::Radix(Radix::Ascii)),
];

const WAVEFORM_MENU: &[CtxEntry] = &[
    CtxEntry::Item("Digital", CtxItem::WaveDigital),
    CtxEntry::Item("Analog", CtxItem::WaveAnalog),
];

const BUS_MENU: &[CtxEntry] = &[
    CtxEntry::Item("Split Bus...", CtxItem::SplitBus),
    CtxEntry::Item("Create Bus...", CtxItem::CreateBus),
];

pub const SIGNAL_MENU: &[CtxEntry] = &[
    CtxEntry::Submenu("Set Radix", RADIX_MENU),
    CtxEntry::Submenu("Set Waveform", WAVEFORM_MENU),
    CtxEntry::Submenu("Bus Operations", BUS_MENU),
    CtxEntry::Item("Remove Signal", CtxItem::Remove),
];

pub const GROUP_MENU: &[CtxEntry] = &[
    CtxEntry::Item("New Group", CtxItem::NewGroup),
    CtxEntry::Item("Rename Group...", CtxItem::RenameGroup),
    CtxEntry::Item("Expand Group", CtxItem::ExpandGroup),
    CtxEntry::Item("Collapse Group", CtxItem::CollapseGroup),
    CtxEntry::Item("Expand All Groups", CtxItem::ExpandAll),
    CtxEntry::Item("Collapse All Groups", CtxItem::CollapseAll),
    CtxEntry::Item("Remove Group", CtxItem::RemoveGroup),
];

pub const SOURCE_MENU: &[CtxEntry] = &[
    CtxEntry::Item("Add to Waveform", CtxItem::AddToWaveform),
    CtxEntry::Item("Select All Module Text", CtxItem::SelectAllSource),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtxTarget {
    Signal(usize),
    /// Stable group id.
    Group(u32),
    /// RTL source pane (selection).
    Source,
}

#[derive(Clone, Debug)]
pub struct ContextMenu {
    pub x: u16,
    pub y: u16,
    /// Selected row in the active level (root or open submenu).
    pub sel: usize,
    pub target: CtxTarget,
    /// Index of the open submenu in the root level.
    pub submenu: Option<usize>,
}

impl CtxEntry {
    pub fn label(&self) -> &'static str {
        match self {
            CtxEntry::Item(name, _) | CtxEntry::Submenu(name, _) => name,
        }
    }
}

impl App {
    /// Entries of the root level, which depends on the menu target.
    pub fn ctx_root(&self) -> &'static [CtxEntry] {
        match self.ctx_menu.as_ref().map(|menu| &menu.target) {
            Some(CtxTarget::Group(_)) => GROUP_MENU,
            Some(CtxTarget::Source) => SOURCE_MENU,
            _ => SIGNAL_MENU,
        }
    }

    /// Entries of the level currently being navigated.
    pub fn ctx_level(&self) -> &'static [CtxEntry] {
        let root = self.ctx_root();
        match self.ctx_menu.as_ref().and_then(|menu| menu.submenu) {
            Some(index) => match root.get(index) {
                Some(CtxEntry::Submenu(_, entries)) => entries,
                _ => root,
            },
            None => root,
        }
    }

    pub fn open_context_menu(&mut self, target: CtxTarget, x: u16, y: u16) {
        self.menu.open = None;
        self.ctx_menu = Some(ContextMenu {
            x,
            y,
            sel: 0,
            target,
            submenu: None,
        });
    }

    pub fn open_ctx_submenu(&mut self, index: usize) {
        if !matches!(self.ctx_root().get(index), Some(CtxEntry::Submenu(..))) {
            return;
        }
        if let Some(menu) = self.ctx_menu.as_mut() {
            menu.submenu = Some(index);
            menu.sel = 0;
        }
    }

    pub fn close_ctx_submenu(&mut self) {
        if let Some(menu) = self.ctx_menu.as_mut() {
            if let Some(index) = menu.submenu.take() {
                menu.sel = index;
            }
        }
    }

    pub fn run_ctx_item(&mut self, item: CtxItem) {
        let Some(menu) = self.ctx_menu.take() else {
            return;
        };
        match (menu.target, item) {
            (CtxTarget::Signal(sig), CtxItem::Radix(radix)) => {
                let targets = self.action_targets(sig);
                for &idx in &targets {
                    self.apply_radix(idx, radix);
                }
                if targets.len() == 1 {
                    let name = self.signal_name(sig);
                    self.msg(format!("radix of {name}: {}", radix.name()));
                } else {
                    self.msg(format!(
                        "radix of {} signals: {}",
                        targets.len(),
                        radix.name()
                    ));
                }
            }
            (CtxTarget::Signal(sig), CtxItem::WaveDigital) => self.set_analog_many(sig, false),
            (CtxTarget::Signal(sig), CtxItem::WaveAnalog) => self.set_analog_many(sig, true),
            (CtxTarget::Signal(sig), CtxItem::SplitBus) => self.open_split_dialog(sig),
            (CtxTarget::Signal(sig), CtxItem::CreateBus) => self.open_create_bus(sig),
            (CtxTarget::Signal(sig), CtxItem::Remove) if self.selection.contains(&sig) => {
                self.remove_selected()
            }
            (CtxTarget::Signal(sig), CtxItem::Remove) => self.remove_signal(sig),
            (CtxTarget::Group(id), CtxItem::NewGroup) => {
                if let Some(index) = self.group_index(id) {
                    self.insert_group_after(index);
                }
            }
            (CtxTarget::Group(id), CtxItem::RenameGroup) => self.open_rename_group(id),
            (CtxTarget::Group(id), CtxItem::ExpandGroup) => {
                if let Some(index) = self.group_index(id) {
                    self.set_group_collapsed(index, false);
                }
            }
            (CtxTarget::Group(id), CtxItem::CollapseGroup) => {
                if let Some(index) = self.group_index(id) {
                    self.set_group_collapsed(index, true);
                }
            }
            (_, CtxItem::ExpandAll) => self.expand_all(),
            (_, CtxItem::CollapseAll) => self.collapse_all(),
            (CtxTarget::Group(id), CtxItem::RemoveGroup) => {
                if let Some(index) = self.group_index(id) {
                    self.remove_group(index);
                }
            }
            (CtxTarget::Source, CtxItem::AddToWaveform) => self.add_source_selection(),
            (CtxTarget::Source, CtxItem::SelectAllSource) => self.select_all_source(),
            _ => {}
        }
    }

    /// Open the rename dialog for a group (prefilled with its current name).
    pub fn open_rename_group(&mut self, id: u32) {
        let Some(index) = self.group_index(id) else {
            return;
        };
        self.renaming_group = Some(id);
        self.input.set(&self.groups[index].name.clone());
        self.open_dialog(Dialog::GroupName);
    }

    pub fn apply_rename_group(&mut self) {
        let Some(id) = self.renaming_group.take() else {
            self.dialog = None;
            return;
        };
        let name = self.input.as_string();
        self.dialog = None;
        self.rename_group(id, name);
    }

    pub(crate) fn signal_name(&self, idx: usize) -> String {
        self.wf
            .as_ref()
            .and_then(|wf| wf.signals.get(idx))
            .map(Signal::full_name)
            .unwrap_or_else(|| "?".to_string())
    }

    pub fn set_analog(&mut self, idx: usize, analog: bool) {
        if self.apply_analog(idx, analog) {
            let name = self.signal_name(idx);
            self.msg(format!(
                "waveform of {name}: {}",
                if analog { "analog" } else { "digital" }
            ));
        } else {
            self.msg("waveform mode applies to logic signals");
        }
    }

    /// Apply a waveform mode to the clicked signal or the whole selection.
    pub fn set_analog_many(&mut self, idx: usize, analog: bool) {
        let targets = self.action_targets(idx);
        if targets.len() == 1 {
            self.set_analog(idx, analog);
            return;
        }
        let mut done = 0usize;
        for &target in &targets {
            if self.apply_analog(target, analog) {
                done += 1;
            }
        }
        if done == 0 {
            self.msg("waveform mode applies to logic signals");
        } else {
            self.msg(format!(
                "waveform of {done} signals: {}",
                if analog { "analog" } else { "digital" }
            ));
        }
    }

    fn apply_analog(&mut self, idx: usize, analog: bool) -> bool {
        let Some(sig) = self.wf.as_ref().and_then(|wf| wf.signals.get(idx)) else {
            return false;
        };
        if sig.kind != SigKind::Bits {
            return false;
        }
        if analog {
            let (min, max) = bits_range(&sig.changes);
            self.analog.insert(idx, (min, max));
        } else {
            self.analog.remove(&idx);
        }
        true
    }

    /// Ask for a chunk width, then split the bus into chunks of that width.
    pub fn open_split_dialog(&mut self, idx: usize) {
        let ok = self
            .wf
            .as_ref()
            .and_then(|wf| wf.signals.get(idx))
            .map(|sig| sig.kind == SigKind::Bits && sig.bits >= 2)
            .unwrap_or(false);
        if !ok {
            self.msg("split bus: select a multi-bit logic signal");
            return;
        }
        self.pending_split = Some(idx);
        self.input.set("1");
        self.open_dialog(Dialog::SplitBus);
    }

    pub fn apply_split_bus(&mut self) {
        let Some(idx) = self.pending_split.take() else {
            self.dialog = None;
            return;
        };
        let width = self.input.as_string().trim().parse::<u32>().unwrap_or(0);
        self.dialog = None;
        if width == 0 {
            self.msg("split bus: width must be a positive integer");
            return;
        }
        self.split_bus(idx, width);
    }

    /// Replace a multi-bit signal with one display signal per bit if
    /// `width == 1`, or per `width`-bit chunk otherwise.
    pub fn split_bus(&mut self, idx: usize, width: u32) {
        let Some((first, last)) = self.append_bit_chunks(idx, width) else {
            return;
        };
        let name = self
            .wf
            .as_ref()
            .and_then(|wf| wf.signals.get(idx))
            .map(|signal| signal.full_name())
            .unwrap_or_default();
        let pos = self
            .display
            .iter()
            .position(|&s| s == idx)
            .unwrap_or(self.display.len());
        let group = self.group_of_pos(pos);
        let start = (pos + 1).min(self.display.len());
        for (offset, sig) in (first..last).enumerate() {
            self.display_insert(group, start + offset, sig);
        }
        self.sel_row = None;
        self.scroll_to_row_of(first);
        let count = last - first;
        self.msg(format!(
            "split {name} into {count} chunks of {width} bit(s)"
        ));
    }

    /// Create the bit/chunk signals of a bus and append them to the waveform.
    /// Returns the `(first, last)` range of the new signals.
    pub(crate) fn append_bit_chunks(&mut self, idx: usize, width: u32) -> Option<(usize, usize)> {
        let source = self
            .wf
            .as_ref()
            .and_then(|wf| wf.signals.get(idx))
            .cloned()?;
        if source.kind != SigKind::Bits || source.bits < 2 {
            self.msg("bus: select a multi-bit logic signal");
            return None;
        }
        let width = width.max(1);
        if width >= source.bits {
            self.msg(format!(
                "bus: width must be smaller than {} bits",
                source.bits
            ));
            return None;
        }
        let mut created = Vec::new();
        let mut offset = 0u32;
        while offset < source.bits {
            let hi = (offset + width).min(source.bits);
            let chunk_bits = (hi - offset) as usize;
            let mut changes: Vec<Change> = Vec::new();
            for change in &source.changes {
                let Some(bits) = change.v.as_bits() else {
                    continue;
                };
                let value = Value::Bits(
                    (offset..hi)
                        .map(|bit| bits.get(bit as usize).copied().unwrap_or(2))
                        .collect(),
                );
                if changes.last().map(|c| c.v == value).unwrap_or(false) {
                    continue;
                }
                changes.push(Change {
                    t: change.t,
                    v: value,
                });
            }
            let base = bus_base_name(&source.name);
            let name = if chunk_bits == 1 {
                format!("{base}[{}]", offset)
            } else {
                format!("{base}[{}:{}]", hi - 1, offset)
            };
            created.push(Signal {
                name,
                bits: chunk_bits as u32,
                var_type: source.var_type.clone(),
                scope: source.scope.clone(),
                kind: SigKind::Bits,
                changes,
                min: f64::INFINITY,
                max: f64::NEG_INFINITY,
                parent: Some(idx),
            });
            offset = hi;
        }

        let first = {
            let wf = self.wf.as_mut().unwrap();
            let first = wf.signals.len();
            wf.signals.extend(created);
            first
        };
        let last = self.wf.as_ref().unwrap().signals.len();
        Some((first, last))
    }

    /// Open the ordering window for building a bus out of the selected
    /// signals (any width; each entry can be trimmed to a bit range).
    pub fn open_create_bus(&mut self, idx: usize) {
        let targets = if self.selection.is_empty() {
            vec![idx]
        } else {
            self.selection.clone()
        };
        if targets.len() < 2 {
            self.msg("create bus: select two or more signals first (Shift/Alt/Ctrl+click, Space)");
            return;
        }
        let mut items = Vec::with_capacity(targets.len());
        for &s in &targets {
            let Some(sig) = self.wf.as_ref().and_then(|wf| wf.signals.get(s)) else {
                return;
            };
            if sig.kind != SigKind::Bits {
                self.msg(format!(
                    "create bus: {} is not a logic signal",
                    sig.full_name()
                ));
                return;
            }
            items.push(BusItem {
                sig: s,
                hi: sig.bits.saturating_sub(1),
                lo: 0,
            });
        }
        self.bus_builder = Some(BusBuilder { items, sel: 0 });
        self.open_dialog(Dialog::CreateBus);
    }

    /// Merge the given signal slices into a bus, first entry = MSB.
    pub fn create_bus_from(&mut self, items: &[BusItem]) {
        if items.len() < 2 {
            return;
        }
        let width: usize = items.iter().map(|item| item.width() as usize).sum();
        let (changes, min, max, scope, base) = {
            let Some(wf) = self.wf.as_ref() else { return };
            let mut times: BTreeSet<Ticks> = BTreeSet::new();
            for item in items {
                for change in &wf.signals[item.sig].changes {
                    times.insert(change.t);
                }
            }
            let mut changes: Vec<Change> = Vec::new();
            for &t in &times {
                let mut value = vec![2u8; width];
                let mut pos = 0usize;
                for item in items {
                    let bits = wf.signals[item.sig]
                        .value_at(t)
                        .and_then(|v| v.as_bits().map(<[u8]>::to_vec));
                    for bit in (item.lo..=item.hi).rev() {
                        value[width - 1 - pos] = bits
                            .as_ref()
                            .and_then(|b| b.get(bit as usize).copied())
                            .unwrap_or(2);
                        pos += 1;
                    }
                }
                let value = Value::Bits(value);
                if changes.last().map(|c| c.v == value).unwrap_or(false) {
                    continue;
                }
                changes.push(Change { t, v: value });
            }
            let (min, max) = bits_range(&changes);
            let first = &wf.signals[items[0].sig];
            (
                changes,
                min,
                max,
                first.scope.clone(),
                bus_base_name(&first.name),
            )
        };

        let signal = Signal {
            name: format!("{base}_bus[{}:0]", width - 1),
            bits: width as u32,
            var_type: "wire".to_string(),
            scope,
            kind: SigKind::Bits,
            changes,
            min,
            max,
            parent: None,
        };
        let new_index = {
            let wf = self.wf.as_mut().unwrap();
            wf.signals.push(signal);
            wf.signals.len() - 1
        };
        let last_row = items
            .iter()
            .filter_map(|item| self.display.iter().position(|x| *x == item.sig))
            .max()
            .map(|r| r + 1)
            .unwrap_or(self.display.len());
        let group = self.group_of_pos(last_row.saturating_sub(1));
        let at = last_row.min(self.display.len());
        self.display_insert(group, at, new_index);
        self.scroll_to_row_of(new_index);
        self.msg(format!(
            "created bus {} from {} bits",
            self.signal_name(new_index),
            width
        ));
    }

    pub(crate) fn scroll_to_row_of(&mut self, sig: usize) {
        self.sel_row = self
            .list_rows()
            .iter()
            .position(|row| matches!(row, super::ListRow::Signal { sig: s, .. } if *s == sig));
        self.scroll_to_sel();
    }
}

/// One entry of the "Create Bus" ordering window: a slice `[hi:lo]`
/// (inclusive bit indices) of a logic signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusItem {
    pub sig: usize,
    pub hi: u32,
    pub lo: u32,
}

impl BusItem {
    pub fn width(&self) -> u32 {
        self.hi - self.lo + 1
    }
}

/// State of the "Create Bus" ordering window.
pub struct BusBuilder {
    pub items: Vec<BusItem>,
    pub sel: usize,
}

impl App {
    pub fn bus_builder(&self) -> Option<&BusBuilder> {
        self.bus_builder.as_ref()
    }

    /// Move the LSB (`msb == false`) or MSB (`msb == true`) of the selected
    /// slice. `delta` is in bits and the range never leaves the signal.
    pub fn bus_builder_trim(&mut self, msb: bool, delta: i64) {
        let Some(bits) = self
            .bus_builder
            .as_ref()
            .and_then(|builder| builder.items.get(builder.sel))
            .and_then(|item| self.wf.as_ref().map(|wf| wf.signals[item.sig].bits))
        else {
            return;
        };
        let Some(builder) = self.bus_builder.as_mut() else {
            return;
        };
        let Some(item) = builder.items.get_mut(builder.sel) else {
            return;
        };
        if bits <= 1 {
            return;
        }
        if msb {
            item.hi = (item.hi as i64 + delta).clamp(item.lo as i64, bits as i64 - 1) as u32;
        } else {
            item.lo = (item.lo as i64 + delta).clamp(0, item.hi as i64) as u32;
        }
    }

    /// Reset the selected slice to the full signal width.
    pub fn bus_builder_reset_range(&mut self) {
        let full = self
            .bus_builder
            .as_ref()
            .and_then(|builder| builder.items.get(builder.sel))
            .and_then(|item| {
                self.wf
                    .as_ref()
                    .map(|wf| wf.signals[item.sig].bits.saturating_sub(1))
            });
        let Some(full) = full else { return };
        if let Some(builder) = self.bus_builder.as_mut() {
            if let Some(item) = builder.items.get_mut(builder.sel) {
                item.hi = full;
                item.lo = 0;
            }
        }
    }

    pub fn bus_builder_move(&mut self, delta: i64) {
        if let Some(builder) = self.bus_builder.as_mut() {
            if builder.items.is_empty() {
                return;
            }
            let last = builder.items.len() as i64 - 1;
            builder.sel = (builder.sel as i64 + delta).clamp(0, last) as usize;
        }
    }

    /// Select a bus-builder row directly (used by the dialog scrollbar).
    pub fn bus_builder_select(&mut self, sel: usize) {
        if let Some(builder) = self.bus_builder.as_mut() {
            if !builder.items.is_empty() {
                builder.sel = sel.min(builder.items.len() - 1);
            }
        }
    }

    /// Move the selected signal up/down within the bus order.
    pub fn bus_builder_reorder(&mut self, delta: i64) {
        let Some(builder) = self.bus_builder.as_mut() else {
            return;
        };
        let target = builder.sel as i64 + delta;
        if target < 0 || target >= builder.items.len() as i64 {
            return;
        }
        builder.items.swap(builder.sel, target as usize);
        builder.sel = target as usize;
    }

    pub fn bus_builder_reverse(&mut self) {
        if let Some(builder) = self.bus_builder.as_mut() {
            if builder.items.is_empty() {
                return;
            }
            builder.items.reverse();
            let last = builder.items.len() - 1;
            builder.sel = last - builder.sel.min(last);
        }
    }

    /// Sort the bus members by hierarchical signal name.
    pub fn bus_builder_sort(&mut self, ascending: bool) {
        let Some(wf) = self.wf.as_ref() else { return };
        let Some(builder) = self.bus_builder.as_mut() else {
            return;
        };
        builder
            .items
            .sort_by_key(|item| wf.signals[item.sig].full_name().to_lowercase());
        if !ascending {
            builder.items.reverse();
        }
        builder.sel = 0;
    }

    pub fn bus_builder_cancel(&mut self) {
        self.dialog = None;
        self.bus_builder = None;
    }

    pub fn bus_builder_commit(&mut self) {
        let Some(builder) = self.bus_builder.take() else {
            return;
        };
        self.dialog = None;
        self.create_bus_from(&builder.items);
    }
}

fn bus_base_name(name: &str) -> String {
    match name.rsplit_once('[') {
        Some((base, rest)) if rest.ends_with(']') && !base.is_empty() => base.to_string(),
        _ => name.to_string(),
    }
}

/// Numeric min/max of a bit vector signal, ignoring unknown (x/z) values.
fn bits_range(changes: &[Change]) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for change in changes {
        let Some(bits) = change.v.as_bits() else {
            continue;
        };
        if bits.len() > 64 || bits.iter().any(|&b| b >= 2) {
            continue;
        }
        let mut value: u64 = 0;
        for &b in bits.iter().rev() {
            value = (value << 1) | b as u64;
        }
        min = min.min(value as f64);
        max = max.max(value as f64);
    }
    if !min.is_finite() || !max.is_finite() {
        (0.0, 1.0)
    } else {
        (min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::app_with;

    const VCD: &str = "$timescale 1ns $end\n\
        $var wire 1 ! b0 $end\n\
        $var wire 1 \" b1 $end\n\
        $var wire 1 # b2 $end\n\
        $var reg 4 $ data [3:0] $end\n\
        $var reg 8 % wide [7:0] $end\n\
        $enddefinitions $end\n\
        #0\n0!\n0\"\n0#\n\
        b0000 $\n\
        b00000000 %\n\
        #5\n1!\n\
        #10\n0!\n1\"\n\
        b0011 $\n\
        b10101010 %\n\
        #15\n1!\n0#\n\
        b1010 $\n\
        b11110000 %\n";

    #[test]
    fn split_bus_creates_bit_signals() {
        let mut app = app_with(VCD);
        app.set_display(vec![3, 4, 0, 1]);
        app.split_bus(3, 1);
        // 4 new bits appended after the source row.
        assert_eq!(app.display.len(), 8);
        assert_eq!(app.display[1..5], [5, 6, 7, 8]);
        let wf = app.wf.as_ref().unwrap();
        for idx in 5..9 {
            assert_eq!(wf.signals[idx].bits, 1);
            assert_eq!(wf.signals[idx].kind, SigKind::Bits);
        }
        assert_eq!(wf.signals[5].name, "data[0]");
        assert_eq!(wf.signals[8].name, "data[3]");
        let times: Vec<u64> = wf.signals[5].changes.iter().map(|c| c.t).collect();
        assert_eq!(times, vec![0, 10, 15]);
    }

    #[test]
    fn split_bus_with_chunk_width() {
        let mut app = app_with(VCD);
        app.set_display(vec![4]);
        app.split_bus(4, 4);
        let wf = app.wf.as_ref().unwrap();
        // Chunks are created from the LSB upward.
        assert_eq!(wf.signals[5].name, "wide[3:0]");
        assert_eq!(wf.signals[5].bits, 4);
        assert_eq!(wf.signals[6].name, "wide[7:4]");
        assert_eq!(wf.signals[6].bits, 4);
        // wide = 0b10101010 -> [3:0] = 0xa
        let low = wf.signals[5].value_at(10).unwrap().as_bits().unwrap();
        assert_eq!(low, &[0, 1, 0, 1]);
    }

    #[test]
    fn split_bus_with_remainder_chunk() {
        let mut app = app_with(VCD);
        app.set_display(vec![4]);
        app.split_bus(4, 3);
        let wf = app.wf.as_ref().unwrap();
        assert_eq!(wf.signals[5].name, "wide[2:0]");
        assert_eq!(wf.signals[5].bits, 3);
        assert_eq!(wf.signals[6].name, "wide[5:3]");
        assert_eq!(wf.signals[6].bits, 3);
        assert_eq!(wf.signals[7].name, "wide[7:6]");
        assert_eq!(wf.signals[7].bits, 2);
        // wide = 0b10101010 -> [7:6] = 0b10
        let top = wf.signals[7].value_at(10).unwrap().as_bits().unwrap();
        assert_eq!(top, &[0, 1]);
    }

    #[test]
    fn create_bus_orders_members_from_builder() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1, 2]);
        app.selection = vec![0, 1, 2];
        app.open_create_bus(0);
        assert!(app.bus_builder().is_some());
        // b0,b1,b2 -> reverse -> b2,b1,b0 (first = MSB)
        app.bus_builder_reverse();
        app.bus_builder_commit();
        let bus = app.wf.as_ref().unwrap().signals.last().unwrap();
        assert_eq!(bus.bits, 3);
        assert_eq!(bus.name, "b2_bus[2:0]");
        // at t=15: b2=0 (MSB), b1=1, b0=1 -> 0b011
        let bits = bus.value_at(15).unwrap().as_bits().unwrap();
        assert_eq!(bits, &[1, 1, 0]);
    }

    #[test]
    fn create_bus_without_selection_asks_for_more_signals() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1, 2]);
        app.selection.clear();
        app.open_create_bus(0);
        assert!(app.bus_builder().is_none());
        assert_eq!(app.dialog, None);
    }

    #[test]
    fn create_bus_joins_arbitrary_widths_and_slices() {
        let mut app = app_with(VCD);
        app.set_display(vec![3, 0]);
        app.selection = vec![3, 0]; // data [3:0] + b0
        app.open_create_bus(3);
        assert_eq!(app.bus_builder().unwrap().items.len(), 2);
        // Trim data to [3:2].
        app.bus_builder_trim(false, 2);
        let item = app.bus_builder().unwrap().items[0];
        assert_eq!((item.hi, item.lo), (3, 2));
        app.bus_builder_commit();
        let bus = app.wf.as_ref().unwrap().signals.last().unwrap();
        assert_eq!(bus.bits, 3);
        assert_eq!(bus.name, "data_bus[2:0]");
        // at t=5: data=0000 -> [3:2]=00, b0=1 -> 0b001
        let bits = bus.value_at(5).unwrap().as_bits().unwrap();
        assert_eq!(bits, &[1, 0, 0]);
        // at t=15: data=1010 -> [3:2]=10, b0=1 -> 0b101
        let bits = bus.value_at(15).unwrap().as_bits().unwrap();
        assert_eq!(bits, &[1, 0, 1]);
    }

    #[test]
    fn bus_builder_trim_stays_inside_the_signal() {
        let mut app = app_with(VCD);
        app.set_display(vec![4]);
        app.selection = vec![4, 0];
        app.open_create_bus(4);
        app.bus_builder_trim(false, 100); // LSB can not pass the MSB
        app.bus_builder_trim(true, -100); // MSB can not pass the LSB
        let item = app.bus_builder().unwrap().items[0];
        assert!(item.lo <= item.hi);
        assert!(item.hi < 8);
        app.bus_builder_reset_range();
        let item = app.bus_builder().unwrap().items[0];
        assert_eq!((item.hi, item.lo), (7, 0));
    }

    #[test]
    fn bus_builder_sort_by_name() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1, 2]);
        app.selection = vec![0, 1, 2];
        app.open_create_bus(0);
        app.bus_builder_sort(false); // descending
        let order: Vec<String> = app
            .bus_builder()
            .unwrap()
            .items
            .iter()
            .map(|item| app.wf.as_ref().unwrap().signals[item.sig].name.clone())
            .collect();
        assert_eq!(order, vec!["b2", "b1", "b0"]);
    }

    #[test]
    fn group_context_menu_uses_group_entries() {
        let mut app = app_with(
            "$timescale 1ns $end\n\
            $scope module top $end\n\
            $var wire 1 ! clk $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\n0!\n",
        );
        app.set_display(vec![0]);
        app.open_context_menu(CtxTarget::Group(0), 5, 5);
        assert_eq!(app.ctx_root().len(), GROUP_MENU.len());
        app.run_ctx_item(CtxItem::CollapseGroup);
        assert_eq!(app.list_rows().len(), 1);
        app.expand_all();
        assert_eq!(app.list_rows().len(), 2);
    }

    #[test]
    fn signal_menu_has_submenus() {
        let mut app = app_with(VCD);
        app.open_context_menu(CtxTarget::Signal(3), 5, 5);
        assert_eq!(app.ctx_level().len(), SIGNAL_MENU.len());
        app.open_ctx_submenu(0); // Set Radix
        assert_eq!(app.ctx_level().len(), RADIX_MENU.len());
        app.close_ctx_submenu();
        assert_eq!(app.ctx_level().len(), SIGNAL_MENU.len());
    }
}
