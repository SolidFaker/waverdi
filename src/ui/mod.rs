pub mod add;
pub mod context;
pub mod dialog;
pub mod layout;
pub mod list;
pub mod menubar;
pub mod scrollbar;
pub mod source;
pub mod status;
pub mod text;
pub mod toolbar;
pub mod tree;
pub mod wave;

use crate::app::App;
use crate::ui::layout::{compute_layout, Layout};
use ratatui::buffer::Buffer;
use ratatui::style::Style;
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.sync_layout(area);
    app.sync_source();
    app.sync_source_trace();
    let l = compute_layout(area, app.splits);
    let t = &app.theme;

    {
        let buf = frame.buffer_mut();
        buf.set_style(area, Style::new().bg(t.bg));
        menubar::draw_bar(buf, &l, app);
        // Frames first: nWave (bottom), source and instance (top, shared row).
        let tree_focused = app.focus == crate::app::Focus::Tree;
        let nwave_focused = matches!(app.focus, crate::app::Focus::List | crate::app::Focus::Wave);
        wave::draw_nwave_frame(buf, &l, t, false);
        source::draw_frame(buf, &l, app);
        tree::draw_frame(buf, &l, t, tree_focused);
        if nwave_focused {
            // The shared row between the top row and nWave follows the focus.
            wave::draw_nwave_frame(buf, &l, t, true);
        }
        toolbar::draw(buf, &l, app);
        match &app.wf {
            None => status::draw_empty(buf, &l, t),
            Some(wf) => {
                tree::draw(buf, &l, app, wf);
                list::draw(buf, &l, app, wf);
                wave::draw(buf, &l, app, wf);
            }
        }
        draw_divider(buf, &l, app);
        source::draw(buf, &l, app);
        status::draw_messages(buf, &l, app);
        status::draw_status(buf, &l, app);
        if let Some(idx) = app.menu.open {
            menubar::draw_dropdown(buf, &l, app, idx);
        }
        context::draw(buf, &l, app);
        toolbar::draw_time_menu(buf, &l, app);
    }

    if app.dialog == Some(crate::app::Dialog::AddSignals) {
        add::draw(frame, l.area, app);
    } else if let Some(dialog) = app.dialog {
        dialog::draw(frame, &l, app, dialog);
    }
}

