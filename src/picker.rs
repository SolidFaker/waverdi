use std::path::PathBuf;

/// Show the operating system's file picker and return the chosen VCD file.
pub fn pick_vcd() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open Waveform")
        .add_filter("VCD waveform", &["vcd"])
        .add_filter("All files", &["*"])
        .pick_file()
}
