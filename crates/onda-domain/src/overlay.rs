//! Recording overlay state and view model (OVL-4 to OVL-15, CLN-3, SND-1).
//!
//! [`Overlay`] owns the panel's content and its notice lifecycle; the shell
//! renders [`OverlayView`] and calls [`Overlay::tick`] at
//! [`Overlay::notice_deadline`].

use std::time::Duration;

use crate::level::{SILENT_MIC_BODY, WaveformHistory, silent_mic_title};
use crate::types::MonoTime;

/// How long a notice stays up (OVL-12).
pub const NOTICE_DURATION: Duration = Duration::from_secs(4);

/// Overlay fade duration (OVL-4).
pub const OVERLAY_FADE: Duration = Duration::from_millis(200);

/// Cancel button tooltip (OVL-7).
pub const CANCEL_TOOLTIP: &str = "Cancel (Esc)";

/// Cancel button accessibility label (OVL-7).
pub const CANCEL_ACCESSIBILITY_LABEL: &str = "Cancel Dictation";

/// A titled message shown on the overlay in place of its live content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// One-line title.
    pub title: String,
    /// Up to three lines of message.
    pub message: String,
}

/// Which processing step the overlay names (OVL-10, CLN-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingStage {
    /// Running ASR.
    Transcribing,
    /// Running OpenRouter cleanup.
    Refining,
}

impl ProcessingStage {
    /// The processing label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Transcribing => "Transcribing...",
            Self::Refining => "Refining...",
        }
    }
}

/// The overlay clock, computed by the caller from the session clock (DICT-9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockDisplay {
    /// "m:ss" or "Stops in m:ss".
    pub text: String,
    /// Render in the warning color.
    pub warning: bool,
}

/// How a session entered Transcribing (OVL-11a, SND-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscribingEntry {
    /// The recording UI was up: play the stop cue.
    FromRecording,
    /// A retry or an unconfirmed provisional stop: the overlay appeared
    /// directly in processing mode. Hide the download overlay; no stop cue.
    Direct,
    /// Already processing; nothing to do.
    AlreadyProcessing,
}

