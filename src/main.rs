mod app;
mod dump;
mod fst;
mod picker;
mod theme;
mod ui;
mod vcd;
mod waveform;

#[cfg(fsdb_sdk)]
mod fsdb;

use app::App;
use clap::Parser;
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
        match dump::parse(file) {
            Ok(out) => {
                if cli.list_signals {
                    print_tree(&out.wf);
                    return;
                }
                let path = file.display().to_string();
                app.apply_parsed(path.clone(), out);
                app::set_title(&path);
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    } else if cli.list_signals {
        eprintln!("waverdi: --list-signals requires a VCD file");
        std::process::exit(1);
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
        term.draw(|f| ui::render(f, app))?;
        if !crossterm::event::poll(Duration::from_millis(50))? {
            continue;
        }
        let quit = match crossterm::event::read()? {
            crossterm::event::Event::Key(k) => app::handle_key(app, k),
            crossterm::event::Event::Mouse(m) => app::handle_mouse(app, m),
            _ => false,
        };
        if quit {
            break;
        }
    }
    Ok(())
}
