//! The dictation hotkey inside the session actor: registration and its
//! failure handling (HOT-8, HOT-12, HOT-17..HOT-20), the lone-modifier
//! gesture (HOT-16, HOT-21..HOT-23), the recorder's capture (HOT-8), and
//! the Fn warning (HOT-35). The shell performs the OS calls; every decision
//! and timer lives here.

use std::sync::{Arc, Mutex, PoisonError};

use onda_domain::hotkey::{
    ACCESSIBILITY_POLL_INTERVAL, FailureMailbox, GestureAction, GestureInput, GlobeKeyAction,
    HotkeyView, LoneModifierGesture, ModifierKey, RegistrationRetry, Shortcut, ShortcutRecorder,
    StreamDisableDecision, StreamReenablePolicy, gesture_input_for_flags_change,
    gesture_input_for_key_down,
};
use onda_domain::messages::UserMessage;
use tokio::sync::oneshot;

use super::{Actor, Msg, Task, blocking_send};
use crate::HotkeyEvent;
use crate::effects::{Effect, RegisterOutcome};
use crate::pipeline::mono;

/// Hotkey state the session actor owns.
pub(super) struct Hotkeys {
    recorder: Arc<Mutex<ShortcutRecorder>>,
    gesture: LoneModifierGesture,
    debounce: Option<Task>,
    stream: StreamReenablePolicy,
    retry: RegistrationRetry,
    mailbox: FailureMailbox,
    // trust read with the failure the mailbox holds (HOT-20)
    held_trust: bool,
    poll: Option<Task>,
    // the shortcut last handed to the shell; `None` while unregistered for a
    // capture (HOT-8)
    registered: Option<Shortcut>,
    // bumped on every register and unregister, so a late result for a
    // superseded registration is dropped
    generation: u64,
    // a combo press is down; HOT-18 releases it on unregister
    combo_down: bool,
    capturing: bool,
    pub(super) globe: GlobeKeyAction,
}

impl Hotkeys {
    pub(super) fn new(recorder: Arc<Mutex<ShortcutRecorder>>) -> Self {
        Self {
            recorder,
            gesture: LoneModifierGesture::new(),
            debounce: None,
            stream: StreamReenablePolicy::new(),
            retry: RegistrationRetry::new(),
            mailbox: FailureMailbox::new(),
            held_trust: false,
            poll: None,
            registered: None,
            generation: 0,
            combo_down: false,
            capturing: false,
            globe: GlobeKeyAction::Unknown,
        }
    }
}

impl Actor {
    // --- raw input (HOT-14a, HOT-16, HOT-17, HOT-20a) ---

    pub(super) fn on_hotkey(&mut self, event: HotkeyEvent) {
        match event {
            HotkeyEvent::ComboDown => {
                // a press queued before a capture's unregister is dropped
                if self.hotkeys.registered.is_none() {
                    return;
                }
                self.hotkeys.combo_down = true;
                self.key_down();
            }
            HotkeyEvent::ComboUp => {
                // only a tracked press is released: HOT-18 already sent the
                // key-up of a press held through an unregister
                if std::mem::take(&mut self.hotkeys.combo_down) {
                    self.key_up();
                }
            }
            HotkeyEvent::Cancel => {
                tracing::info!("Escape cancelled the session");
                self.cancel();
            }
            HotkeyEvent::ModifierFlags(change) => {
                let Some(key) = self.registered_lone_modifier() else {
                    return;
                };
                // a real event proves the stream healthy (HOT-17)
                self.hotkeys.stream.on_event();
                let phase = self.hotkeys.gesture.phase();
                if let Some(input) = gesture_input_for_flags_change(&change, key, phase) {
                    self.gesture_input(input);
                }
            }
            HotkeyEvent::KeyDown(code) => {
                if self.registered_lone_modifier().is_none() {
                    return;
                }
                self.hotkeys.stream.on_event();
                if let Some(input) = gesture_input_for_key_down(&code) {
                    self.gesture_input(input);
                }
            }
            HotkeyEvent::StreamDisabled { status } => self.stream_disabled(status),
        }
    }

    pub(super) fn gesture_input(&mut self, input: GestureInput) {
        match self.hotkeys.gesture.handle(input) {
            GestureAction::None => {}
            GestureAction::ScheduleDebounce { token, after } => {
                self.hotkeys.debounce = Some(self.timer(after, Msg::Debounce(token)));
            }
            GestureAction::CancelDebounce => self.hotkeys.debounce = None,
            GestureAction::Down => self.key_down(),
            GestureAction::Up => self.key_up(),
            GestureAction::Discard => {
                let intent = self.press.discard(self.snapshot());
                self.apply_intent(intent);
            }
        }
    }

