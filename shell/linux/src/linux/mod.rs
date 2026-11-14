//! Linux shell, kept separate from macOS and the platform-neutral core.

mod clipboard;
mod core;
mod hotkeys;
mod media;
mod output;
mod overlay;
mod portal;
mod process;
mod sounds;
mod speech;
mod system;
mod target;
mod tray;
mod ui;
mod updates;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "com.rselbach.onda";

enum Message {
    Core(onda_app::Event),
    Hotkey(onda_app::HotkeyEvent),
    Download(onda_app::seams::DownloadUpdate),
    DevicesChanged,
    UpdaterChanged,
    ShortcutStatus(String),
    Notice(String, String),
    Inhibit(Option<String>),
    Tray(tray::Command),
    CancelDictation,
}

fn send(sender: &async_channel::Sender<Message>, message: Message) {
    if sender.try_send(message).is_err() {
        tracing::debug!("shell event receiver closed");
    }
}

fn application_id() -> String {
    // Scratch runs isolate both application activation and stored credentials.
    match std::env::var_os("ONDA_DATA_DIR") {
        Some(path) => {
            use std::hash::{Hash, Hasher};
            let mut hash = std::hash::DefaultHasher::new();
            path.hash(&mut hash);
            format!("{APP_ID}.scratch.h{:x}", hash.finish())
        }
        None => APP_ID.into(),
    }
}

pub fn run() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "onda=info".into()),
        )
        .init();
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 && args[1] == "--version" {
        println!("Onda {}", env!("ONDA_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "Onda — local dictation\n\nonda [--background]\nonda --version\nonda --start | --stop | --toggle | --cancel | --paste-last | --status | --quit\n\nControl commands require a running Onda. ONDA_DATA_DIR selects a separate data directory."
        );
        return glib::ExitCode::SUCCESS;
    }
    let id = application_id();
    let control_only = args.get(1).is_some_and(|arg| {
        matches!(
            arg.as_str(),
            "--start" | "--stop" | "--toggle" | "--cancel" | "--paste-last" | "--status" | "--quit"
        )
    });
    let mut flags = gio::ApplicationFlags::HANDLES_COMMAND_LINE;
    if control_only {
        flags |= gio::ApplicationFlags::IS_LAUNCHER;
    }
    let application = adw::Application::builder()
        .application_id(id)
        .flags(flags)
        .build();
    let state: Rc<RefCell<Option<Rc<ui::Window>>>> = Rc::default();
    let holder: Rc<RefCell<Option<gio::ApplicationHoldGuard>>> = Rc::default();
    let command_state = state.clone();
    application.connect_command_line(move |application, command| {
        let arguments = command.arguments();
        let argument = arguments.get(1).and_then(|arg| arg.to_str()).unwrap_or("");
        let control = matches!(
            argument,
            "--start" | "--stop" | "--toggle" | "--cancel" | "--paste-last" | "--status" | "--quit"
        );
        if arguments.len() > 2 || (!argument.is_empty() && argument != "--background" && !control) {
            command.printerr_literal("Unknown arguments; run onda --help.\n");
            return glib::ExitCode::from(2);
        }
        if control && !command.is_remote() {
            command.printerr_literal("Onda is not running. Open Onda first.\n");
            return glib::ExitCode::FAILURE;
        }
        if command_state.borrow().is_none() {
            match ui::Window::new(application) {
                Ok(window) => {
                    *holder.borrow_mut() = Some(application.hold());
                    *command_state.borrow_mut() = Some(window);
                }
                Err(error) => {
                    command.printerr_literal(&format!("Could not start Onda: {error:#}\n"));
                    return glib::ExitCode::FAILURE;
                }
            }
        }
        if let Some(window) = command_state.borrow().as_ref() {
            if matches!(argument, "--start" | "--stop" | "--toggle" | "--paste-last") {
                window.core.output.held.update(false);
            }
            match argument {
                "--start" => window.core.app.start_dictation(),
                "--stop" => window.core.app.stop_dictation(),
                "--toggle" => window.core.app.toggle_dictation_from_menu(),
                "--cancel" => window.core.app.cancel_dictation(),
                "--paste-last" => window.core.app.deliver_last_transcript(),
                "--quit" => application.quit(),
                "--status" => command.print_literal(&format!("{}\n", window.status())),
                "--background" => {}
                _ => window.present(),
            }
        }
        glib::ExitCode::SUCCESS
    });
    application.connect_shutdown(move |_| {
        if let Some(window) = state.borrow_mut().take() {
            window
                .core
                .runtime
                .block_on(window.core.app.shutdown_async());
        }
    });
    application.run()
}
