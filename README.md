# waverdi

A Verdi-style RTL waveform viewer for the terminal, written in Rust.

waverdi is meant for SSH sessions and headless machines: open a VCD or FST
dump and browse signals, values and waveform traces without ever leaving the
terminal.

![waverdi screenshot](shot/waveform.png)

## Features

- **VCD parser** — scopes, vectors, `real`, `string`, `x`/`z`, every value
  base (`b`, `o`, `h`, `d`, `r`, `s`), `$dumpvars`, timescale handling.
- **FST support** — GTKWave's Fast Signal Trace format is read through the
  [wellen](https://github.com/ekiwi/wellen) library.
- **FSDB support** — on Linux, when `VERDI_HOME` points at a Verdi install,
  `.fsdb` files are read directly through Synopsys' FSDB Reader (FFR) via a
  small C++ bridge (`csrc/ffr_bridge.cpp`). Without the SDK, waverdi explains
  how to enable it or to convert the dump with `fsdb2vcd`.
- **Verdi-like layout** — menu bar on top; the upper 60% holds the `Instance`
  hierarchy browser (left) and an RTL source pane placeholder (right); the
  lower 40% is the merged `nWave` window with its shortcut bar, the `Signal
  List` and the waveform view (ruler, cursor and range markers).
- **Waveform rendering** — thin high/low rails, `/` rising and `\` falling
  edges (dense activity collapses to `│`), inline bus values, analog rendering
  for `real` and logic signals.
- **Signal List groups** — user-defined groups (G0 by default) hold the
  displayed signals; create / rename / remove them from the group context
  menu. Group boundaries are marked in the waveform pane; the names only show
  in the Signal List.
- **Signal operations** — radix (Hex/Binary/Octal/Decimal/ASCII), digital ⇄
  analog waveform, split a bus into bits, merge consecutive 1-bit signals into
  a bus, search for a value (`v` then `n`/`N`), cursor time shown on the ruler.
- **Full mouse support** — clickable menus/toolbar, draggable pane borders and
  scrollbars, drag to reorder signals, selection-to-zoom, wheel zoom,
  right-click context menus.
- **SSH friendly** — the native file dialog is disabled automatically over
  SSH (or when there is no display). A built-in terminal file browser takes
  its place, and a GUI-free build is available.

## Build

```sh
cargo build --release
```

On a headless/SSH-only server, build without the optional `rfd` GUI
dependency and the viewer always uses its built-in terminal file browser:

```sh
cargo build --release --no-default-features
```

### FSDB (Linux + Verdi)

Direct FSDB reading uses the proprietary FSDB Reader SDK that ships with
Verdi. Source the Synopsys environment before building so that `VERDI_HOME`
is set, then compile as usual:

```sh
source ~/synopsys/env.sh       # sets VERDI_HOME, VCS_HOME, ...
cargo build --release
```

The build script compiles `csrc/ffr_bridge.cpp` against
`$VERDI_HOME/share/FsdbReader/ffrAPI.h` and links `libnffr`/`libnsys`. On
machines without the SDK the build stays pure Rust and opening `.fsdb`
prints a hint instead. A cross-check against Verdi's own `fsdb2vcd`
converter is part of the test suite.

## Usage

```sh
waverdi waveform/counter.vcd                 # open a VCD dump
waverdi waveform/demo.fst                    # open an FST dump
waverdi wave.fsdb                            # open an FSDB dump (Verdi SDK build)
waverdi --list-signals waveform/complex.vcd  # print the hierarchy and exit
waverdi --no-gui waveform/counter.vcd        # force the built-in TUI file browser
waverdi --gui waveform/counter.vcd           # force the native file dialog
```

The `waveform/` folder contains demos: `counter.vcd` (4-bit counter with
enable/carry), `complex.vcd` (a small CPU/RAM/sensor design with 16 signals,
tristate and analog values) and `demo.fst` (a nested tb/u_dut design in FST).

### File dialog

The **Open** action uses the operating system dialog when a desktop is
available. It is skipped automatically when running over SSH
(`SSH_CONNECTION`, `SSH_CLIENT` or `SSH_TTY`) or when neither `DISPLAY` nor
`WAYLAND_DISPLAY` is set; the built-in browser is used instead. You can always
override with `--gui` / `--no-gui`, or open the browser directly with `O`.

## Keys

| Key | Action |
| --- | --- |
| `q`, `Ctrl+C/Q` | quit |
| `o` / `O` | open (system dialog / built-in TUI browser) |
| `:` | go to time (`1500`, `1.5us`, ...) |
| `g` | vim prefix: `ge` previous falling edge, `gg` first row |
| `G` | jump to the last row |
| `s` | search signals |
| `v` | find a value in the selected signal (dialog) |
| `n` / `N` | next / previous value match (wraps) |
| `z` / `Z` | zoom in / out around the cursor |
| `f` | fit the whole time range |
| `c` | center the cursor |
| `h` / `l` | move the cursor one column left / right |
| `0` / `$` | jump to the start / end of time |
| `←` / `→` | move the cursor (hold `Shift` for x10) |
| `j` / `k` | next / previous signal row |
| `J` / `K` | move the selected signal down / up (across groups) |
| `gg` | jump to the first row |
| `V` | visual mode: `j` / `k` extend the multi-selection |
| `dd` | cut the selected signal(s) into the register |
| `p` | paste the register below the current signal / into the current group |
| `Space` | toggle the row in the multi-selection |
| `Shift`+`↑` / `↓` | extend the selection from its anchor |
| `Esc` | clear the multi-selection / leave visual mode |
| `w` / `b` | next / previous edge (on 1-bit signals: next / previous rising edge) |
| `e` / `ge` | next / previous falling edge (1-bit signals; on buses: next / previous change) |
| `-` / `=` | zoom out / in |
| `,` / `.` | previous / next transition |
| `Home` / `End` | jump to start / end |
| `a`, `Enter` | add from the Instance pane (in Signal List: expand/collapse a group) |
| `←` / `→` | on a group row: collapse / expand |
| `x` | in the waveform pane: cut the selection (alias for `dd`) |
| `r` | cycle radix, or rename the selected group |
| `h` | in the Signal List: show full / short hierarchical names |
| `↑` `↓`, `PgUp` `PgDn` | navigate lists |
| `Tab` | cycle focus (Instance → Signal List → Waveform) |
| `F1`, `?` | key bindings |

## Mouse

| Action | Effect |
| --- | --- |
| click menu / toolbar | execute |
| drag pane borders | resize panes |
| drag scrollbars | scroll signals / pan time |
| drag a Signal List row | reorder signals |
| click ruler | set cursor; drag on the waveform selects a time range |
| click inside a selection | zoom to the selected range |
| wheel over waveform | zoom at the pointer (over lists: scroll) |
| `Shift`+wheel | pan |
| middle click | zoom out |
| right click signal / group | context menu (submenus for radix, waveform, bus) |
| `Shift`/`Alt`+click a signal | add / remove it from the multi-selection |
| `Ctrl`+click a signal | select every signal between the anchor and the click |
| `Shift`/`Alt`+click in Instance | multi-select signals; a double click adds them all |
| double click a group | collapse / expand it (rename with `r` or the context menu) |
| double click | expand a scope in Instance, or add a signal |
| dialog `✕` / scrollbar | close the dialog / drag the scrollbar |
| Time button in the shortcut bar | cycle the ruler time base (timescale → fs … s) |

> Windows Terminal reserves `Shift`+click for text selection and never
> forwards it to the application; there the keyboard `V` / `Space` /
> `Shift`+`↑`/`↓` bindings (or `Alt`+click) do the multi-selection.

Actions such as `dd`, `r` (radix / rename), the context-menu radix and the
waveform modes apply to the whole multi-selection when the clicked signal is
part of it. The status bar shows the selection, the visual mode and the
register size.

## Groups

The Signal List is organised in user groups instead of the design hierarchy:
each signal row shows its leaf name (press `h` for the full hierarchical
name). A default `G0` group exists; adding a signal puts it into the group
under the cursor, and when a signal lands in the newest group a fresh empty
group is appended after it. Group numbers always continue from the highest
existing number (`G0 G1 G2 G3 G4`, delete `G3`, the next group is `G5`; delete
`G5` and it is reused). Renaming a group with `r` or the context menu changes
only its label, never its number. `J`/`K` (or dragging) move a signal across
group boundaries — it joins the group it lands in — and the group context menu
can create a new group after the clicked one.

## Context menu

- **Set Radix** ▸ Hex / Binary / Octal / Decimal / ASCII
- **Set Waveform** ▸ Digital / Analog
- **Bus Operations** ▸ Split Bus (opens a width prompt, `data` →
  `data[0]`…`data[n]`), Create Bus (opens an ordering window for the
  currently selected signals; any width, first row = MSB)
- **Remove Signal**
- On a group: New Group / Rename / Expand / Collapse / Expand All /
  Collapse All / Remove Group

Inside the Create Bus window: `↑`/`↓` select, `Shift`+`↑`/`↓` reorder,
`h`/`l` trim the LSB, `H`/`L` trim the MSB, `x` reset to the full range
(e.g. `{sig1[4:3], sig2[0], sig4[66:43]}`), `s`/`S` sort by name
ascending/descending, `r` reverse, `Enter` create, `Esc` cancel.

## Project layout

```
csrc/              ffr_bridge.cpp — FSDB Reader (FFR) C++ bridge
build.rs           detects VERDI_HOME / FSDB SDK and builds the bridge
waveform/          demo dumps (counter.vcd, complex.vcd, demo.fst)
shot/              README screenshot
src/
├── main.rs        CLI, terminal setup, event loop
├── app/           state machine: input, keys, mouse, view, navigation, actions
├── ui/            ratatui rendering: layout, tree, list, wave, menus, dialogs
├── waveform/      data model: signals, values, time, scope tree
├── vcd.rs         VCD parser
├── fst.rs         FST loader (via wellen)
├── fsdb.rs        FSDB loader (via the Verdi FFR SDK, Linux)
├── dump.rs        format detection and dispatcher
├── picker.rs      native dialog detection / wrapper
└── theme.rs       color palette
```

## Development

```sh
cargo test                       # unit + UI render tests
cargo test fsdb                  # FSDB tests (needs VERDI_HOME; cross-checks fsdb2vcd)
cargo test --no-default-features # pure TUI build
cargo clippy --all-targets
cargo fmt --check
```