    fn registered_lone_modifier(&self) -> Option<ModifierKey> {
        self.hotkeys
            .registered
            .as_ref()
            .and_then(Shortcut::lone_modifier)
    }

    // HOT-17: re-enable at most 5 times in 5 s, then tear down and report
    fn stream_disabled(&mut self, status: i32) {
        if self.registered_lone_modifier().is_none() {
            return;
        }
        self.gesture_input(GestureInput::StreamDisabled);
        match self.hotkeys.stream.on_disabled(mono(self.origin)) {
            StreamDisableDecision::Reenable => {
                tracing::warn!("the OS disabled the hotkey event stream; re-enabling it");
                self.effects.send(Effect::ReenableStream);
            }
            StreamDisableDecision::GiveUp => {
                tracing::error!(
                    "the OS keeps disabling the hotkey event stream; giving up (Accessibility was likely revoked)"
                );
                self.effects.send(Effect::UnregisterHotkey);
                self.check_hotkey_failure(Some(self.hotkeys.generation), status);
            }
            StreamDisableDecision::AlreadyDead => {}
        }
    }

    // --- registration (HOT-13, HOT-14, HOT-18, SET-5) ---

    /// Registers the configured shortcut. An explicit registration (launch,
    /// settings change, capture start or end) first ends any retry episode
    /// (HOT-19); a retry after Accessibility was granted does not.
    pub(super) fn register_hotkey(&mut self, explicit: bool) {
        if explicit {
            self.end_retry_episode();
        }
        self.release_held_hotkey();
        let shortcut = self.settings.hotkey.clone();
        self.hotkeys.generation += 1;
        self.hotkeys.registered = Some(shortcut.clone());
        self.hotkeys.stream = StreamReenablePolicy::new();
        let generation = self.hotkeys.generation;
        let (done, outcome) = oneshot::channel();
        self.effects.send(Effect::RegisterHotkey { shortcut, done });
        let messages = self.messages.clone();
        tokio::spawn(async move {
            // the sender only drops when the effects worker failed, which
            // it logs
            if let Ok(outcome) = outcome.await {
                let _ = messages.send(Msg::HotkeyRegistered {
                    generation,
                    outcome,
                });
            }
        });
        self.read_globe_key();
    }

    fn unregister_hotkey(&mut self) {
        self.release_held_hotkey();
        self.hotkeys.registered = None;
        self.hotkeys.generation += 1;
        self.effects.send(Effect::UnregisterHotkey);
    }

    // HOT-18: a press held through an unregister gets its key-up now, so
    // Hold mode does not stay recording
    fn release_held_hotkey(&mut self) {
        if std::mem::take(&mut self.hotkeys.combo_down) {
            self.key_up();
        }
        self.gesture_input(GestureInput::StreamDisabled);
    }

    pub(super) fn hotkey_registered(&mut self, generation: u64, outcome: RegisterOutcome) {
        if generation != self.hotkeys.generation {
            return;
        }
        match outcome {
            RegisterOutcome::Registered => {
                tracing::info!(
                    hotkey = %self.settings.hotkey.display(self.ports.platform.key_names),
                    retried = self.hotkeys.retry.is_pending(),
                    "registered the dictation hotkey"
                );
                self.end_retry_episode();
            }
            RegisterOutcome::Failed { status, trusted } => {
                self.registration_failed(status, trusted);
            }
        }
    }

    /// Reads Accessibility trust off the actor, then handles a failure with
    /// `status`. `generation` ties it to a registration; `None` means the
    /// shell reported it on its own.
    pub(super) fn check_hotkey_failure(&self, generation: Option<u64>, status: i32) {
        let permissions = self.ports.permissions.clone();
        tokio::spawn(blocking_send(self.messages.clone(), move || {
            Msg::HotkeyFailure {
                generation,
                status,
                trusted: permissions.accessibility_trusted(),
            }
        }));
    }

    pub(super) fn hotkey_failure(&mut self, generation: Option<u64>, status: i32, trusted: bool) {
        if generation.is_some_and(|g| g != self.hotkeys.generation) {
            return;
        }
        self.registration_failed(status, trusted);
    }

    // HOT-20: held until the UI is ready, keeping only the latest
    fn registration_failed(&mut self, status: i32, trusted: bool) {
        tracing::error!(
            status,
            hotkey = %self.settings.hotkey.display(self.ports.platform.key_names),
            "could not register the dictation hotkey"
        );
        match self.hotkeys.mailbox.report(status) {
            Some(status) => self.deliver_hotkey_failure(status, trusted),
            None => self.hotkeys.held_trust = trusted,
        }
    }