/// What [`Overlay::tick`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeTick {
    /// Nothing expired.
    Unchanged,
    /// The notice cleared: re-sync the non-dictation UI (OVL-15).
    Cleared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionStage {
    Recording,
    Processing(ProcessingStage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveNotice {
    notice: Notice,
    expires_at: MonoTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Panel {
    Hidden,
    Session {
        stage: SessionStage,
        notice: Option<ActiveNotice>,
    },
    // up only for a notice: no session, or the session ended under it
    NoticeOnly(ActiveNotice),
}

/// Recording overlay state.
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    panel: Panel,
    levels: WaveformHistory,
    preview: String,
    silent: bool,
    target_app: Option<String>,
    input_device: Option<String>,
}

impl Default for Overlay {
    fn default() -> Self {
        Self::new()
    }
}

impl Overlay {
    /// A hidden overlay.
    pub fn new() -> Self {
        Self {
            panel: Panel::Hidden,
            levels: WaveformHistory::new(),
            preview: String::new(),
            silent: false,
            target_app: None,
            input_device: None,
        }
    }

    /// Shows the recording UI for a new session (OVL-11): resets levels,
    /// preview, labels, silent flag, target app, input device, and drops any
    /// notice without reporting it cleared.
    pub fn show_recording(&mut self, target_app: Option<String>, input_device: Option<String>) {
        self.reset();
        self.target_app = target_app;
        self.input_device = input_device;
        self.panel = Panel::Session {
            stage: SessionStage::Recording,
            notice: None,
        };
    }

    /// The session entered Transcribing (OVL-10, OVL-11a).
    ///
    /// From a shown recording, switches to processing. Otherwise (a retry,
    /// or a provisional recording stopped before confirmation) resets and
    /// shows the overlay directly in processing mode with `target_app`.
    pub fn enter_transcribing(&mut self, target_app: Option<String>) -> TranscribingEntry {
        match &mut self.panel {
            Panel::Session {
                stage: stage @ SessionStage::Recording,
                ..
            } => {
                *stage = SessionStage::Processing(ProcessingStage::Transcribing);
                self.silent = false;
                TranscribingEntry::FromRecording
            }
            Panel::Session {
                stage: SessionStage::Processing(_),
                ..
            } => TranscribingEntry::AlreadyProcessing,
            Panel::Hidden | Panel::NoticeOnly(_) => {
                self.reset();
                self.target_app = target_app;
                self.panel = Panel::Session {
                    stage: SessionStage::Processing(ProcessingStage::Transcribing),
                    notice: None,
                };
                TranscribingEntry::Direct
            }
        }
    }

    /// Cleanup started: the label becomes "Refining..." (CLN-3). Ignored
    /// without a session.
    pub fn enter_refining(&mut self) {
        if let Panel::Session { stage, .. } = &mut self.panel {
            *stage = SessionStage::Processing(ProcessingStage::Refining);
        }
    }

    /// Adds a microphone level, keeping the last
    /// [`WAVEFORM_HISTORY`](crate::level::WAVEFORM_HISTORY) (OVL-8).
    pub fn push_level(&mut self, level: f32) {
        self.levels.push(level);
    }

    /// Replaces the live preview text.
    pub fn set_preview(&mut self, text: impl Into<String>) {
        self.preview = text.into();
    }

    /// Sets whether the microphone is silent (from the silent-mic detector).
    pub fn set_silent(&mut self, silent: bool) {
        self.silent = silent;
    }

    /// Names the input device after a mid-recording rebuild, so the silent
    /// microphone title follows the switch (AUD-13).
    pub fn set_input_device(&mut self, input_device: Option<String>) {
        self.input_device = input_device;
    }

    /// The session ended. The panel hides now, or when the notice on it
    /// clears (OVL-13).
    pub fn end_session(&mut self) {
        let Panel::Session { notice, .. } = &mut self.panel else {
            return;
        };
        self.panel = match notice.take() {
            Some(active) => Panel::NoticeOnly(active),
            None => Panel::Hidden,
        };
        self.target_app = None;
    }

    /// Shows a notice for [`NOTICE_DURATION`], replacing any notice on screen
    /// and restarting the timer (OVL-12). During a session it covers the
    /// live content (OVL-13); otherwise the panel comes up just for it
    /// (OVL-14).
    pub fn show_notice(&mut self, notice: Notice, now: MonoTime) {
        let active = ActiveNotice {
            notice,
            expires_at: now + NOTICE_DURATION,
        };
        match &mut self.panel {
            Panel::Session { notice, .. } => *notice = Some(active),
            Panel::Hidden | Panel::NoticeOnly(_) => {
                self.reset();
                self.panel = Panel::NoticeOnly(active);
            }
        }
    }

    /// Clears an expired notice, hiding a panel that was up only for it.
    pub fn tick(&mut self, now: MonoTime) -> NoticeTick {
        match &mut self.panel {
            Panel::Session { notice, .. }
                if notice.as_ref().is_some_and(|n| now >= n.expires_at) =>
            {
                *notice = None;
                NoticeTick::Cleared
            }
            Panel::NoticeOnly(active) if now >= active.expires_at => {
                self.panel = Panel::Hidden;
                NoticeTick::Cleared
            }
            Panel::Hidden | Panel::Session { .. } | Panel::NoticeOnly(_) => NoticeTick::Unchanged,
        }
    }

    /// When the shell should next call [`Overlay::tick`].
    pub fn notice_deadline(&self) -> Option<MonoTime> {
        self.active_notice().map(|n| n.expires_at)
    }

    /// A notice is on screen (keeps the download overlay away, OVL-18).
    pub fn has_notice(&self) -> bool {
        self.active_notice().is_some()
    }

    /// The overlay view model. `clock` is shown only while recording.
    pub fn view(&self, clock: Option<&ClockDisplay>) -> OverlayView {
        let (visible, stage, notice) = match &self.panel {
            Panel::Hidden => (false, None, None),
            Panel::Session { stage, notice } => (true, Some(*stage), notice.as_ref()),
            Panel::NoticeOnly(active) => (true, None, Some(active)),
        };
        let content = match (notice, stage) {
            (Some(active), _) => OverlayContent::Notice(active.notice.clone()),
            (None, Some(SessionStage::Processing(processing))) => OverlayContent::Processing {
                label: processing.label(),
                preview: (!self.preview.is_empty()).then(|| self.preview.clone()),
            },
            (None, _) if self.silent => OverlayContent::SilentMicrophone {
                title: silent_mic_title(self.input_device.as_deref()),
                message: SILENT_MIC_BODY,
            },
            (None, _) if !self.preview.is_empty() => OverlayContent::Preview {
                text: self.preview.clone(),
                levels: self.levels.levels(),
            },
            (None, _) => OverlayContent::Waveform {
                levels: self.levels.levels(),
            },
        };
        let trailing = match (notice, stage) {
            (None, Some(SessionStage::Recording)) => Some(TrailingControls {
                clock: clock.cloned(),
            }),
            (None, Some(SessionStage::Processing(_))) => Some(TrailingControls { clock: None }),
            (Some(_), _) | (None, None) => None,
        };
        // OVL-6, OUT-12: a session shows its target, else the frontmost app;
        // a panel up only for a notice stands in for no app
        let icon = match (&self.panel, &self.target_app) {
            (Panel::Session { .. }, Some(app)) => OverlayIcon::App(app.clone()),
            (Panel::Session { .. }, None) => OverlayIcon::FrontmostApp,
            (Panel::NoticeOnly(_) | Panel::Hidden, _) => OverlayIcon::None,
        };
        OverlayView {
            visible,
            icon,
            content,
            trailing,
        }
    }

    fn active_notice(&self) -> Option<&ActiveNotice> {
        match &self.panel {
            Panel::Session { notice, .. } => notice.as_ref(),
            Panel::NoticeOnly(active) => Some(active),
            Panel::Hidden => None,
        }
    }

    fn reset(&mut self) {
        self.levels.reset();
        self.preview.clear();
        self.silent = false;
        self.target_app = None;
        self.input_device = None;
    }
}

/// The overlay's single content mode (OVL-5): notice > processing > silent
/// microphone > preview > waveform.
#[derive(Debug, Clone, PartialEq)]
pub enum OverlayContent {
    /// A notice with a red warning triangle (OVL-12).
    Notice(Notice),
    /// Idle dots with a sweep, a label, and the last preview (OVL-10).
    Processing {
        /// "Transcribing..." or "Refining...".
        label: &'static str,
        /// Last preview text, when there is one.
        preview: Option<String>,
    },
    /// The microphone delivers no signal (AUD-13).
    SilentMicrophone {
        /// "No sound from <device>".
        title: String,
        /// Hint line.
        message: &'static str,
    },
    /// Up to two lines of preview above a short waveform (OVL-9).
    Preview {
        /// Preview text.
        text: String,
        /// Levels, oldest first.
        levels: Vec<f32>,
    },
    /// The level meter (OVL-8).
    Waveform {
        /// Levels, oldest first.
        levels: Vec<f32>,
    },
}

/// Trailing controls while a session is active and no notice shows (OVL-7):
/// the clock (recording only) and the cancel button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrailingControls {
    /// Clock, while recording.
    pub clock: Option<ClockDisplay>,
}

/// The overlay's leading icon (OVL-6, OUT-12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayIcon {
    /// No icon.
    None,
    /// The target app's icon, by app identifier.
    App(String),
    /// The target is unknown: the frontmost app's icon, unless that app is
    /// Onda itself.
    FrontmostApp,
}

