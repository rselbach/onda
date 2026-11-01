//! Core → shell events, delivered in emission order (ARCH-5).

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use onda_domain::hotkey::HotkeyView;
use onda_domain::non_dictation_ui::DownloadOverlayView;
use onda_domain::overlay::OverlayView;
use onda_domain::status_menu::StatusMenu;
use onda_domain::tray::TrayView;

use crate::views::{
    AboutPage, GeneralPage, GettingStartedPage, HistoryPage, HotkeyPage, MainWindowSection,
    OnboardingView, PostProcessingPage, ReplacementsPage, SpeechPage, UpdateNotice,
};

/// A complete view model or request for the shell.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The tray icon changed.
    TrayChanged(TrayView),
    /// The status menu changed.
    MenuChanged(Box<StatusMenu>),
    /// The recording overlay changed, notices included (OVL-5..OVL-15).
    OverlayChanged(OverlayView),
    /// The download overlay changed (OVL-16..OVL-19).
    DownloadOverlayChanged(DownloadOverlayView),
    /// History changed; the shell re-queries (HIS-10).
    HistoryChanged,
    /// The shortcut, its recorder, or the Fn warning changed (HOT-6,
    /// HOT-11, HOT-35).
    HotkeyChanged(HotkeyView),
    /// Show the onboarding window (ONB-13, WIN-4, DICT-5). Its content
    /// arrives in [`Event::OnboardingChanged`].
    OpenOnboarding,
    /// Close the onboarding window (ONB-10).
    CloseOnboarding,
    /// The onboarding view model (ONB-1..ONB-9).
    OnboardingChanged(Box<OnboardingView>),
    /// Open the main window on a section, selecting it even when the window
    /// is already open (UI-4, UI-11, DICT-5).
    OpenMainWindow(MainWindowSection),
    /// Show the model update notice window (DL-14).
    ShowUpdateNotice(Box<UpdateNotice>),
    /// Close the model update notice window.
    CloseUpdateNotice,
    /// Whether Onda should behave as a regular app (dock icon, app switcher,
    /// app menu) because one of its windows is open (UI-2).
    AppPresenceChanged {
        /// A window is open.
        regular: bool,
    },
    /// Bring Onda to the front, e.g. after a Keychain prompt pushed it back
    /// (CLN-25).
    ActivateApp,
    /// Show a blocking modal alert: the channel of critical messages.
    /// Nothing in v1 sends one.
    ShowAlert {
        /// The message title.
        title: String,
        /// The message body.
        message: String,
        /// The only button's label, "OK".
        button: &'static str,
    },
    /// The Getting Started page (WIN-4..WIN-7).
    GettingStartedChanged(Box<GettingStartedPage>),
    /// The General page (WIN-8..WIN-15).
    GeneralChanged(Box<GeneralPage>),
    /// The Hotkey page beyond the recorder (WIN-16, WIN-17).
    HotkeyPageChanged(Box<HotkeyPage>),
    /// The Speech page (WIN-18..WIN-20).
    SpeechChanged(Box<SpeechPage>),
    /// The Post-Processing page (CLN-26, CLN-27).
    PostProcessingChanged(Box<PostProcessingPage>),
    /// The Replacements page (REP-8..REP-12).
    ReplacementsChanged(Box<ReplacementsPage>),
    /// The History page (HIS-17..HIS-27).
    HistoryPageChanged(Box<HistoryPage>),
    /// The About page (WIN-21).
    AboutChanged(Box<AboutPage>),
}

/// Receives core events. Called from one core thread, in emission order;
/// the shell hops to its UI thread without reordering.
pub trait EventSink: Send + Sync {
    /// Handles one event.
    fn emit(&self, event: Event);
}

enum Item {
    Event(Event),
    Flush(Sender<()>),
}

/// One FIFO channel drained by one thread, so events can never overtake each
/// other (ARCH-5), and a slow shell never blocks the session actor.
pub(crate) struct EventBus {
    sender: Mutex<Option<Sender<Item>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl EventBus {
    pub(crate) fn new(sink: Arc<dyn EventSink>) -> Self {
        let (sender, receiver) = mpsc::channel::<Item>();
        let worker = thread::Builder::new()
            .name("onda-events".to_owned())
            .spawn(move || {
                for item in receiver {
                    match item {
                        Item::Event(event) => sink.emit(event),
                        Item::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            });
        match worker {
            Ok(worker) => Self {
                sender: Mutex::new(Some(sender)),
                worker: Mutex::new(Some(worker)),
            },
            Err(error) => {
                // without a delivery thread nothing can reach the shell
                tracing::error!(%error, "could not start the event thread");
                Self {
                    sender: Mutex::new(None),
                    worker: Mutex::new(None),
                }
            }
        }
    }

    pub(crate) fn emit(&self, event: Event) {
        self.send(Item::Event(event));
    }

    /// Blocks until every event emitted so far reached the sink.
    pub(crate) fn flush(&self) {
        let (done, wait) = mpsc::channel();
        self.send(Item::Flush(done));
        // an error means the worker is gone, so nothing is pending
        let _ = wait.recv();
    }

    /// Delivers what is queued, then stops the worker. Idempotent.
    pub(crate) fn close(&self) {
        // dropping the sender ends the worker loop once the queue drains
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(sender);
        let worker = self
            .worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(worker) = worker
            && worker.join().is_err()
        {
            tracing::error!("the event thread panicked");
        }
    }

    fn send(&self, item: Item) {
        let sender = self.sender.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sender) = sender.as_ref() {
            // a send only fails after close, when nobody is listening
            let _ = sender.send(item);
        }
    }
}

impl Drop for EventBus {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onda_domain::tray::TrayIcon;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl EventSink for Recorder {
        fn emit(&self, event: Event) {
            let Event::TrayChanged(view) = event else {
                return;
            };
            if let Ok(mut seen) = self.0.lock() {
                seen.push(view.accessibility_label);
            }
        }
    }

    fn tray(label: String) -> Event {
        Event::TrayChanged(TrayView {
            icon: TrayIcon::Ready,
            red_tint: false,
            accessibility_label: label,
        })
    }

    fn seen(recorder: &Recorder) -> Vec<String> {
        recorder
            .0
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }

    #[test]
    fn arch_5_delivers_in_emission_order() {
        let recorder = Arc::new(Recorder::default());
        let bus = EventBus::new(recorder.clone());
        let want: Vec<String> = (0..1_000).map(|i| format!("event {i}")).collect();
        for label in &want {
            bus.emit(tray(label.clone()));
        }
        bus.close();
        assert_eq!(seen(&recorder), want);
    }

    #[test]
    fn flush_waits_for_everything_emitted_before_it() {
        let recorder = Arc::new(Recorder::default());
        let bus = EventBus::new(recorder.clone());
        for i in 0..100 {
            bus.emit(tray(format!("event {i}")));
        }
        bus.flush();
        assert_eq!(seen(&recorder).len(), 100);
    }
}