/// Vertical divider between the Signal List and the waveforms.
fn draw_divider(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    let fg = if app.focus == crate::app::Focus::Tree {
        t.panel_border
    } else {
        t.accent
    };
    for row in l.list.y..l.list.bottom() {
        if let Some(cell) = buf.cell_mut((l.list_grip_x(), row)) {
            cell.set_symbol("│");
            cell.set_fg(fg);
            cell.set_bg(t.bg);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::vcd;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const VCD: &str = "$timescale 1ns $end\n\
        $var wire 1 ! clk $end\n\
        $var wire 4 \" data $end\n\
        $var real 64 $ temp $end\n\
        $enddefinitions $end\n\
        #0\n0!\nb0000 \"\nr1.5 $\n\
        #5\n1!\nb0101 \"\n\
        #10\n0!\nr2.25 $\n\
        #15\n1!\nb1010 \"\n";

    fn render_app(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content
            .chunks(buffer.area.width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn mouse_at(col: u16, row: u16) -> crossterm::event::MouseEvent {
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: col,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }

    fn wheel_at(col: u16, row: u16, up: bool) -> crossterm::event::MouseEvent {
        crossterm::event::MouseEvent {
            kind: if up {
                crossterm::event::MouseEventKind::ScrollUp
            } else {
                crossterm::event::MouseEventKind::ScrollDown
            },
            column: col,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }

    #[test]
    fn empty_state_renders() {
        let mut app = App::new();
        let screen = render_app(&mut app, 100, 30);
        assert!(screen.contains("waverdi"));
        assert!(screen.contains("no waveform"));
    }

    #[test]
    fn waveform_renders_signal_data() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0, 1, 2]);
        app.fit();
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains("Instance"));
        assert!(screen.contains("Signal List"));
        assert!(screen.contains("clk"));
        assert!(screen.contains("data"));
        assert!(screen.contains("temp"));
        // High/low rails and a bus value must show up in the waveform pane.
        assert!(screen.contains('▔') || screen.contains('▁'));
        assert!(screen.contains("h5") || screen.contains("h0"));
    }

    #[test]
    fn edges_align_with_the_cursor_column() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 30));
        app.set_display(vec![0, 1]);
        app.scale = 2.0;
        app.t0 = 0.0;
        let screen = render_app(&mut app, 100, 30);
        let l = app.layout();
        let lines: Vec<Vec<char>> = screen.lines().map(|line| line.chars().collect()).collect();
        // Row 0 is the G0 header; clk and data follow.
        let clk = &lines[l.rows.y as usize + 1];
        assert_eq!(clk[l.rows.x as usize + app.x_at_tick(5) as usize], '/');
        assert_eq!(clk[l.rows.x as usize + app.x_at_tick(10) as usize], '\\');
        let data = &lines[l.rows.y as usize + 2];
        assert_eq!(data[l.rows.x as usize + app.x_at_tick(5) as usize], '╳');
    }

    #[test]
    fn renders_fst_waveform() {
        let out = crate::fst::parse_fst(std::path::Path::new("waveform/demo.fst")).unwrap();
        let mut app = App::new();
        app.apply_parsed("waveform/demo.fst", out);
        app.set_display((0..app.wf.as_ref().unwrap().signals.len()).collect());
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains("nWave"), "{screen}");
        assert!(screen.contains("clk"), "{screen}");
        assert!(screen.contains('▔') || screen.contains('▁'), "{screen}");
    }

    #[test]
    #[cfg(fsdb_sdk)]
    fn renders_fsdb_waveform() {
        let Some(home) = std::env::var_os("VERDI_HOME") else {
            return;
        };
        let path =
            std::path::Path::new(&home).join("demo/nCompare/nCmp_demo1/demo_RTL_verilog.fsdb");
        if !path.is_file() {
            return;
        }
        let out = crate::fsdb::parse_fsdb(&path).unwrap();
        let mut app = App::new();
        app.apply_parsed("demo_RTL_verilog.fsdb", out);
        app.set_display(
            (0..app.wf.as_ref().unwrap().signals.len())
                .take(12)
                .collect(),
        );
        let first = app.wf.as_ref().unwrap().signals[0].name.clone();
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains(&first), "{screen}");
        assert!(screen.contains("Signal List"), "{screen}");
    }

    #[test]
    fn fst_find_value() {
        let out = crate::fst::parse_fst(std::path::Path::new("waveform/demo.fst")).unwrap();
        let mut app = App::new();
        app.apply_parsed("waveform/demo.fst", out);
        app.set_display((0..app.wf.as_ref().unwrap().signals.len()).collect());
        let state = app
            .wf
            .as_ref()
            .unwrap()
            .signals
            .iter()
            .position(|sig| sig.name == "state")
            .unwrap();
        let row = app
            .list_rows()
            .iter()
            .position(|row| matches!(row, crate::app::ListRow::Signal { sig, .. } if *sig == state))
            .unwrap();
        app.sel_row = Some(row);
        app.value_query = Some("h2".to_string());
        app.search_value(true);
        assert_eq!(app.cursor, 30);
    }

    #[test]
    fn isolated_bit_edges_render_diagonals() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0]);
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains('/'), "{screen}");
        assert!(screen.contains('\\'), "{screen}");
    }

    #[test]
    fn list_shows_transition_when_cursor_sits_on_edge() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0]);
        app.cursor = 5; // clk 0 -> 1
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains('→'), "{screen}");
        app.cursor = 7; // between edges
        let screen = render_app(&mut app, 120, 30);
        assert!(!screen.contains('→'), "{screen}");
    }

    #[test]
    fn edge_detection_matches_display_column() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0]);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 120, 30));
        app.t0 = 0.0;
        app.scale = 10.0;
        app.cursor = 11; // column 1 covers ticks 5..15
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains('→'), "{screen}");
        app.cursor = 26; // column 3 covers ticks 25..35, no edge
        let screen = render_app(&mut app, 120, 30);
        assert!(!screen.contains('→'), "{screen}");
    }

    #[test]
    fn multi_selection_is_highlighted() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0, 1]);
        app.selection = vec![0, 1];
        app.sel_row = Some(1);
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let l = app.layout();
        // Row 0 is the group header; clk is selected and data is multi-selected.
        assert_eq!(
            buffer.cell((l.list.x + 2, l.list.y + 3)).unwrap().bg,
            crate::theme::Theme::DARK.row_sel_bg
        );
        assert_eq!(
            buffer.cell((l.list.x + 2, l.list.y + 4)).unwrap().bg,
            crate::theme::Theme::DARK.multi_sel_bg
        );
    }

    #[test]
    fn context_menu_renders() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0]);
        app.open_context_menu(crate::app::CtxTarget::Signal(0), 5, 5);
        let screen = render_app(&mut app, 100, 30);
        assert!(screen.contains("Set Radix"), "{screen}");
        assert!(screen.contains("Set Waveform"), "{screen}");
        assert!(screen.contains("Bus Operations"), "{screen}");
    }

    #[test]
    fn list_renders_user_groups() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0]);
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains("G0 (1)"), "{screen}");
        assert!(screen.contains("clk"), "{screen}");
        // Collapse: the group row stays, its signal does not.
        app.open_context_menu(crate::app::CtxTarget::Group(0), 5, 5);
        assert_eq!(app.ctx_root().len(), 7); // new / rename / expand / collapse / all / all / remove
        app.set_group_collapsed(0, true);
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains("G0 (1)"), "{screen}");
        // Signals are only drawn in the Signal List, which is collapsed.
        assert!(!screen.contains("clk"), "{screen}");
    }

    #[test]
    fn hierarchy_names_toggle_in_the_signal_list() {
        let scoped = "$timescale 1ns $end\n\
            $scope module top $end\n\
            $scope module sub $end\n\
            $var wire 1 ! clk $end\n\
            $upscope $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let out = vcd::parse_bytes(scoped.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0]);
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains("clk"), "{screen}");
        assert!(!screen.contains("top.sub.clk"), "{screen}");
        app.show_full_names = true;
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains("top.sub.clk"), "{screen}");
    }

    #[test]
    fn signal_context_submenus_render() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0]);
        app.open_context_menu(crate::app::CtxTarget::Signal(0), 5, 5);
        let screen = render_app(&mut app, 100, 30);
        assert!(screen.contains("Set Radix"), "{screen}");
        assert!(screen.contains("Bus Operations"), "{screen}");
        assert!(screen.contains("Highlight"), "{screen}");
        app.open_ctx_submenu(0);
        let screen = render_app(&mut app, 100, 30);
        assert!(screen.contains("Hex"), "{screen}");
        assert!(screen.contains("ASCII"), "{screen}");
        app.close_ctx_submenu();
        app.open_ctx_submenu(2);
        let screen = render_app(&mut app, 100, 30);
        assert!(screen.contains("Red"), "{screen}");
        assert!(screen.contains("Cyan"), "{screen}");
        app.close_ctx_submenu();
        app.open_ctx_submenu(3);
        let screen = render_app(&mut app, 100, 30);
        assert!(screen.contains("Split Bus..."), "{screen}");
        assert!(screen.contains("Create Bus..."), "{screen}");
    }

    #[test]
    fn highlight_color_paints_list_wave_and_source() {
        use crate::rtl::{RtlDb, SourceSet};
        use crate::ui::layout::Layout;
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $var wire 8 ! data [7:0] $end\n\
            $var wire 8 \" other [7:0] $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\nb0 !\nb0 \"\n";
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 40));
        let dir = std::env::temp_dir().join(format!("waverdi_hl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tb.sv");
        std::fs::write(
            &path,
            "module tb;\n    logic [7:0] data;\n    logic [7:0] other;\nendmodule\n",
        )
        .unwrap();
        app.sources = Some(SourceSet::from_files(vec![path], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.wf.as_mut().unwrap().tree.nodes[1].module = "tb".to_string();
        app.expanded.insert(1);
        app.touch_panes();
        app.tree_sel = 1;
        app.sync_source();
        app.add_signal(0);
        app.sel_row = None; // added rows are selected while focused
        let color = ratatui::style::Color::Rgb(0x5c, 0x1f, 0x1f);
        app.set_highlight(0, Some(color));
        assert_eq!(app.highlight_of(0), Some(color));

        // Renders the app and counts cells with `color` in one pane.
        let count_bg = |app: &mut App,
                        pick: fn(&Layout) -> ratatui::layout::Rect,
                        color: ratatui::style::Color|
         -> usize {
            let backend = TestBackend::new(100, 40);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|f| super::render(f, app)).unwrap();
            let buffer = terminal.backend().buffer();
            let rect = pick(&app.layout());
            let mut count = 0usize;
            for y in rect.y..rect.bottom() {
                for x in rect.x..rect.right() {
                    if buffer.cell((x, y)).map(|cell| cell.bg) == Some(color) {
                        count += 1;
                    }
                }
            }
            count
        };
        assert!(
            count_bg(&mut app, |l| l.list, color) > 0,
            "Signal List name not highlighted"
        );
        assert!(
            count_bg(&mut app, |l| l.rows, color) > 0,
            "waveform row not highlighted"
        );
        assert!(
            count_bg(&mut app, |l| l.source, color) > 0,
            "Source name not highlighted"
        );

        // A multi-name source selection highlights every selected signal and
        // the colour belongs to the signal, so it shows when added later.
        app.set_highlight(0, None);
        app.clear_all();
        let source_color = ratatui::style::Color::Rgb(0x1f, 0x4d, 0x4d);
        let (line_a, col_a, line_b, col_b) = {
            let view = app.source_view.as_ref().unwrap();
            let line_a = 1usize;
            let line_b = 2usize;
            (
                line_a,
                view.lines[line_a].find("data").unwrap(),
                line_b,
                view.lines[line_b].find("other").unwrap() + "other".len(),
            )
        };
        app.set_source_cursor(line_a, col_a);
        app.begin_source_selection();
        app.extend_source_selection_to(line_b, col_b);
        app.finish_source_selection();
        app.highlight_source_word(Some(source_color));
        assert_eq!(app.highlight_of(0), Some(source_color));
        assert_eq!(app.highlight_of(1), Some(source_color));
        assert!(app.highlighted_source_names().contains_key("data"));
        assert!(app.highlighted_source_names().contains_key("other"));
        assert!(count_bg(&mut app, |l| l.source, source_color) > 0);
        assert_eq!(count_bg(&mut app, |l| l.rows, source_color), 0);
        app.add_signal(0);
        app.add_signal(1);
        app.sel_row = None;
        assert!(count_bg(&mut app, |l| l.list, source_color) > 0);
        assert!(count_bg(&mut app, |l| l.rows, source_color) > 0);

        // Only an explicit clear removes the highlight.
        app.highlight_source_word(None);
        assert!(!app.highlighted_source_names().contains_key("data"));
        assert!(!app.highlighted_source_names().contains_key("other"));
        assert_eq!(app.highlight_of(0), None);
        assert_eq!(app.highlight_of(1), None);
        assert_eq!(count_bg(&mut app, |l| l.source, source_color), 0);
        assert_eq!(count_bg(&mut app, |l| l.list, source_color), 0);
        assert_eq!(count_bg(&mut app, |l| l.rows, source_color), 0);

        // Clearing from the Signal List also removes the Source highlight of
        // that signal, but keeps the other selected signal highlighted.
        app.highlight_source_word(Some(source_color));
        app.set_highlight_many(&[0], None);
        assert_eq!(app.highlight_of(0), None);
        assert_eq!(app.highlight_of(1), Some(source_color));
        assert!(!app.highlighted_source_names().contains_key("data"));
        assert!(app.highlighted_source_names().contains_key("other"));
        app.set_highlight_many(&[1], None);
        assert!(!app.highlighted_source_names().contains_key("other"));
    }

    #[test]
    fn zoomed_out_view_shows_a_narrow_pulse() {
        // A single 1ps high pulse in a 1s range must stay visible.
        let vcd = "$timescale 1ps $end\n\
            $var wire 1 ! sig $end\n\
            $enddefinitions $end\n\
            #0\n0!\n#1000000\n1!\n#1000001\n0!\n#1000000000000\n0!\n";
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0]);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 30));
        // Keep the cursor line away from the pulse column under test.
        let span = app.wf.as_ref().unwrap().total_ticks();
        app.cursor = app.wf.as_ref().unwrap().start + span * 7 / 10;

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let l = app.layout();
        let row = app
            .list_rows()
            .iter()
            .position(|row| matches!(row, crate::app::ListRow::Signal { sig: 0, .. }))
            .unwrap();
        let y = l.rows.y + row as u16;
        let pulse_col = app.x_at_tick(1_000_000) as u16;
        let cell = buffer.cell((l.rows.x + pulse_col, y)).unwrap();
        assert_eq!(
            cell.symbol(),
            "│",
            "pulse marker missing: {} fg={:?}",
            cell.symbol().escape_unicode(),
            cell.fg
        );
        // The surrounding low level is still drawn normally.
        let quiet = buffer.cell((l.rows.x + pulse_col + 10, y)).unwrap();
        assert_eq!(quiet.symbol(), "▁");
    }

    #[test]
    fn bus_labels_survive_narrow_segments() {
        // `h1` is too narrow for a label; `h2` right after it is wide and
        // must still be labelled.
        let vcd = "$timescale 1ns $end\n\
            $var reg 4 ! data $end\n\
            $enddefinitions $end\n\
            #0\nh1 !\n#1\nh2 !\n#100\nh3 !\n#1000\nh4 !\n";
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0]);
        let screen = render_app(&mut app, 120, 30);
        assert!(
            screen.contains("h2"),
            "missing value after narrow segment:\n{screen}"
        );
        assert!(screen.contains("h3"), "{screen}");
    }

    #[test]
    fn add_signals_dialog_renders() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.open_add_signals();
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains("Add Signals"), "{screen}");
        assert!(screen.contains("Apply"), "{screen}");
        assert!(screen.contains("OK"), "{screen}");
        assert!(screen.contains("Cancel"), "{screen}");
        assert!(screen.contains("Filter: all"), "{screen}");
    }

    #[test]
    fn add_dialog_click_selects_the_clicked_signal_in_nested_scopes() {
        let vcd = "$timescale 1ns $end\n\
            $scope module top $end\n\
            $var wire 1 ! a $end\n\
            $var wire 1 \" b $end\n\
            $scope module sub $end\n\
            $var wire 1 # c $end\n\
            $var wire 1 $ d $end\n\
            $upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\n0!\n0\"\n0#\n0$\n";
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 120, 30));
        app.open_add_signals();
        let sub = {
            let wf = app.wf.as_ref().unwrap();
            let top = wf.tree.nodes[wf.tree.root].children[0];
            wf.tree.nodes[top].children[0]
        };
        app.add_navigate(sub);
        assert_eq!(app.add_signal_list(), vec![2, 3]);
        let screen = app.last_area;
        let l = crate::ui::add::layout(screen);
        let mut clicked = false;
        for col in l.signals.x..l.signals.right() {
            if let Some(crate::ui::add::AddHit::Signal(signal)) =
                crate::ui::add::hit(screen, &app, col, l.signals.y)
            {
                if signal == 3 {
                    crate::app::handle_mouse(&mut app, mouse_at(col, l.signals.y));
                    clicked = true;
                    break;
                }
            }
        }
        assert!(clicked, "signal cell not found");
        let selected = &app.add_signals.as_ref().unwrap().selected;
        assert!(selected.contains(&3), "{selected:?}");
        assert!(
            !selected.contains(&0) && !selected.contains(&1),
            "{selected:?}"
        );
    }

    #[test]
    fn add_dialog_panes_scroll_independently() {
        use crate::app::AddFocus;
        let mut vcd = String::from("$timescale 1ns $end\n$scope module top $end\n");
        for i in 0..40u8 {
            vcd.push_str(&format!(
                "$var wire 1 {} s{:02} $end\n",
                (33 + i) as char,
                i
            ));
        }
        vcd.push_str("$upscope $end\n$enddefinitions $end\n#0\n0!\n");
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 60, 20));
        app.open_add_signals();
        let top = {
            let wf = app.wf.as_ref().unwrap();
            wf.tree.nodes[wf.tree.root].children[0]
        };
        app.add_navigate(top);
        let cols = crate::ui::add::pane_cols(app.last_area, &app, AddFocus::Signals);
        let visible = crate::ui::add::pane_visible(app.last_area, &app, AddFocus::Signals);
        assert!(cols > 0);
        assert!(app.add_pane_len(AddFocus::Signals) > visible);

        let l = crate::ui::add::layout(app.last_area);
        // Wheel over the signals pane scrolls that pane only.
        crate::app::handle_mouse(&mut app, wheel_at(l.signals.x + 1, l.signals.y, false));
        assert_eq!(app.add_scroll_of(AddFocus::Signals), 3 * cols);
        assert_eq!(app.add_scroll_of(AddFocus::Tree), 0);

        // Wheel over the tree does not disturb the signals pane.
        crate::app::handle_mouse(&mut app, wheel_at(l.tree.x + 1, l.tree.y, false));
        assert_eq!(app.add_scroll_of(AddFocus::Signals), 3 * cols);

        // Dragging the signals scrollbar to the bottom shows the last page.
        crate::app::handle_mouse(
            &mut app,
            mouse_at(l.signals.right() - 1, l.signals.bottom() - 1),
        );
        let len = app.add_pane_len(AddFocus::Signals);
        assert_eq!(app.add_scroll_of(AddFocus::Signals), len - visible);
    }

    fn pane_color(app: &mut App, x: u16, y: u16) -> ratatui::style::Color {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, app)).unwrap();
        terminal.backend().buffer().cell((x, y)).unwrap().fg
    }

    #[test]
    fn focused_pane_is_highlighted() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 30));
        app.set_display(vec![0]);
        let l = app.layout();
        app.focus = crate::app::Focus::List;
        assert_eq!(
            pane_color(&mut app, l.list.x + 1, l.list.y),
            crate::theme::Theme::DARK.accent
        );
        app.focus = crate::app::Focus::Wave;
        assert_eq!(
            pane_color(&mut app, l.nwave.x + 1, l.nwave.y),
            crate::theme::Theme::DARK.accent
        );
        app.focus = crate::app::Focus::Tree;
        assert_eq!(
            pane_color(&mut app, l.tree.x, l.tree.y),
            crate::theme::Theme::DARK.accent
        );
    }

    #[test]
    fn ruler_labels_are_clipped_at_the_divider() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0]);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 30));
        app.time_base = crate::waveform::TimeBase::Ps;
        app.t0 = 1000.0;
        app.scale = 1.0;
        let screen = render_app(&mut app, 100, 30);
        let l = app.layout();
        let lines: Vec<Vec<char>> = screen.lines().map(|line| line.chars().collect()).collect();
        let ruler = &lines[l.ruler.y as usize];
        // The divider column stays intact; the label is clipped with `<`.
        assert_eq!(ruler[l.list_grip_x() as usize], '│');
        assert_eq!(ruler[l.wave.x as usize], '<');
        let text: String = ruler.iter().collect();
        assert!(text.contains("<000ps"), "{screen}");
    }

    #[test]
    fn mixed_theme_keeps_the_waveform_dark() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 30));
        app.set_display(vec![0]);
        app.set_theme_kind(crate::theme::ThemeKind::Mixed);
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let l = app.layout();
        // Dark waveform canvas, light Signal List header.
        assert_eq!(
            buffer.cell((l.wave.x + 1, l.wave.bottom() - 1)).unwrap().bg,
            crate::theme::Theme::DARK.bg
        );
        assert_eq!(
            buffer.cell((l.list.x + 1, l.list.y)).unwrap().bg,
            crate::theme::Theme::LIGHT.list_header_bg
        );
        // Other panes keep light backgrounds: the Signal List uses the light
        // alternate row colour while the waveform rows stay dark.
        assert_eq!(
            buffer.cell((l.list.x + 2, l.list.y + 2)).unwrap().bg,
            crate::theme::Theme::LIGHT.row_alt
        );
        assert_eq!(
            buffer.cell((l.rows.x + 2, l.rows.y)).unwrap().bg,
            crate::theme::Theme::DARK.bg
        );
    }

    #[test]
    fn values_never_cover_the_list_dividers() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 30));
        app.set_display(vec![1]);
        app.splits.value_pct = crate::ui::layout::Splits::MIN_VALUE_PCT;
        app.cursor = 5; // data 0 -> 5: the Value cell shows the transition
        let screen = render_app(&mut app, 100, 30);
        let l = app.layout();
        let lines: Vec<Vec<char>> = screen.lines().map(|line| line.chars().collect()).collect();
        let row = &lines[(l.list.y + 3) as usize]; // G0 header then the data row
        assert_eq!(row[l.list_grip_x() as usize], '│');
        assert_eq!(row[l.value_grip_x(app.splits.value_pct) as usize], '│');
    }

    #[test]
    fn light_theme_dialog_uses_a_light_background() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 30));
        app.set_theme_kind(crate::theme::ThemeKind::Light);
        app.open_dialog(crate::app::Dialog::Keys);
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let area = super::dialog::dialog_rect(app.last_area, &app, crate::app::Dialog::Keys);
        assert_eq!(
            buffer.cell((area.x + 2, area.y + 2)).unwrap().bg,
            crate::theme::Theme::LIGHT.popup_bg
        );
    }

    #[test]
    fn dialog_overlay_preserves_pane_backgrounds() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 30));
        app.set_display(vec![0]);
        app.set_theme_kind(crate::theme::ThemeKind::Mixed);
        app.open_dialog(crate::app::Dialog::Keys);
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let l = app.layout();
        let lum = |c: ratatui::style::Color| match c {
            ratatui::style::Color::Rgb(r, g, b) => r as u32 + g as u32 + b as u32,
            _ => 0,
        };
        // Outside the dialog the waveform stays dark and the list stays light:
        // the overlay dims each pane instead of repainting one theme colour.
        let wave = buffer.cell((l.wave.right() - 2, l.wave.y + 2)).unwrap().bg;
        let list = buffer.cell((l.list.x + 1, l.list.y)).unwrap().bg;
        assert!(lum(wave) < lum(list), "wave {wave:?} vs list {list:?}");
        assert_ne!(wave, crate::theme::Theme::LIGHT.popup_bg);
        assert!(lum(wave) < lum(crate::theme::Theme::DARK.bg) + 32);
    }

    #[test]
    fn instance_pane_shows_hierarchy_and_module_columns() {
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $scope module dut $end\n\
            $var wire 1 ! clk $end\n\
            $upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.wf.as_mut().unwrap().tree.nodes[2].module = "counter".to_string();
        app.expanded.insert(1);
        app.touch_panes();
        let screen = render_app(&mut app, 100, 30);
        assert!(screen.contains("Hierarchy"), "{screen}");
        assert!(screen.contains("Module"), "{screen}");
        assert!(screen.contains("counter"), "{screen}");
    }

    #[test]
    fn source_selection_highlights_signal_names() {
        use crate::rtl::{RtlDb, SourceSet};
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $scope module dut $end\n\
            $var wire 4 ! count [3:0] $end\n\
            $upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\nb0000 !\n";
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 40));
        let dir = std::env::temp_dir().join(format!("waverdi_src_hl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("counter.sv");
        std::fs::write(
            &path,
            "module counter(output logic [3:0] count);\n\
             logic [3:0] next;\n\
             assign count = next;\n\
             endmodule\n",
        )
        .unwrap();
        app.sources = Some(SourceSet::from_files(vec![path], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.wf.as_mut().unwrap().tree.nodes[2].module = "counter".to_string();
        app.expanded.insert(1);
        app.touch_panes();
        app.tree_sel = 2;
        app.sync_source();
        let col = {
            let view = app.source_view.as_ref().unwrap();
            view.lines[2].find("count").unwrap()
        };
        app.set_source_cursor(2, col);
        app.select_source_word();
        app.set_source_cursor(2, 0); // move the cursor off the selected word
        app.focus = crate::app::Focus::Source;

        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let l = app.layout();
        let rect = super::source::code_rect(&l);
        let view = app.source_view.as_ref().unwrap();
        let gutter = super::source::gutter_width(view);
        let count_cell = buffer
            .cell((rect.x + gutter + col as u16, rect.y + 2))
            .unwrap();
        assert_eq!(count_cell.bg, crate::theme::Theme::DARK.src_signal);
        // The neighbouring signal on the same line is not selected.
        let next_col = view.lines[2].find("next").unwrap();
        let next_cell = buffer
            .cell((rect.x + gutter + next_col as u16, rect.y + 2))
            .unwrap();
        assert_ne!(next_cell.bg, crate::theme::Theme::DARK.src_signal);
    }

    #[test]
    fn source_pane_renders_tabs_as_spaces() {
        use crate::rtl::{RtlDb, SourceSet};
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $scope module dut $end\n\
            $var wire 4 ! count [3:0] $end\n\
            $upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\nb0000 !\n";
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 40));
        let dir = std::env::temp_dir().join(format!("waverdi_src_tab_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("counter.sv");
        std::fs::write(
            &path,
            "module counter(input logic clk);\n\tlogic\t[3:0] count;\nendmodule\n",
        )
        .unwrap();
        app.sources = Some(SourceSet::from_files(vec![path], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.wf.as_mut().unwrap().tree.nodes[2].module = "counter".to_string();
        app.expanded.insert(1);
        app.touch_panes();
        app.tree_sel = 2;
        app.sync_source();
        let (expanded, scroll, count_col, gutter) = {
            let view = app.source_view.as_ref().unwrap();
            assert_eq!(view.lines[1], "    logic   [3:0] count;");
            (
                view.lines[1].clone(),
                view.scroll,
                view.lines[1].find("count").unwrap() as u16,
                super::source::gutter_width(view),
            )
        };

        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let l = app.layout();
        let rect = super::source::code_rect(&l);
        let row = rect.y + (1 - scroll) as u16;
        for x in rect.x..rect.right() {
            let symbol = buffer
                .cell((x, row))
                .map(|cell| cell.symbol())
                .unwrap_or("");
            assert_ne!(symbol, "\t", "tab cell at column {x}");
        }
        assert_eq!(expanded.find("count"), Some(count_col as usize));
        let cell = buffer.cell((rect.x + gutter + count_col, row)).unwrap();
        assert_eq!(cell.symbol(), "c");
    }

    #[test]
    fn dialogs_render() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        for dialog in [
            crate::app::Dialog::Goto,
            crate::app::Dialog::Find,
            crate::app::Dialog::FindValue,
            crate::app::Dialog::SplitBus,
            crate::app::Dialog::CreateBus,
            crate::app::Dialog::GroupName,
            crate::app::Dialog::Settings,
            crate::app::Dialog::Keys,
            crate::app::Dialog::About,
        ] {
            app.dialog = Some(dialog);
            let _ = render_app(&mut app, 100, 30);
        }
    }

    #[test]
    fn dialogs_render_on_small_screens() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        for (w, h) in [(40u16, 12u16), (30, 8), (24, 6)] {
            for dialog in [
                crate::app::Dialog::Goto,
                crate::app::Dialog::Find,
                crate::app::Dialog::FindValue,
                crate::app::Dialog::SplitBus,
                crate::app::Dialog::CreateBus,
                crate::app::Dialog::GroupName,
                crate::app::Dialog::Keys,
                crate::app::Dialog::About,
            ] {
                app.open_dialog(dialog);
                let _ = render_app(&mut app, w, h);
            }
        }
    }

    #[test]
    fn keys_dialog_scrolls_to_later_sections() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0, 1]);
        app.open_dialog(crate::app::Dialog::Keys);
        let first = render_app(&mut app, 60, 14);
        assert!(!first.contains("Mouse"), "{first}");
        for _ in 0..10 {
            crate::app::handle_key(
                &mut app,
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::PageDown,
                    crossterm::event::KeyModifiers::NONE,
                ),
            );
        }
        let later = render_app(&mut app, 60, 14);
        assert!(later.contains("dialogs: ✕ closes"), "{later}");
    }

    #[test]
    fn tui_browser_dialog_renders() {
        let mut app = App::new();
        app.use_gui = false;
        app.open_tui_browser();
        // The working directory holds more entries than fit on screen: scroll
        // the browser to Cargo.toml before rendering.
        {
            let rows = super::dialog::browser_rows(ratatui::layout::Rect::new(0, 0, 100, 30));
            let browser = app.browser.as_mut().unwrap();
            let index = browser
                .entries
                .iter()
                .position(|entry| entry.name == "Cargo.toml")
                .expect("Cargo.toml in the crate root");
            browser.sel = index;
            browser.scroll_to_sel(rows);
        }
        let screen = render_app(&mut app, 100, 30);
        assert!(screen.contains("Open Waveform"), "{screen}");
        assert!(screen.contains("Enter open"), "{screen}");
        assert!(screen.contains("Cargo.toml"), "{screen}");
    }

    #[test]
    fn context_menu_highlights_the_whole_row() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 1 ! clk $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0]);
        app.open_context_menu(crate::app::CtxTarget::Signal(0), 5, 5);
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let l = app.layout();
        let root = super::context::root_rect(&l, &app).expect("context menu open");
        // The selected (first) entry fills the inner width.
        let y = root.y + 1;
        let left = buffer.cell((root.x + 1, y)).unwrap();
        let right = buffer.cell((root.right() - 2, y)).unwrap();
        assert_eq!(left.bg, crate::theme::Theme::DARK.accent);
        assert_eq!(right.bg, crate::theme::Theme::DARK.accent);
        // No name/title on the top border any more.
        let top: String = (root.x..root.right())
            .map(|x| buffer.cell((x, root.y)).unwrap().symbol())
            .collect();
        assert!(!top.contains("clk"), "{top}");
    }

    #[test]
    fn menu_dropdown_renders() {
        let mut app = App::new();
        app.menu.open = Some(0);
        let screen = render_app(&mut app, 80, 20);
        assert!(screen.contains("Open Waveform..."));
        app.menu.open = Some(4);
        let screen = render_app(&mut app, 80, 20);
        assert!(screen.contains("Key Bindings"));
    }

    #[test]
    fn signal_list_right_aligns_names_and_keeps_tails() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 1 ! a_very_long_signal_name $end\n\
            $var wire 1 \" clk $end\n\
            $enddefinitions $end\n#0\n0!\n0\"\n";
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0, 1]);
        let backend = TestBackend::new(60, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let l = app.layout();
        let grip = l.value_grip_x(app.splits.value_pct);
        let row = |y: u16| -> String {
            (l.list.x..grip)
                .map(|x| buffer.cell((x, y)).map(|cell| cell.symbol()).unwrap_or(" "))
                .collect()
        };
        let long = row(l.list.y + 3);
        assert!(long.contains('…'), "{long}");
        assert!(long.trim_end().ends_with("name"), "{long}");
        let short = row(l.list.y + 4);
        assert_eq!(short.trim(), "clk");
        // The horizontal scrollbar is drawn on the pane's bottom row.
        let bar = row(l.list.bottom() - 1);
        assert!(bar.contains('█'), "{bar}");
        assert!(bar.contains('─'), "{bar}");
        // Right-aligned: the name sits next to the value divider, not at the
        // left edge of the column.
        assert!(short.starts_with(' '), "{short:?}");
    }

    #[test]
    fn status_bar_keeps_the_file_name_visible() {
        let mut app = App::new();
        app.path = "/home/some/very/long/path/to/the/waveform_dump.fsdb".to_string();
        let screen = render_app(&mut app, 100, 20);
        assert!(screen.contains("waveform_dump.fsdb"), "{screen}");
    }

    #[test]
    fn source_scrollbar_is_drawn_for_long_files() {
        use crate::rtl::{RtlDb, SourceSet};
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $var wire 1 ! clk $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        let dir = std::env::temp_dir().join(format!("waverdi_src_bar_ui_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("long.sv");
        let mut text = String::from("module long(input logic clk);\n");
        for line in 0..80 {
            text.push_str(&format!("    assign w{line} = clk;\n"));
        }
        text.push_str("endmodule\n");
        std::fs::write(&path, text).unwrap();
        app.sources = Some(SourceSet::from_files(vec![path], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.wf.as_mut().unwrap().tree.nodes[1].module = "long".to_string();
        app.tree_sel = 1;
        app.sync_source();

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let l = app.layout();
        let code = super::source::code_rect(&l);
        let col = super::source::scrollbar_col(&l, app.source_view.as_ref().unwrap())
            .expect("scrollbar column");
        let bar: String = (code.y..code.bottom())
            .map(|y| {
                buffer
                    .cell((col, y))
                    .map(|cell| cell.symbol())
                    .unwrap_or(" ")
            })
            .collect();
        assert!(bar.contains('│'), "{bar}");
        assert!(bar.contains('█'), "{bar}");
        // The last column of code is kept free for the bar.
        assert_eq!(col, code.right() - 1);
    }

    #[test]
    fn inactive_generate_branches_render_dimmed() {
        use crate::rtl::{RtlDb, SourceSet};
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $var wire 1 ! clk $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        let dir = std::env::temp_dir().join(format!("waverdi_inactive_ui_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let text = r#"module gen_top(input logic clk);
    generate
        if (0) begin : off
            assign dead = clk;
        end else begin : on
            assign live = clk;
        end
    endgenerate
endmodule
"#;
        let path = dir.join("gen_top.sv");
        std::fs::write(&path, text).unwrap();
        app.sources = Some(SourceSet::from_files(vec![path], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.wf.as_mut().unwrap().tree.nodes[1].module = "gen_top".to_string();
        app.tree_sel = 1;
        app.sync_source();

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let l = app.layout();
        let code = super::source::code_rect(&l);
        let view = app.source_view.as_ref().unwrap();
        let dead = view
            .lines
            .iter()
            .position(|line| line.contains("assign dead"))
            .unwrap();
        let live = view
            .lines
            .iter()
            .position(|line| line.contains("assign live"))
            .unwrap();
        let line_fg = |line: usize| {
            let y = code.y + (line - view.scroll) as u16;
            let x = code.x + super::source::gutter_width(view);
            buffer.cell((x, y)).unwrap().fg
        };
        assert_eq!(line_fg(dead), crate::theme::Theme::DARK.dim);
        assert_ne!(line_fg(live), crate::theme::Theme::DARK.dim);
    }

    #[test]
    fn signal_list_dims_the_hierarchy_prefix() {
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $scope module dut $end\n\
            $var wire 1 ! clk $end\n\
            $upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.set_display(vec![0]);
        app.show_full_names = true;
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let l = app.layout();
        let grip = l.value_grip_x(app.splits.value_pct);
        let y = l.list.y + 3; // G0 header, the signal row
        let cell_fg = |symbol: &str| {
            (l.list.x..grip)
                .map(|x| buffer.cell((x, y)).unwrap())
                .find(|cell| cell.symbol() == symbol)
                .map(|cell| cell.fg)
        };
        // `tb.dut.clk`: the path is dim, the name keeps the signal colour.
        assert_eq!(cell_fg("t"), Some(crate::theme::Theme::DARK.dim));
        assert_eq!(cell_fg("d"), Some(crate::theme::Theme::DARK.dim));
        assert_eq!(cell_fg("k"), Some(crate::theme::Theme::DARK.name));
    }

    #[test]
    fn instance_pane_has_bottom_scrollbars_and_a_full_divider() {
        let mut vcd = String::from("$timescale 1ns $end\n");
        for (level, name) in ["a", "b", "c", "d", "e"].iter().enumerate() {
            vcd.push_str(&format!("$scope module {name}{level} $end\n"));
        }
        for _ in 0..5 {
            vcd.push_str("$upscope $end\n");
        }
        vcd.push_str("$enddefinitions $end\n#0\n");
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        let nodes = app.wf.as_ref().unwrap().tree.nodes.len();
        app.expanded.extend(0..nodes);
        app.touch_panes();
        for id in 1..nodes {
            app.wf.as_mut().unwrap().tree.nodes[id].module = "counter_pipeline_stage".to_string();
        }
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let l = app.layout();
        let inner = super::layout::tree_inner(&l);
        let (_, sep_x, _) = super::tree::columns(inner, app.splits.hier_pct);
        // The divider reaches the bottom row of the pane.
        assert_eq!(
            buffer.cell((sep_x, inner.bottom() - 1)).unwrap().symbol(),
            "│"
        );
        let bar: String = (inner.x..inner.right())
            .map(|x| {
                buffer
                    .cell((x, inner.bottom() - 1))
                    .unwrap()
                    .symbol()
                    .to_string()
            })
            .collect();
        assert!(bar.contains('█'), "{bar}");
        assert!(bar.contains('─'), "{bar}");
    }

    #[test]
    fn source_pane_title_names_the_instance_and_file() {
        use crate::rtl::{RtlDb, SourceSet};
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $scope module dut $end\n\
            $var wire 1 ! clk $end\n\
            $upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let out = vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        let dir = std::env::temp_dir().join(format!("waverdi_src_title_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("counter.sv");
        std::fs::write(
            &path,
            "module counter(input logic clk);\n\
             logic [7:0] next;\n\
             assign next = 8'h00;\n\
             endmodule\n",
        )
        .unwrap();
        app.sources = Some(SourceSet::from_files(vec![path.clone()], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.wf.as_mut().unwrap().tree.nodes[2].module = "counter".to_string();
        app.expanded.insert(1);
        app.touch_panes();
        app.tree_sel = 2;
        app.sync_source();
        // The old header/footer lines are logged instead of drawn.
        assert!(app
            .messages
            .last()
            .unwrap()
            .contains("source: tb.dut [module counter]"));

        let title = app.source_title().expect("title");
        assert!(title.starts_with("Source - tb.dut("), "{title}");
        assert!(title.ends_with("counter.sv)"), "{title}");
        let screen = render_app(&mut app, 100, 30);
        assert!(screen.contains("Source - tb.dut("), "{screen}");
        // The code area is the whole inner rect: no header/footer rows.
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 30));
        let l = app.layout();
        assert_eq!(
            super::source::code_rect(&l).height,
            l.source.height.saturating_sub(2)
        );
    }
}
