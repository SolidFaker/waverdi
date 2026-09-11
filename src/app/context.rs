use super::App;
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
    ExpandGroup,
    CollapseGroup,
    ExpandAll,
    CollapseAll,
    RemoveGroup,
}

pub const SIGNAL_ITEMS: [(&str, CtxItem); 10] = [
    ("Set Radix: Hex", CtxItem::Radix(Radix::Hex)),
    ("Set Radix: Binary", CtxItem::Radix(Radix::Bin)),
    ("Set Radix: Octal", CtxItem::Radix(Radix::Oct)),
    ("Set Radix: Decimal", CtxItem::Radix(Radix::Dec)),
    ("Set Radix: ASCII", CtxItem::Radix(Radix::Ascii)),
    ("Set Waveform: Digital", CtxItem::WaveDigital),
    ("Set Waveform: Analog", CtxItem::WaveAnalog),
    ("Bus: Split Bus", CtxItem::SplitBus),
    ("Bus: Create Bus", CtxItem::CreateBus),
    ("Remove Signal", CtxItem::Remove),
];

pub const GROUP_ITEMS: [(&str, CtxItem); 5] = [
    ("Expand Group", CtxItem::ExpandGroup),
    ("Collapse Group", CtxItem::CollapseGroup),
    ("Expand All Groups", CtxItem::ExpandAll),
    ("Collapse All Groups", CtxItem::CollapseAll),
    ("Remove Group", CtxItem::RemoveGroup),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CtxTarget {
    Signal(usize),
    Group(String),
}

#[derive(Clone, Debug)]
pub struct ContextMenu {
    pub x: u16,
    pub y: u16,
    pub sel: usize,
    pub target: CtxTarget,
}

impl App {
    pub fn ctx_items(&self) -> &'static [(&'static str, CtxItem)] {
        match self.ctx_menu.as_ref().map(|menu| &menu.target) {
            Some(CtxTarget::Group(_)) => &GROUP_ITEMS,
            _ => &SIGNAL_ITEMS,
        }
    }

    pub fn open_context_menu(&mut self, target: CtxTarget, x: u16, y: u16) {
        self.menu.open = None;
        self.ctx_menu = Some(ContextMenu {
            x,
            y,
            sel: 0,
            target,
        });
    }

    pub fn run_ctx_item(&mut self, item: CtxItem) {
        let Some(menu) = self.ctx_menu.take() else {
            return;
        };
        match (menu.target, item) {
            (CtxTarget::Signal(sig), CtxItem::Radix(radix)) => {
                self.radix.insert(sig, radix);
                let name = self.signal_name(sig);
                self.msg(format!("radix of {name}: {}", radix.name()));
            }
            (CtxTarget::Signal(sig), CtxItem::WaveDigital) => self.set_analog(sig, false),
            (CtxTarget::Signal(sig), CtxItem::WaveAnalog) => self.set_analog(sig, true),
            (CtxTarget::Signal(sig), CtxItem::SplitBus) => self.split_bus(sig),
            (CtxTarget::Signal(sig), CtxItem::CreateBus) => self.create_bus(sig),
            (CtxTarget::Signal(sig), CtxItem::Remove) => self.remove_signal(sig),
            (CtxTarget::Group(path), CtxItem::ExpandGroup) => {
                self.set_group_collapsed(&path, false)
            }
            (CtxTarget::Group(path), CtxItem::CollapseGroup) => {
                self.set_group_collapsed(&path, true)
            }
            (_, CtxItem::ExpandAll) => self.expand_all(),
            (_, CtxItem::CollapseAll) => self.collapse_all(),
            (CtxTarget::Group(path), CtxItem::RemoveGroup) => self.remove_group(&path),
            _ => {}
        }
    }

    fn signal_name(&self, idx: usize) -> String {
        self.wf
            .as_ref()
            .and_then(|wf| wf.signals.get(idx))
            .map(Signal::full_name)
            .unwrap_or_else(|| "?".to_string())
    }

    pub fn set_analog(&mut self, idx: usize, analog: bool) {
        let Some(sig) = self.wf.as_ref().and_then(|wf| wf.signals.get(idx)) else {
            return;
        };
        if sig.kind != SigKind::Bits {
            self.msg("waveform mode applies to logic signals");
            return;
        }
        let name = sig.full_name();
        if analog {
            let (min, max) = bits_range(&sig.changes);
            self.analog.insert(idx, (min, max));
            self.msg(format!("waveform of {name}: analog"));
        } else {
            self.analog.remove(&idx);
            self.msg(format!("waveform of {name}: digital"));
        }
    }

    /// Replace a multi-bit signal with one display signal per bit.
    pub fn split_bus(&mut self, idx: usize) {
        let Some(source) = self.wf.as_ref().and_then(|wf| wf.signals.get(idx)).cloned() else {
            return;
        };
        if source.kind != SigKind::Bits || source.bits < 2 {
            self.msg("split bus: select a multi-bit logic signal");
            return;
        }
        let width = source.bits as usize;
        let mut created = Vec::with_capacity(width);
        for bit in 0..width {
            let mut changes: Vec<Change> = Vec::new();
            for change in &source.changes {
                let Some(bits) = change.v.as_bits() else {
                    continue;
                };
                let value = Value::Bits(vec![bits.get(bit).copied().unwrap_or(2)]);
                if changes.last().map(|c| c.v == value).unwrap_or(false) {
                    continue;
                }
                changes.push(Change {
                    t: change.t,
                    v: value,
                });
            }
            created.push(Signal {
                name: format!("{}[{bit}]", source.name),
                bits: 1,
                var_type: source.var_type.clone(),
                scope: source.scope.clone(),
                kind: SigKind::Bits,
                changes,
                min: f64::INFINITY,
                max: f64::NEG_INFINITY,
            });
        }

        let first = {
            let wf = self.wf.as_mut().unwrap();
            let first = wf.signals.len();
            wf.signals.extend(created);
            first
        };
        let row = self
            .display
            .iter()
            .position(|&s| s == idx)
            .map(|r| r + 1)
            .unwrap_or(self.display.len());
        let last = self.wf.as_ref().unwrap().signals.len();
        for (k, sig) in (first..last).enumerate() {
            let at = (row + k).min(self.display.len());
            self.display.insert(at, sig);
        }
        self.sel_row = None;
        self.scroll_to_row_of(first);
        let name = source.full_name();
        self.msg(format!("split {name} into {width} bits"));
    }

    pub(crate) fn scroll_to_row_of(&mut self, sig: usize) {
        self.sel_row = self
            .list_rows()
            .iter()
            .position(|row| matches!(row, super::ListRow::Signal { sig: s, .. } if *s == sig));
        self.scroll_to_sel();
    }

    /// Merge the clicked 1-bit signal and the following displayed 1-bit
    /// signals (top to bottom, first = MSB) into a new bus.
    pub fn create_bus(&mut self, idx: usize) {
        let Some(row) = self.display.iter().position(|&s| s == idx) else {
            return;
        };
        let mut bits = Vec::new();
        {
            let Some(wf) = &self.wf else { return };
            for &s in self.display[row..].iter() {
                let sig = &wf.signals[s];
                if sig.kind == SigKind::Bits && sig.bits == 1 {
                    bits.push(s);
                    if bits.len() == 32 {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
        if bits.len() < 2 {
            self.msg("create bus: need at least 2 consecutive 1-bit signals");
            return;
        }

        let width = bits.len();
        let (changes, min, max, scope, base) = {
            let wf = self.wf.as_ref().unwrap();
            let mut times: BTreeSet<Ticks> = BTreeSet::new();
            for &s in &bits {
                for change in &wf.signals[s].changes {
                    times.insert(change.t);
                }
            }
            let mut changes: Vec<Change> = Vec::new();
            for &t in &times {
                let mut value = vec![0u8; width];
                for (i, &s) in bits.iter().enumerate() {
                    let bit = wf.signals[s]
                        .value_at(t)
                        .and_then(Value::as_bits)
                        .and_then(|b| b.first().copied())
                        .unwrap_or(2);
                    value[width - 1 - i] = bit;
                }
                let value = Value::Bits(value);
                if changes.last().map(|c| c.v == value).unwrap_or(false) {
                    continue;
                }
                changes.push(Change { t, v: value });
            }
            let (min, max) = bits_range(&changes);
            let first = &wf.signals[bits[0]];
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
        };
        let new_index = {
            let wf = self.wf.as_mut().unwrap();
            wf.signals.push(signal);
            wf.signals.len() - 1
        };
        let at = (row + width).min(self.display.len());
        self.display.insert(at, new_index);
        self.scroll_to_row_of(new_index);
        self.msg(format!(
            "created bus {} from {} bits",
            self.signal_name(new_index),
            width
        ));
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
        $var reg 4 # data [3:0] $end\n\
        $enddefinitions $end\n\
        #0\n0!\n0\"\n\
        b0000 #\n\
        #5\n1!\n\
        #10\n0!\n1\"\n\
        b0011 #\n\
        #15\n1!\n\
        b1010 #\n";

    #[test]
    fn split_bus_creates_bit_signals() {
        let mut app = app_with(VCD);
        app.display = vec![0, 1, 2];
        app.split_bus(2);
        // 4 new bits appended after the source row.
        assert_eq!(app.display.len(), 7);
        assert_eq!(app.display[3..], [3, 4, 5, 6]);
        let wf = app.wf.as_ref().unwrap();
        for idx in 3..7 {
            assert_eq!(wf.signals[idx].bits, 1);
            assert_eq!(wf.signals[idx].kind, crate::waveform::SigKind::Bits);
        }
        assert_eq!(wf.signals[3].name, "data[0]");
        assert_eq!(wf.signals[6].name, "data[3]");
        // bit 0 changes: 0,1,0
        let times: Vec<u64> = wf.signals[3].changes.iter().map(|c| c.t).collect();
        assert_eq!(times, vec![0, 10, 15]);
    }

    #[test]
    fn create_bus_merges_consecutive_bits() {
        let mut app = app_with(VCD);
        app.display = vec![0, 1, 2];
        app.create_bus(0);
        // The new bus is inserted right after the two source bits.
        assert_eq!(app.display, vec![0, 1, 3, 2]);
        let bus = &app.wf.as_ref().unwrap().signals[3];
        assert_eq!(bus.bits, 2);
        assert_eq!(bus.name, "b0_bus[1:0]");
        // b0 is MSB, b1 is LSB: at t=10, b0=0, b1=1 -> b01
        assert_eq!(bus.value_at(10).unwrap().as_bits(), Some(&[1u8, 0u8][..]));
    }

    #[test]
    fn set_analog_and_digital() {
        let mut app = app_with(VCD);
        app.set_analog(2, true);
        assert!(app.analog.contains_key(&2));
        app.set_analog(2, false);
        assert!(!app.analog.contains_key(&2));
    }

    #[test]
    fn context_item_applies_radix() {
        let mut app = app_with(VCD);
        app.display = vec![2];
        app.sel_row = Some(0);
        app.open_context_menu(CtxTarget::Signal(2), 5, 5);
        app.run_ctx_item(CtxItem::Radix(crate::waveform::Radix::Dec));
        assert_eq!(app.radix_for(2), crate::waveform::Radix::Dec);
        assert!(app.ctx_menu.is_none());
    }

    #[test]
    fn group_context_menu_uses_group_items() {
        let mut app = app_with(
            "$timescale 1ns $end\n\
            $scope module top $end\n\
            $var wire 1 ! clk $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\n0!\n",
        );
        app.display = vec![0];
        app.open_context_menu(CtxTarget::Group("top".to_string()), 5, 5);
        assert_eq!(app.ctx_items().len(), GROUP_ITEMS.len());
        app.run_ctx_item(CtxItem::CollapseGroup);
        assert_eq!(app.list_rows().len(), 1);
        assert!(app.list_rows()[0].clone().eq(&crate::app::ListRow::Group {
            path: "top".to_string(),
            name: "top".to_string(),
            depth: 0,
            count: 1,
            collapsed: true,
        }));
    }
}
