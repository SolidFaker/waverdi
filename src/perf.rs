use crate::app::App;
use crate::waveform::{Change, ScopeTree, SigKind, SigState, Signal, TimeScale, Value, Waveform};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

fn signal(name: String, bits: u32, changes: Vec<Change>) -> Signal {
    Signal {
        name,
        bits,
        var_type: "wire".to_string(),
        dir: String::new(),
        scope: Vec::new(),
        kind: SigKind::Bits,
        changes: Arc::new(changes),
        min: 0.0,
        max: 1.0,
        parent: None,
        members: Vec::new(),
        state: SigState::Ready,
    }
}

fn waveform(signals: Vec<Signal>) -> Waveform {
    let end = signals
        .iter()
        .filter_map(|signal| signal.changes.last())
        .map(|change| change.t)
        .max()
        .unwrap_or(1);
    Waveform {
        ts: TimeScale::default(),
        start: 0,
        end,
        signals,
        tree: ScopeTree::new(),
        radix: HashMap::new(),
        value_times_cache: Vec::new(),
    }
}

fn measure(label: &str, iterations: usize, mut run: impl FnMut()) {
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        run();
        samples.push(start.elapsed());
    }
    samples.sort();
    println!(
        "{label}: min={:?}, median={:?}, samples={samples:?}",
        samples[0],
        samples[samples.len() / 2]
    );
}

#[test]
#[ignore = "performance comparison; run explicitly in release mode"]
fn list_frame_performance() {
    let signals = (0..10_000)
        .map(|index| {
            let mut signal = signal(
                format!("signal_{index}"),
                1,
                vec![Change {
                    t: 0,
                    v: Value::Small(0, 1),
                }],
            );
            signal.scope = (0..8).map(|depth| format!("scope_{depth}")).collect();
            signal
        })
        .collect();
    let mut app = App::new();
    app.apply_waveform("<perf>".to_string(), waveform(signals), Vec::new());
    app.set_display((0..10_000).collect());
    app.show_full_names = true;
    app.settings.splits.top_pct = 20;
    app.fit();
    let backend = ratatui::backend::TestBackend::new(160, 60);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| crate::ui::render(frame, &mut app))
        .unwrap();
    measure("list full-name cursor-only frames", 12, || {
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();
        black_box(app.list_content_width());
    });
}

#[test]
#[ignore = "performance comparison; run explicitly in release mode"]
fn aggregate_merge_performance() {
    const MEMBERS: usize = 256;
    const CHANGES: usize = 1_000;
    let mut signals = Vec::with_capacity(MEMBERS + 1);
    for member in 0..MEMBERS {
        let changes = (0..CHANGES)
            .map(|index| Change {
                t: (index * MEMBERS + member) as u64,
                v: Value::Small((index & 1) as u64, 1),
            })
            .collect();
        let mut signal = signal(format!("m{member}"), 1, changes);
        signal.parent = Some(MEMBERS);
        signals.push(signal);
    }
    let mut aggregate = signal("aggregate".to_string(), 1, Vec::new());
    aggregate.var_type = "aggregate".to_string();
    aggregate.members = (0..MEMBERS).collect();
    signals.push(aggregate);
    let wf = waveform(signals);
    let expected = wf.value_times(MEMBERS).len();
    measure("aggregate 256 x 1000 interleaved changes", 6, || {
        black_box(wf.value_times(MEMBERS));
    });
    assert_eq!(expected, 128_001);
}

#[test]
#[ignore = "performance comparison; run explicitly in release mode"]
fn navigation_and_value_query_performance() {
    const COUNT: usize = 2_000_000;
    let changes = (0..COUNT)
        .map(|index| Change {
            t: index as u64,
            v: Value::Small((index & 1) as u64, 1),
        })
        .collect();
    let mut app = App::new();
    app.apply_waveform(
        "<perf>".to_string(),
        waveform(vec![signal("clk".to_string(), 1, changes)]),
        Vec::new(),
    );
    app.set_display(vec![0]);
    app.sel_row = Some(1);
    app.sync_layout(ratatui::layout::Rect::new(0, 0, 160, 60));
    let cursor = (COUNT as u64) * 95 / 100;
    measure("next transition from 95% of 2M changes", 12, || {
        app.cursor = cursor;
        app.jump_edge(true, None);
        black_box(app.cursor);
    });
    app.value_query = Some("hffff_ffff".to_string());
    measure("numeric miss over 2M changes", 6, || {
        app.cursor = 0;
        app.search_value(true);
        black_box(app.cursor);
    });
}

#[test]
#[ignore = "performance comparison; run explicitly in release mode"]
fn lazy_leaf_preparation_performance() {
    let signals = (0..90_000)
        .map(|index| signal(format!("s{index}"), 1, Vec::new()))
        .collect();
    let wf = waveform(signals);
    measure("64 leaves from 90k signals", 12, || {
        let mut total = 0usize;
        for index in 0..64 {
            total += wf.value_leaves(index).len();
        }
        black_box(total);
    });
}
