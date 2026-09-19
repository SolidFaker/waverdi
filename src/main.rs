mod app;
mod dump;
mod fst;
mod picker;
mod rtl;
mod session;
mod theme;
mod ui;
mod vcd;
mod waveform;

#[cfg(fsdb_sdk)]
mod fsdb;

use app::App;
use clap::Parser;
use crossterm::event::{Event, MouseEventKind};
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use waveform::Waveform;

#[derive(Parser)]
#[command(
    name = "waverdi",
    version,
    about = "Verdi-style terminal RTL waveform viewer (VCD)"
)]
struct Cli {
    #[arg(help = "VCD file to open")]
    file: Option<PathBuf>,
    #[arg(
        short = 'f',
        long = "filelist",
        value_name = "FILELIST",
        help = "RTL filelist (VCS format)"
    )]
    filelists: Vec<PathBuf>,
    #[arg(short, long, help = "list signals and exit")]
    list_signals: bool,
    #[arg(long, help = "force the native GUI file dialog")]
    gui: bool,
    #[arg(
        long,
        conflicts_with = "gui",
        help = "disable GUI dialogs and use the built-in TUI browser"
    )]
    no_gui: bool,
}

/// `waverdi --list-signals file.vcd`: hierarchical text dump of the design.
fn print_tree(wf: &Waveform) {
    fn rec(wf: &Waveform, id: usize, depth: usize) {
        let node = &wf.tree.nodes[id];
        let module = if node.module.is_empty() || node.module == node.name {
            String::new()
        } else {
            format!("  ({})", node.module)
        };
        println!("{}{}{}", "  ".repeat(depth), node.name, module);
        for &sig in &node.signals {
            let signal = &wf.signals[sig];
            println!(
                "{}  {} ({}, {} bit)",
                "  ".repeat(depth + 1),
                signal.name,
                signal.var_type,
                signal.bits
            );
        }
        for &child in &node.children {
            rec(wf, child, depth + 1);
        }
    }
    rec(wf, wf.tree.root, 0);
    println!("\n{}", wf.summary());
}

fn main() {
    let cli = Cli::parse();
    let mut app = App::new();
    if cli.gui {
        app.use_gui = cfg!(feature = "gui");
    } else if cli.no_gui {
        app.use_gui = false;
    }

    if let Some(file) = &cli.file {
        if cli.list_signals {
            match dump::parse(file) {
                Ok(out) => {
                    print_tree(&out.wf);
                    return;
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        app.start_load(&file.display().to_string());
    } else if cli.list_signals {
        eprintln!("waverdi: --list-signals requires a VCD file");
        std::process::exit(1);
    }

    // A filelist passed on the command line wins over dump auto-discovery.
    for list in &cli.filelists {
        app.load_filelist(&list.display().to_string());
    }

    if let Err(e) = run_tui(app) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run_tui(mut app: App) -> io::Result<()> {
    let mut term = ratatui::init();
    let result = run_loop(&mut term, &mut app);
    let _ = crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture);
    ratatui::restore();
    result
}

fn run_loop(term: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<()> {
    crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)?;
    loop {
        let loading = app.load.as_ref().is_some_and(|job| !job.finished);
        // Idle frames are skipped: the dirty flag is set by every input event,
        // by loader events and by the idle tick when it changes state.
        if app.needs_redraw || loading || app.is_dragging() {
            // Draw also resizes the terminal buffer after a Resize event.
            term.draw(|f| ui::render(f, app))?;
            app.take_needs_redraw();
        }
        // Poll faster while something is moving (load progress, drag, scroll
        // auto-repeat) and slower when the app is fully idle.
        let timeout = if loading || app.is_dragging() {
            50
        } else {
            250
        };
        if !crossterm::event::poll(Duration::from_millis(timeout))? {
            app::tick(app);
            continue;
        }
        let quit = match crossterm::event::read()? {
            Event::Key(k) => app::handle_key(app, k),
            Event::Mouse(m) => {
                // 1003h reports every pointer move; without a drag there is
                // nothing to update, so those events must not force a frame.
                if m.kind == MouseEventKind::Moved && !app.is_dragging() {
                    false
                } else {
                    app::handle_mouse(app, m)
                }
            }
            Event::Resize(..) => {
                app.needs_redraw = true;
                false
            }
            _ => false,
        };
        if quit {
            break;
        }
    }
    Ok(())
}
