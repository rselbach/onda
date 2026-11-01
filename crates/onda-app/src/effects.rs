//! Fire-and-forget shell calls (cues, idle-sleep prevention, hotkey
//! registration, the Accessibility prompt, notifications), run one at a time
//! in the order the session asked for them, off the session actor (ARCH-6).

use std::sync::Arc;

use onda_domain::hotkey::{ModifierSet, Shortcut};
use onda_domain::messages::{Channel, MessageClass, UserMessage, route};
use tokio::sync::{mpsc, oneshot};
use tokio::task::spawn_blocking;

use crate::ports::{
    Cue, HotkeySource, LoginItem, Notifier, Permissions, Power, SoundPlayer, Updater,
};

/// Reason given to the OS for preventing idle sleep (DICT-10).
const IDLE_SLEEP_REASON: &str = "Dictation in progress";

/// How registering the dictation shortcut went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegisterOutcome {
    Registered,
    /// `trusted` is Accessibility trust read right after the failure
    /// (HOT-19).
    Failed {
        status: i32,
        trusted: bool,
    },
}

pub(crate) enum Effect {
    Sound(Cue),
    PreventIdleSleep,
    AllowIdleSleep,
    RegisterHotkey {
        shortcut: Shortcut,
        done: oneshot::Sender<RegisterOutcome>,
    },
    UnregisterHotkey,
    ReenableStream,
    RegisterCancel(Vec<ModifierSet>),
    UnregisterCancel,
    PromptAccessibility,
    /// Ask for notification permission (ONB-12).
    RequestNotifications,
    /// Open keyboard settings (HOT-35).
    OpenKeyboardSettings,
    /// Open the system Login Items settings (LOGIN-2).
    OpenLoginItems,
    /// A user-initiated update check (UPD-1).
    CheckForUpdates,
    /// Turn automatic update checks on or off.
    SetAutomaticUpdates(bool),
    /// A background message: a notification when authorized, else (or when
    /// the post fails) the overlay notice (MSG-1).
    Notify(UserMessage),
    Flush(oneshot::Sender<()>),
}

pub(crate) struct EffectPorts {
    pub(crate) sounds: Arc<dyn SoundPlayer>,
    pub(crate) power: Arc<dyn Power>,
    pub(crate) hotkeys: Arc<dyn HotkeySource>,
    pub(crate) permissions: Arc<dyn Permissions>,
    pub(crate) notifier: Arc<dyn Notifier>,
    pub(crate) login_item: Arc<dyn LoginItem>,
    pub(crate) updater: Arc<dyn Updater>,
    pub(crate) tray_term: Arc<str>,
}

/// Handle to the effects task.
#[derive(Clone)]
pub(crate) struct Effects {
    queue: mpsc::UnboundedSender<Effect>,
}

impl Effects {
    /// Spawns the worker. `fallback` receives messages that must show on the
    /// overlay because no notification could be posted.
    pub(crate) fn spawn(
        ports: EffectPorts,
        fallback: impl Fn(UserMessage) + Send + Sync + 'static,
    ) -> Self {
        let (queue, mut inbox) = mpsc::unbounded_channel::<Effect>();
        let ports = Arc::new(ports);
        let fallback = Arc::new(fallback);
        tokio::spawn(async move {
            while let Some(effect) = inbox.recv().await {
                if let Effect::Flush(done) = effect {
                    let _ = done.send(());
                    continue;
                }
                let ports = ports.clone();
                let fallback = fallback.clone();
                let ran = spawn_blocking(move || run(&ports, effect, fallback.as_ref())).await;
                if let Err(error) = ran {
                    tracing::error!(%error, "shell call failed");
                }
            }
        });
        Self { queue }
    }

    pub(crate) fn send(&self, effect: Effect) {
        let _ = self.queue.send(effect);
    }

    /// Waits until every effect sent so far ran.
    pub(crate) async fn flush(&self) {
        let (done, wait) = oneshot::channel();
        if self.queue.send(Effect::Flush(done)).is_ok() {
            let _ = wait.await;
        }
    }
}

fn run(ports: &EffectPorts, effect: Effect, fallback: &(dyn Fn(UserMessage) + Send + Sync)) {
    match effect {
        Effect::Sound(cue) => ports.sounds.play(cue),
        Effect::PreventIdleSleep => ports.power.begin_prevent_idle_sleep(IDLE_SLEEP_REASON),
        Effect::AllowIdleSleep => ports.power.end_prevent_idle_sleep(),
        Effect::RegisterHotkey { shortcut, done } => {
            let outcome = match ports.hotkeys.register(&shortcut) {
                Ok(()) => RegisterOutcome::Registered,
                Err(status) => RegisterOutcome::Failed {
                    status,
                    trusted: ports.permissions.accessibility_trusted(),
                },
            };
            // nobody waits once the session actor stopped
            let _ = done.send(outcome);
        }
        Effect::UnregisterHotkey => ports.hotkeys.unregister(),
        Effect::ReenableStream => ports.hotkeys.reenable_stream(),
        Effect::RegisterCancel(modifiers) => {
            if let Err(error) = ports.hotkeys.register_cancel(&modifiers) {
                // the overlay's cancel button still works (HOT-34)
                tracing::warn!(%error, "could not register the Escape cancel hotkey");
            }
        }
        Effect::UnregisterCancel => ports.hotkeys.unregister_cancel(),
        Effect::PromptAccessibility => ports.permissions.prompt_accessibility(),
        Effect::RequestNotifications => ports.notifier.request_authorization(),
        Effect::OpenKeyboardSettings => ports.hotkeys.open_keyboard_settings(),
        Effect::OpenLoginItems => ports.login_item.open_settings(),
        Effect::CheckForUpdates => ports.updater.check_now(),
        Effect::SetAutomaticUpdates(enabled) => ports.updater.set_automatic_checks(enabled),
        Effect::Notify(message) => {
            let authorization = ports.notifier.authorization();
            match route(MessageClass::Background, authorization) {
                Channel::Notification => {
                    let body = message.body(&ports.tray_term);
                    if let Err(error) = ports.notifier.post(message.title(), &body) {
                        tracing::warn!(%error, title = message.title(), "notification failed");
                        fallback(message);
                    }
                }
                Channel::OverlayNotice | Channel::Alert => fallback(message),
            }
        }
        Effect::Flush(_) => {}
    }
}
