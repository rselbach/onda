//! Onda's Linux desktop entry point.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
fn main() -> gtk::glib::ExitCode {
    linux::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("The GTK shell is available on Linux. Use shell/macos on macOS.");
}
