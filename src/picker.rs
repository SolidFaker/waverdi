use std::path::PathBuf;

/// True when the process looks like it is running over SSH.
fn ssh_session() -> bool {
    ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .iter()
        .any(|key| std::env::var_os(key).is_some())
}

#[cfg(windows)]
fn has_display() -> bool {
    true
}

#[cfg(not(windows))]
fn has_display() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Whether a native GUI file dialog can be used in this environment.
///
/// SSH sessions never take it (the window would appear on a desktop the user
/// cannot see), and a build without the `gui` feature never takes it either.
pub fn detect_gui() -> bool {
    cfg!(feature = "gui") && !ssh_session() && has_display()
}

/// Show the operating system's file picker and return the chosen dump.
#[cfg(feature = "gui")]
pub fn pick_vcd() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open Waveform")
        .add_filter("Waveform (VCD/FST)", &["vcd", "fst"])
        .add_filter("FSDB (needs Verdi FFR)", &["fsdb"])
        .add_filter("All files", &["*"])
        .pick_file()
}

/// Built without the `gui` feature: there is no native dialog to show.
#[cfg(not(feature = "gui"))]
pub fn pick_vcd() -> Option<PathBuf> {
    None
}

/// Show the operating system's file picker and return the chosen filelist.
#[cfg(feature = "gui")]
pub fn pick_filelist() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Load Filelist")
        .add_filter("Filelist", &["f", "list", "txt"])
        .add_filter("All files", &["*"])
        .pick_file()
}

/// Built without the `gui` feature: there is no native dialog to show.
#[cfg(not(feature = "gui"))]
pub fn pick_filelist() -> Option<PathBuf> {
    None
}
