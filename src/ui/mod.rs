pub mod context;
pub mod dialog;
pub mod layout;
pub mod list;
pub mod menubar;
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
        source::draw_frame(buf, &l, t);
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
        if app.wf.is_some() {
            wave::draw_scrollbars(buf, &l, app);
        }
        if let Some(idx) = app.menu.open {
            menubar::draw_dropdown(buf, &l, app, idx);
        }
        context::draw(buf, &l, app);
        toolbar::draw_time_menu(buf, &l, app);
    }

    if let Some(dialog) = app.dialog {
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
        // Only the Instance pane still shows the signal name.
        assert_eq!(screen.matches("clk").count(), 1, "{screen}");
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
        app.open_ctx_submenu(0);
        let screen = render_app(&mut app, 100, 30);
        assert!(screen.contains("Hex"), "{screen}");
        assert!(screen.contains("ASCII"), "{screen}");
        app.close_ctx_submenu();
        app.open_ctx_submenu(2);
        let screen = render_app(&mut app, 100, 30);
        assert!(screen.contains("Split Bus..."), "{screen}");
        assert!(screen.contains("Create Bus..."), "{screen}");
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
            crate::app::Dialog::Filelist,
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