/// Everything the shell needs to draw the recording overlay
/// (`OverlayChanged`).
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayView {
    /// The panel is on screen.
    pub visible: bool,
    /// Leading icon.
    pub icon: OverlayIcon,
    /// Content mode.
    pub content: OverlayContent,
    /// Clock and cancel button, when shown.
    pub trailing: Option<TrailingControls>,
}

/// Where a panel's fade stands (OVL-4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FadeState {
    /// Off screen.
    #[default]
    Hidden,
    /// On screen, fully or fading in.
    Shown,
    /// Fading out.
    FadingOut,
}

/// What the shell should animate (OVL-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeCommand {
    /// Nothing.
    None,
    /// Reposition (OVL-1), then fade in: from transparent, or from the
    /// current opacity when interrupting a fade-out (no blink).
    FadeIn {
        /// Start from the current opacity.
        from_current: bool,
    },
    /// Fade out, then report [`PanelFade::fade_out_finished`].
    FadeOut,
    /// Take the panel off screen.
    OrderOut,
}

/// Fade bookkeeping shared by both overlay panels (OVL-4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PanelFade {
    state: FadeState,
}

impl PanelFade {
    /// A hidden panel.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current state.
    pub fn state(self) -> FadeState {
        self.state
    }

    /// Show the panel. Already shown: nothing.
    pub fn show(&mut self) -> FadeCommand {
        let from_current = match self.state {
            FadeState::Shown => return FadeCommand::None,
            FadeState::Hidden => false,
            FadeState::FadingOut => true,
        };
        self.state = FadeState::Shown;
        FadeCommand::FadeIn { from_current }
    }