    pub(super) fn deliver_held_hotkey_failure(&mut self) {
        if let Some(status) = self.hotkeys.mailbox.handler_ready() {
            let trusted = self.hotkeys.held_trust;
            self.deliver_hotkey_failure(status, trusted);
        }
    }

    // HOT-19: a combo failure always notifies and ends any lone-modifier
    // episode; a lone-modifier episode notifies on its first failure only
    fn deliver_hotkey_failure(&mut self, status: i32, trusted: bool) {
        let hotkey = self.settings.hotkey.clone();
        let platform = self.ports.platform;
        if let Some(notice) = self
            .hotkeys
            .retry
            .record_failure(&hotkey, status, trusted, platform)
        {
            self.notify(UserMessage::HotkeyRegistrationFailed(notice));
        }
        self.update_trust_polling();
    }

    // --- Accessibility polling while a lone-modifier failure is pending
    // (HOT-19) ---

    fn end_retry_episode(&mut self) {
        self.hotkeys.retry.reset();
        self.hotkeys.poll = None;
    }

    fn update_trust_polling(&mut self) {
        if !self.hotkeys.retry.is_pending() {
            self.hotkeys.poll = None;
            return;
        }
        if self.hotkeys.poll.is_none() {
            self.hotkeys.poll =
                Some(self.timer(ACCESSIBILITY_POLL_INTERVAL, Msg::PollAccessibility));
        }
    }

    pub(super) fn poll_accessibility(&mut self) {
        self.hotkeys.poll = None;
        self.check_trust();
    }

    fn check_trust(&self) {
        if !self.hotkeys.retry.is_pending() {
            return;
        }
        let permissions = self.ports.permissions.clone();
        tokio::spawn(blocking_send(self.messages.clone(), move || {
            Msg::TrustChecked(permissions.accessibility_trusted())
        }));
    }

    pub(super) fn trust_checked(&mut self, trusted: bool) {
        self.permissions.accessibility = trusted;
        // retry only when trust went from untrusted to trusted
        if self.hotkeys.retry.record_trust(trusted) {
            tracing::info!("Accessibility granted; registering the dictation hotkey again");
            self.register_hotkey(false);
        }
        self.update_trust_polling();
    }

    pub(super) fn hotkey_app_became_active(&mut self) {
        self.check_trust();
        self.read_globe_key();
    }

    // SET-5: a changed shortcut re-registers at once
    pub(super) fn hotkey_settings_changed(&mut self) {
        self.refresh_settings();
        if self.hotkeys.capturing || self.hotkeys.registered.as_ref() == Some(&self.settings.hotkey)
        {
            return;
        }
        self.register_hotkey(true);
    }

    // --- recording a shortcut (HOT-8, HOT-12) ---

    pub(super) fn capture_began(&mut self) {
        self.hotkeys.capturing = true;
        self.end_retry_episode();
        self.unregister_hotkey();
    }

    // the recorder disappeared (page change, window close): its capture
    // ends as a cancel, so the hotkey is never left unregistered (HOT-8)
    pub(super) fn end_recorder_capture(&mut self) {
        let stopped = self
            .hotkeys
            .recorder
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop();
        if stopped {
            self.capture_ended();
        }
    }

    pub(super) fn capture_ended(&mut self) {
        self.hotkeys.capturing = false;
        self.refresh_settings();
        self.register_hotkey(true);
    }

    // HOT-12: save first, then register the new value once
    pub(super) fn capture_committed(&mut self, shortcut: &Shortcut) {
        self.hotkeys.capturing = false;
        self.save_hotkey(shortcut);
        self.register_hotkey(true);
    }

    pub(super) fn set_hotkey(&mut self, shortcut: &Shortcut) {
        self.save_hotkey(shortcut);
        if !self.hotkeys.capturing {
            self.register_hotkey(true);
        }
    }

    fn save_hotkey(&mut self, shortcut: &Shortcut) {
        if let Err(error) = self.ports.settings.set_hotkey(shortcut) {
            tracing::error!(%error, "could not save the hotkey");
        }
        self.refresh_settings();
    }

    // --- Fn warning (HOT-35) and the view ---

    pub(super) fn read_globe_key(&self) {
        if self.settings.hotkey.lone_modifier() != Some(ModifierKey::Fn) {
            return;
        }
        let hotkeys = self.ports.hotkeys.clone();
        tokio::spawn(blocking_send(self.messages.clone(), move || {
            Msg::GlobeKey(hotkeys.globe_key_action())
        }));
    }

    pub(super) fn hotkey_view(&self) -> HotkeyView {
        let recorder = self
            .hotkeys
            .recorder
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        HotkeyView::resolve(
            &self.settings.hotkey,
            &recorder,
            self.hotkeys.globe,
            self.ports.platform,
        )
    }
}
