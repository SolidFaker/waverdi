# waverdi

A Verdi-style RTL waveform viewer for the terminal, written in Rust.

waverdi is meant for SSH sessions and headless machines: open a VCD dump and
browse signals, values and waveform traces without ever leaving the terminal.

```
 waverdi  File   View   Signal   Trace   Help
  Open   │ Zoom In   │ Zoom Out   │ Fit   │ Center   │ Goto   │ Find   │ Prev Tr   │ Next Tr   │
┌ nTrace ─────────────────────┐ Signal List              Value     │ 100ns      150ns      200ns │
│▾ design                     │▾ tb/ (7)                          │   ┴          ┴          ┴   │
│  ▸ tb                       │  clk                    b0→b1     │▁│▔│▁│▔│▁▁│▔│▁│▔│▁│▔▔│▔│▁ │
│                             │  rst_n                  b1        │▔▔▔▔▔▔▔▔▔│▔▔▔▔▔▔▔▔▔▔▔▔▔▔ │
│                             │  ▾ u_cpu/ (3)                     │        │
│                             │    state                h1→h2     │╳h1───│──╳─╳──╳──╳─h1──╳── │
│                             │    pc                   h00→h01   │h00───│h01────╳─h02────╳ │
│                             │    data                 h648d5ce9 │──────│───────╳────────╳ │
│                             │  ▾ u_ram/ (5)                     │
│                             │    addr, wdata, we, rdata, dq     │                         │
└─────────────────────────────┘                                   │────████───────────────
 counter.vcd | timescale 1ns | cursor 150ns | ΔT 70ns | zoom 7ns/char | signals 7 | focus nTrace
```

## Features

- **VCD parser** — scopes, vectors, `real`, `string`, `x`/`z`, every value
  base (`b`, `o`, `h`, `d`, `r`, `s`), `$dumpvars`, timescale handling.
- **Verdi-like layout** — `nTrace` hierarchy browser, `Signal List` with
  current values, `nWave` waveform pane with ruler, cursor and range markers.
- **Waveform rendering** — thin high/low rails, `/` rising and `\` falling
  edges (dense activity collapses to `│`), inline bus values, analog rendering
  for `real` and logic signals.
- **Hierarchy groups in the Signal List** — displayed signals are grouped by
  scope and can be collapsed / expanded / removed. Groups are mirrored in the
  waveform pane.
- **Signal operations** — radix (Hex/Binary/Octal/Decimal/ASCII), digital ⇄
  analog waveform, split a bus into bits, merge consecutive 1-bit signals into
  a bus.
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

## Usage

```sh
waverdi counter.vcd                 # open a dump
waverdi --list-signals complex.vcd  # print the hierarchy and exit
waverdi --no-gui counter.vcd        # force the built-in TUI file browser
waverdi --gui counter.vcd           # force the native file dialog
```

`counter.vcd` (4-bit counter with enable/carry) and `complex.vcd` (a small
CPU/RAM/sensor design with 16 signals, tristate and analog values) are
included as demos.

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
| `g` | go to time (`1500`, `1.5us`, ...) |
| `s` | search signals |
| `z` / `Z` | zoom in / out around the cursor |
| `f` | fit the whole time range |
| `c` | center the cursor |
| `←` / `→` | move the cursor (hold `Shift` for x10) |
| `,` / `.` | previous / next transition |
| `Home` / `End` | jump to start / end |
| `a`, `Enter` | add from nTrace (in Signal List: expand/collapse a group) |
| `←` / `→` | on a group row: collapse / expand |
| `d` / `x` | remove selected / remove all |
| `r` | cycle radix (Bin/Oct/Dec/Hex/ASCII) |
| `↑` `↓`, `PgUp` `PgDn` | navigate lists |
| `Tab` | cycle focus (nTrace → Signal List → Waveform) |
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
| right click signal / group | context menu (radix, waveform, bus, group, remove) |
| double click | expand scope/group, or add a signal |

## Context menu

- **Set Radix**: Hex / Binary / Octal / Decimal / ASCII
- **Set Waveform**: Digital / Analog
- **Bus**: Split Bus (`data` → `data[0]`…`data[n]`), Create Bus (the clicked
  1-bit signal plus the following consecutive 1-bit signals, first = MSB)
- **Remove**
- On a group: Expand / Collapse / Expand All / Collapse All / Remove Group

## Project layout

```
src/
├── main.rs        CLI, terminal setup, event loop
├── app/           state machine: input, keys, mouse, view, navigation, actions
├── ui/            ratatui rendering: layout, tree, list, wave, menus, dialogs
├── waveform/      data model: signals, values, time, scope tree
├── vcd.rs         VCD parser
├── picker.rs      native dialog detection / wrapper
└── theme.rs       color palette
```

## Development

```sh
cargo test                       # unit + UI render tests
cargo test --no-default-features # pure TUI build
cargo clippy --all-targets
cargo fmt --check
```