    /// Hide the panel. A hide during a hide is ignored.
    pub fn hide(&mut self) -> FadeCommand {
        match self.state {
            FadeState::Shown => {
                self.state = FadeState::FadingOut;
                FadeCommand::FadeOut
            }
            FadeState::Hidden | FadeState::FadingOut => FadeCommand::None,
        }
    }

    /// The fade-out animation finished. Ignored if a show superseded it.
    pub fn fade_out_finished(&mut self) -> FadeCommand {
        match self.state {
            FadeState::FadingOut => {
                self.state = FadeState::Hidden;
                FadeCommand::OrderOut
            }
            FadeState::Hidden | FadeState::Shown => FadeCommand::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: f64) -> MonoTime {
        Duration::from_secs_f64(s)
    }

    fn notice(title: &str) -> Notice {
        Notice {
            title: title.to_owned(),
            message: "Troy Barnes has to wait a moment.".to_owned(),
        }
    }

    fn recording() -> Overlay {
        let mut overlay = Overlay::new();
        overlay.show_recording(
            Some("edu.greendale.messenger".to_owned()),
            Some("MacBook Pro Microphone".to_owned()),
        );
        overlay
    }

    #[derive(Debug)]
    enum Step {
        ShowRecording,
        EnterTranscribing,
        EndSession,
        Notice(&'static str, f64),
        Tick(f64),
    }

    fn apply(overlay: &mut Overlay, step: &Step) -> Option<NoticeTick> {
        match step {
            Step::ShowRecording => overlay.show_recording(None, None),
            Step::EnterTranscribing => {
                overlay.enter_transcribing(None);
            }
            Step::EndSession => overlay.end_session(),
            Step::Notice(title, at) => overlay.show_notice(notice(title), secs(*at)),
            Step::Tick(at) => return Some(overlay.tick(secs(*at))),
        }
        None
    }

    #[test]
    fn ovl_12_to_15_notice_lifecycle() {
        struct Case {
            name: &'static str,
            steps: Vec<Step>,
            want_visible: bool,
            want_notice: Option<&'static str>,
            want_cleared: usize,
        }
        let cases = [
            Case {
                name: "OVL-14 idle notice shows the panel until it clears",
                steps: vec![Step::Notice("Model Not Ready", 0.0), Step::Tick(3.9)],
                want_visible: true,
                want_notice: Some("Model Not Ready"),
                want_cleared: 0,
            },
            Case {
                name: "OVL-12 notice clears after 4 s and hides its panel",
                steps: vec![Step::Notice("Model Not Ready", 0.0), Step::Tick(4.0)],
                want_visible: false,
                want_notice: None,
                want_cleared: 1,
            },
            Case {
                name: "OVL-12 replacing restarts the timer",
                steps: vec![
                    Step::Notice("Paste Failed", 0.0),
                    Step::Notice("Copy Failed", 3.0),
                    Step::Tick(4.5),
                ],
                want_visible: true,
                want_notice: Some("Copy Failed"),
                want_cleared: 0,
            },
            Case {
                name: "OVL-12 replaced notice clears on the new timer",
                steps: vec![
                    Step::Notice("Paste Failed", 0.0),
                    Step::Notice("Copy Failed", 3.0),
                    Step::Tick(7.0),
                ],
                want_visible: false,
                want_notice: None,
                want_cleared: 1,
            },
            Case {
                name: "OVL-13 mid-session notice keeps the session overlay",
                steps: vec![
                    Step::ShowRecording,
                    Step::Notice("Clipboard Restore Failed", 1.0),
                    Step::Tick(5.0),
                ],
                want_visible: true,
                want_notice: None,
                want_cleared: 1,
            },
            Case {
                name: "OVL-13 session end waits for the notice",
                steps: vec![
                    Step::ShowRecording,
                    Step::Notice("Used Raw Transcript", 1.0),
                    Step::EndSession,
                    Step::Tick(4.0),
                ],
                want_visible: true,
                want_notice: Some("Used Raw Transcript"),
                want_cleared: 0,
            },
            Case {
                name: "OVL-13 deferred hide completes when the notice clears",
                steps: vec![
                    Step::ShowRecording,
                    Step::Notice("Used Raw Transcript", 1.0),
                    Step::EndSession,
                    Step::Tick(5.0),
                ],
                want_visible: false,
                want_notice: None,
                want_cleared: 1,
            },
            Case {
                name: "notice after session end keeps the panel until it clears",
                steps: vec![
                    Step::ShowRecording,
                    Step::EndSession,
                    Step::Notice("No Speech Detected", 1.0),
                    Step::Tick(5.0),
                ],
                want_visible: false,
                want_notice: None,
                want_cleared: 1,
            },
            Case {
                name: "OVL-14 session start drops the idle notice without a clear",
                steps: vec![
                    Step::Notice("Still Transcribing", 0.0),
                    Step::ShowRecording,
                    Step::Tick(4.0),
                ],
                want_visible: true,
                want_notice: None,
                want_cleared: 0,
            },
            Case {
                name: "stale deferred hide does not hide the next session",
                steps: vec![
                    Step::ShowRecording,
                    Step::Notice("Session A", 0.0),
                    Step::EndSession,
                    Step::ShowRecording,
                    Step::Notice("Session B", 1.0),
                    Step::Tick(5.0),
                ],
                want_visible: true,
                want_notice: None,
                want_cleared: 1,
            },
            Case {
                name: "OVL-11a direct processing takes over an idle notice",
                steps: vec![
                    Step::Notice("Model Download Failed", 0.0),
                    Step::EnterTranscribing,
                    Step::Tick(4.0),
                ],
                want_visible: true,
                want_notice: None,
                want_cleared: 0,
            },
            Case {
                name: "hidden overlay ticks nothing",
                steps: vec![Step::Tick(10.0)],
                want_visible: false,
                want_notice: None,
                want_cleared: 0,
            },
        ];
        for tc in cases {
            let mut overlay = Overlay::new();
            let mut cleared = 0;
            for step in &tc.steps {
                if apply(&mut overlay, step) == Some(NoticeTick::Cleared) {
                    cleared += 1;
                }
            }
            let view = overlay.view(None);
            assert_eq!(view.visible, tc.want_visible, "{}", tc.name);
            let got_notice = match &view.content {
                OverlayContent::Notice(n) => Some(n.title.as_str()),
                _ => None,
            };
            assert_eq!(got_notice, tc.want_notice, "{}", tc.name);
            assert_eq!(
                overlay.has_notice(),
                tc.want_notice.is_some(),
                "{}",
                tc.name
            );
            assert_eq!(cleared, tc.want_cleared, "{}", tc.name);
        }
    }

    #[test]
    fn ovl_12_notice_deadline() {
        let mut overlay = Overlay::new();
        assert_eq!(overlay.notice_deadline(), None, "no notice");
        overlay.show_notice(notice("Paste Failed"), secs(10.0));
        assert_eq!(overlay.notice_deadline(), Some(secs(14.0)), "first notice");
        overlay.show_notice(notice("Copy Failed"), secs(12.0));
        assert_eq!(
            overlay.notice_deadline(),
            Some(secs(16.0)),
            "replacement restarts"
        );
    }

    #[test]
    fn ovl_5_content_priority() {
        struct Case {
            name: &'static str,
            setup: fn(&mut Overlay),
            want: OverlayContent,
        }
        let cases = [
            Case {
                name: "waveform before any preview",
                setup: |o| o.push_level(0.1),
                want: OverlayContent::Waveform { levels: vec![0.1] },
            },
            Case {
                name: "preview when text exists",
                setup: |o| {
                    o.push_level(0.2);
                    o.set_preview("Troy Barnes studies at Greendale");
                },
                want: OverlayContent::Preview {
                    text: "Troy Barnes studies at Greendale".to_owned(),
                    levels: vec![0.2],
                },
            },
            Case {
                name: "silent microphone beats preview",
                setup: |o| {
                    o.set_preview("Troy Barnes");
                    o.set_silent(true);
                },
                want: OverlayContent::SilentMicrophone {
                    title: "No sound from MacBook Pro Microphone".to_owned(),
                    message: "Check that it's unmuted, or pick another input in Settings.",
                },
            },
            Case {
                name: "processing beats silent microphone and keeps the preview",
                setup: |o| {
                    o.set_preview("Troy Barnes studies at Greendale Community College");
                    o.enter_transcribing(None);
                    o.set_silent(true);
                },
                want: OverlayContent::Processing {
                    label: "Transcribing...",
                    preview: Some("Troy Barnes studies at Greendale Community College".to_owned()),
                },
            },
            Case {
                name: "CLN-3 refining label",
                setup: |o| {
                    o.enter_transcribing(None);
                    o.enter_refining();
                },
                want: OverlayContent::Processing {
                    label: "Refining...",
                    preview: None,
                },
            },
            Case {
                name: "notice beats processing",
                setup: |o| {
                    o.enter_transcribing(None);
                    o.show_notice(notice("Couldn't Refine Transcript"), secs(0.0));
                },
                want: OverlayContent::Notice(notice("Couldn't Refine Transcript")),
            },
        ];
        for tc in cases {
            let mut overlay = recording();
            (tc.setup)(&mut overlay);
            assert_eq!(overlay.view(None).content, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn silent_microphone_without_device_name() {
        let mut overlay = Overlay::new();
        overlay.show_recording(None, None);
        overlay.set_silent(true);
        let want = OverlayContent::SilentMicrophone {
            title: "No sound from the microphone".to_owned(),
            message: SILENT_MIC_BODY,
        };
        assert_eq!(overlay.view(None).content, want);
    }

    #[test]
    fn ovl_7_trailing_controls() {
        struct Case {
            name: &'static str,
            setup: fn(&mut Overlay),
            want: Option<TrailingControls>,
        }
        let clock = ClockDisplay {
            text: "0:07".to_owned(),
            warning: false,
        };
        let cases = [
            Case {
                name: "recording shows clock and cancel",
                setup: |_| {},
                want: Some(TrailingControls {
                    clock: Some(ClockDisplay {
                        text: "0:07".to_owned(),
                        warning: false,
                    }),
                }),
            },
            Case {
                name: "OVL-10 processing hides the clock",
                setup: |o| {
                    o.enter_transcribing(None);
                },
                want: Some(TrailingControls { clock: None }),
            },
            Case {
                name: "notice hides trailing controls",
                setup: |o| o.show_notice(notice("Paste Failed"), secs(0.0)),
                want: None,
            },
            Case {
                name: "notice-only panel has no controls",
                setup: |o| {
                    o.show_notice(notice("Paste Failed"), secs(0.0));
                    o.end_session();
                },
                want: None,
            },
            Case {
                name: "trailing controls return after the notice clears",
                setup: |o| {
                    o.show_notice(notice("Paste Failed"), secs(0.0));
                    o.tick(secs(4.0));
                },
                want: Some(TrailingControls {
                    clock: Some(ClockDisplay {
                        text: "0:07".to_owned(),
                        warning: false,
                    }),
                }),
            },
        ];
        for tc in cases {
            let mut overlay = recording();
            (tc.setup)(&mut overlay);
            assert_eq!(overlay.view(Some(&clock)).trailing, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn ovl_11_new_session_resets() {
        let mut overlay = recording();
        overlay.push_level(0.4);
        overlay.set_preview("Six seasons and a movie");
        overlay.set_silent(true);
        overlay.enter_transcribing(None);
        overlay.enter_refining();
        overlay.end_session();

        overlay.show_recording(Some("edu.greendale.notes".to_owned()), None);

        let want = OverlayView {
            visible: true,
            icon: OverlayIcon::App("edu.greendale.notes".to_owned()),
            content: OverlayContent::Waveform { levels: vec![] },
            trailing: Some(TrailingControls { clock: None }),
        };
        assert_eq!(overlay.view(None), want);
    }

    #[test]
    fn ovl_11a_transcribing_entry() {
        struct Case {
            name: &'static str,
            setup: fn(&mut Overlay),
            want: TranscribingEntry,
            want_icon: OverlayIcon,
        }
        let cases = [
            Case {
                name: "from a shown recording",
                setup: |o| o.show_recording(Some("edu.greendale.messenger".to_owned()), None),
                want: TranscribingEntry::FromRecording,
                want_icon: OverlayIcon::App("edu.greendale.messenger".to_owned()),
            },
            Case {
                name: "retry with no recording",
                setup: |_| {},
                want: TranscribingEntry::Direct,
                want_icon: OverlayIcon::App("edu.greendale.retry".to_owned()),
            },
            Case {
                name: "already processing",
                setup: |o| {
                    o.show_recording(None, None);
                    o.enter_transcribing(None);
                },
                want: TranscribingEntry::AlreadyProcessing,
                want_icon: OverlayIcon::FrontmostApp,
            },
            Case {
                name: "during an idle notice",
                setup: |o| o.show_notice(notice("Paste Failed"), secs(0.0)),
                want: TranscribingEntry::Direct,
                want_icon: OverlayIcon::App("edu.greendale.retry".to_owned()),
            },
        ];
        for tc in cases {
            let mut overlay = Overlay::new();
            (tc.setup)(&mut overlay);
            let got = overlay.enter_transcribing(Some("edu.greendale.retry".to_owned()));
            assert_eq!(got, tc.want, "{}", tc.name);
            let view = overlay.view(None);
            assert!(view.visible, "{}", tc.name);
            assert!(
                matches!(
                    view.content,
                    OverlayContent::Processing {
                        label: "Transcribing...",
                        ..
                    }
                ),
                "{}",
                tc.name
            );
            assert_eq!(view.icon, tc.want_icon, "{}", tc.name);
        }
    }

    #[test]
    fn ovl_8_level_history() {
        let mut overlay = recording();
        for i in 0..70 {
            overlay.push_level(i as f32);
        }
        let OverlayContent::Waveform { levels } = overlay.view(None).content else {
            panic!("want waveform");
        };
        assert_eq!(levels.len(), crate::level::WAVEFORM_HISTORY);
        assert_eq!(levels.first(), Some(&10.0));
        assert_eq!(levels.last(), Some(&69.0));
    }

    #[test]
    fn ovl_4_fade_bookkeeping() {
        #[derive(Clone, Copy)]
        enum Op {
            Show,
            Hide,
            Finished,
        }
        struct Case {
            name: &'static str,
            ops: Vec<Op>,
            want_last: FadeCommand,
            want_state: FadeState,
        }
        let cases = [
            Case {
                name: "fresh show fades in from transparent",
                ops: vec![Op::Show],
                want_last: FadeCommand::FadeIn {
                    from_current: false,
                },
                want_state: FadeState::Shown,
            },
            Case {
                name: "show while visible does nothing",
                ops: vec![Op::Show, Op::Show],
                want_last: FadeCommand::None,
                want_state: FadeState::Shown,
            },
            Case {
                name: "hide fades out",
                ops: vec![Op::Show, Op::Hide],
                want_last: FadeCommand::FadeOut,
                want_state: FadeState::FadingOut,
            },
            Case {
                name: "hide during a hide is ignored",
                ops: vec![Op::Show, Op::Hide, Op::Hide],
                want_last: FadeCommand::None,
                want_state: FadeState::FadingOut,
            },
            Case {
                name: "show interrupting a hide fades up from current",
                ops: vec![Op::Show, Op::Hide, Op::Show],
                want_last: FadeCommand::FadeIn { from_current: true },
                want_state: FadeState::Shown,
            },
            Case {
                name: "finished fade orders out",
                ops: vec![Op::Show, Op::Hide, Op::Finished],
                want_last: FadeCommand::OrderOut,
                want_state: FadeState::Hidden,
            },
            Case {
                name: "superseded fade does not order out",
                ops: vec![Op::Show, Op::Hide, Op::Show, Op::Finished],
                want_last: FadeCommand::None,
                want_state: FadeState::Shown,
            },
            Case {
                name: "hide while hidden does nothing",
                ops: vec![Op::Hide],
                want_last: FadeCommand::None,
                want_state: FadeState::Hidden,
            },
        ];
        for tc in cases {
            let mut fade = PanelFade::new();
            let mut last = FadeCommand::None;
            for op in &tc.ops {
                last = match op {
                    Op::Show => fade.show(),
                    Op::Hide => fade.hide(),
                    Op::Finished => fade.fade_out_finished(),
                };
            }
            assert_eq!(last, tc.want_last, "{}", tc.name);
            assert_eq!(fade.state(), tc.want_state, "{}", tc.name);
        }
    }
}
