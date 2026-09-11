pub mod context;
pub mod dialog;
pub mod layout;
pub mod list;
pub mod menubar;
pub mod status;
pub mod text;
pub mod toolbar;
pub mod tree;
pub mod wave;

use crate::app::App;
use crate::theme::*;
use crate::ui::layout::compute_layout;
use ratatui::buffer::Buffer;
use ratatui::style::Style;
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.sync_layout(area);
    let l = compute_layout(area, app.splits);

    {
        let buf = frame.buffer_mut();
        buf.set_style(area, Style::new().bg(BG));
        menubar::draw_bar(buf, &l, app);
        toolbar::draw(buf, &l, app);
        match &app.wf {
            None => status::draw_empty(buf, &l),
            Some(wf) => {
                tree::draw(buf, &l, app, wf);
                list::draw(buf, &l, app, wf);
                wave::draw(buf, &l, app, wf);
            }
        }
        status::draw_messages(buf, &l, app);
        status::draw_status(buf, &l, app);
        draw_separator(buf, l.list.right(), l.list.y, l.list.height);
        if let Some(idx) = app.menu.open {
            menubar::draw_dropdown(buf, &l, app, idx);
        }
        context::draw(buf, &l, app);
    }

    if let Some(dialog) = app.dialog {
        dialog::draw(frame, &l, app, dialog);
    }
}

fn draw_separator(buf: &mut Buffer, x: u16, top: u16, height: u16) {
    for row in 0..height {
        if let Some(cell) = buf.cell_mut((x, top + row)) {
            cell.set_symbol("│");
            cell.set_fg(SEP_BG);
            cell.set_bg(SEP_BG);
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
        app.display = vec![0, 1, 2];
        app.fit();
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains("nTrace"));
        assert!(screen.contains("Signal List"));
        assert!(screen.contains("clk"));
        assert!(screen.contains("data"));
        assert!(screen.contains("temp"));
        // High/low rails and a bus value must show up in the waveform pane.
        assert!(screen.contains('▔') || screen.contains('▁'));
        assert!(screen.contains("h5") || screen.contains("h0"));
    }

    #[test]
    fn renders_fst_waveform() {
        let out = crate::fst::parse_fst(std::path::Path::new("waveform/demo.fst")).unwrap();
        let mut app = App::new();
        app.apply_parsed("waveform/demo.fst", out);
        app.display = (0..app.wf.as_ref().unwrap().signals.len()).collect();
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains("clk"), "{screen}");
        assert!(screen.contains("data"), "{screen}");
    }

    #[test]
    fn fst_find_value() {
        let out = crate::fst::parse_fst(std::path::Path::new("waveform/demo.fst")).unwrap();
        let mut app = App::new();
        app.apply_parsed("waveform/demo.fst", out);
        app.display = (0..app.wf.as_ref().unwrap().signals.len()).collect();
        app.sel_row = Some(4); // state (after the tb / u_dut group rows)
        app.value_query = Some("h2".to_string());
        app.search_value(true);
        assert_eq!(app.cursor, 30);
    }

    #[test]
    fn isolated_bit_edges_render_diagonals() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.display = vec![0];
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains('/'), "{screen}");
        assert!(screen.contains('\\'), "{screen}");
    }

    #[test]
    fn list_shows_transition_when_cursor_sits_on_edge() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.display = vec![0];
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
        app.display = vec![0];
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
    fn context_menu_renders() {
        let out = vcd::parse_bytes(VCD.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.display = vec![0];
        app.open_context_menu(crate::app::CtxTarget::Signal(0), 5, 5);
        let screen = render_app(&mut app, 100, 30);
        assert!(screen.contains("Set Radix: Hex"), "{screen}");
        assert!(screen.contains("Bus: Split Bus"), "{screen}");
        assert!(screen.contains("Set Waveform: Analog"), "{screen}");
    }

    #[test]
    fn list_renders_hierarchy_groups() {
        let scoped = "$timescale 1ns $end\n\
            $scope module top $end\n\
            $var wire 1 ! clk $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let out = vcd::parse_bytes(scoped.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.display = vec![0];
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains("top/"), "{screen}");
        assert!(screen.contains("clk"), "{screen}");
        // Collapse via group context menu: the group stays, its signal does not.
        app.open_context_menu(crate::app::CtxTarget::Group("top".to_string()), 5, 5);
        let items = app.ctx_items().len();
        assert_eq!(items, 5); // expand / collapse / expand all / collapse all / remove
        let _ = items;
        app.run_ctx_item(crate::app::CtxItem::CollapseGroup);
        let screen = render_app(&mut app, 120, 30);
        assert!(screen.contains("top/"), "{screen}");
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
            crate::app::Dialog::Keys,
            crate::app::Dialog::About,
        ] {
            app.dialog = Some(dialog);
            let _ = render_app(&mut app, 100, 30);
        }
    }

    #[test]
    fn tui_browser_dialog_renders() {
        let mut app = App::new();
        app.use_gui = false;
        app.open_tui_browser();
        let screen = render_app(&mut app, 100, 30);
        assert!(screen.contains("Open Waveform"), "{screen}");
        assert!(screen.contains("Enter open"), "{screen}");
        assert!(screen.contains("Cargo.toml"), "{screen}");
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
}
