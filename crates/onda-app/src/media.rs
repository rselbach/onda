//! Media pause ownership across sessions (MEDIA-1..MEDIA-5).
//!
//! One task owns the ownership state and handles session begin/end in
//! order. Port calls run one at a time on blocking threads (MEDIA-5); a
//! guard moves into each call, so a 500 ms query that the core abandoned
//! still holds the queue until the shell returns. Before each decision the
//! task looks at commands already queued behind the current one, which is
//! how the MEDIA-3 races between back-to-back sessions resolve.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::spawn_blocking;

use crate::ports::{MediaControl, NowPlaying};
use crate::session::SessionId;

/// The playing query gives up after this long and counts as not playing
/// (MEDIA-1).
pub(crate) const QUERY_TIMEOUT: Duration = Duration::from_millis(500);

/// Delay before checking that a pause took (MEDIA-1).
pub(crate) const PAUSE_VERIFICATION_DELAY: Duration = Duration::from_millis(150);

/// Whether a now-playing report means media is playing (MEDIA-1): the
/// playing flag when reported, else a playback rate above zero. A failed
/// query, no track, and a track without either fact count as not playing.
pub(crate) fn is_playing(report: &NowPlaying) -> bool {
    match report {
        NowPlaying::Failed { reason } => {
            tracing::warn!(%reason, "media query failed; treating media as not playing");
            false
        }
        NowPlaying::NoTrack => {
            tracing::info!("no now-playing track");
            false
        }
        NowPlaying::Track {
            is_playing,
            playback_rate,
        } => is_playing.unwrap_or_else(|| playback_rate.is_some_and(|rate| rate > 0.0)),
    }
}

enum Command {
    Begin(SessionId),
    End(SessionId),
    Flush(oneshot::Sender<()>),
}

/// Handle to the media task.
#[derive(Clone)]
pub(crate) struct Media {
    commands: mpsc::UnboundedSender<Command>,
}

impl Media {
    pub(crate) fn spawn(port: Arc<dyn MediaControl>) -> Self {
        let (commands, inbox) = mpsc::unbounded_channel();
        let worker = Worker {
            port,
            calls: Arc::new(Mutex::new(())),
            inbox,
            queued: VecDeque::new(),
            paused: false,
        };
        tokio::spawn(worker.run());
        Self { commands }
    }

    /// A confirmed session started: pause media if it is playing.
    pub(crate) fn begin(&self, session: SessionId) {
        let _ = self.commands.send(Command::Begin(session));
    }

    /// A session that began ended: resume media if it paused it.
    pub(crate) fn end(&self, session: SessionId) {
        let _ = self.commands.send(Command::End(session));
    }

    /// Waits until every command sent so far was handled.
    pub(crate) async fn flush(&self) {
        let (done, wait) = oneshot::channel();
        if self.commands.send(Command::Flush(done)).is_ok() {
            let _ = wait.await;
        }
    }
}

struct Worker {
    port: Arc<dyn MediaControl>,
    calls: Arc<Mutex<()>>,
    inbox: mpsc::UnboundedReceiver<Command>,
    queued: VecDeque<Command>,
    // media is paused because of us; the next session to begin inherits it
    paused: bool,
}

impl Worker {
    async fn run(mut self) {
        loop {
            let command = match self.queued.pop_front() {
                Some(command) => command,
                None => match self.inbox.recv().await {
                    Some(command) => command,
                    None => return,
                },
            };
            match command {
                Command::Begin(session) => self.begin(session).await,
                Command::End(session) => self.end(session).await,
                Command::Flush(done) => {
                    let _ = done.send(());
                }
            }
        }
    }

    async fn begin(&mut self, session: SessionId) {
        if self.paused {
            // a previous session's pause is still in effect: this session
            // owns it now and must not query or pause again (MEDIA-3)
            return;
        }
        let playing = self.query().await;
        if self.ended_already(session) || !playing {
            // a query that returns after its session ended never pauses
            return;
        }
        if !self.call(|port| port.pause()).await {
            self.paused = self.call(|port| port.play_pause_key()).await;
            return;
        }
        self.paused = true;
        tokio::time::sleep(PAUSE_VERIFICATION_DELAY).await;
        if self.ended_already(session) {
            return;
        }
        if self.query().await {
            tracing::info!("media still playing after pause; sending the play/pause key");
            self.call(|port| port.play_pause_key()).await;
        }
    }

    async fn end(&mut self, _session: SessionId) {
        if !self.paused {
            return;
        }
        if self.next_session_begins() {
            // hand our pause to the session that is already starting
            return;
        }
        self.paused = false;
        if !self.call(|port| port.play()).await {
            tracing::warn!("could not resume media");
            return;
        }
        if self.next_session_begins() {
            // the play landed after a new session started: pause again for it
            self.paused = self.call(|port| port.pause()).await;
        }
    }

    async fn query(&self) -> bool {
        let port = self.port.clone();
        let guard = self.calls.clone().lock_owned().await;
        let call = spawn_blocking(move || {
            let _guard = guard;
            is_playing(&port.now_playing())
        });
        match tokio::time::timeout(QUERY_TIMEOUT, call).await {
            Ok(Ok(playing)) => playing,
            Ok(Err(error)) => {
                tracing::warn!(%error, "media query failed");
                false
            }
            Err(_) => {
                tracing::warn!("media query timed out; treating media as not playing");
                false
            }
        }
    }

    async fn call(&self, op: impl FnOnce(&dyn MediaControl) -> bool + Send + 'static) -> bool {
        let port = self.port.clone();
        let guard = self.calls.clone().lock_owned().await;
        spawn_blocking(move || {
            let _guard = guard;
            op(port.as_ref())
        })
        .await
        .unwrap_or(false)
    }

    fn drain(&mut self) {
        while let Ok(command) = self.inbox.try_recv() {
            self.queued.push_back(command);
        }
    }

    fn ended_already(&mut self, session: SessionId) -> bool {
        self.drain();
        self.queued
            .iter()
            .any(|command| matches!(command, Command::End(ended) if *ended == session))
    }

    fn next_session_begins(&mut self) -> bool {
        self.drain();
        self.queued
            .iter()
            .any(|command| matches!(command, Command::Begin(_)))
    }
}

#[cfg(test)]
mod tests {
    //! Ports Jabber's MediaPlaybackServiceTests.

    use super::*;
    use std::sync::{Mutex as StdMutex, PoisonError, mpsc as std_mpsc};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Sent {
        Pause,
        Play,
        Key,
    }

    #[derive(Default)]
    struct Fake {
        playing: StdMutex<VecDeque<bool>>,
        playing_default: bool,
        queries: StdMutex<usize>,
        sent: StdMutex<Vec<Sent>>,
        pause_result: Option<bool>,
        play_result: Option<bool>,
        key_result: Option<bool>,
        hold_query: StdMutex<Option<std_mpsc::Receiver<bool>>>,
        hold_send: StdMutex<Option<std_mpsc::Receiver<bool>>>,
    }

    fn lock<T>(m: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }

    impl Fake {
        fn playing(results: &[bool]) -> Self {
            Self {
                playing: StdMutex::new(results.iter().copied().collect()),
                ..Self::default()
            }
        }

        fn sent(&self) -> Vec<Sent> {
            lock(&self.sent).clone()
        }

        fn send(&self, what: Sent, result: Option<bool>) -> bool {
            lock(&self.sent).push(what);
            let held = lock(&self.hold_send).take();
            if let Some(release) = held {
                return release.recv().unwrap_or(true);
            }
            result.unwrap_or(true)
        }
    }

    impl MediaControl for Fake {
        fn now_playing(&self) -> NowPlaying {
            *lock(&self.queries) += 1;
            let held = lock(&self.hold_query).take();
            let playing = match held {
                Some(release) => release.recv().unwrap_or(false),
                None => lock(&self.playing)
                    .pop_front()
                    .unwrap_or(self.playing_default),
            };
            NowPlaying::Track {
                is_playing: Some(playing),
                playback_rate: None,
            }
        }

        fn pause(&self) -> bool {
            self.send(Sent::Pause, self.pause_result)
        }

        fn play(&self) -> bool {
            self.send(Sent::Play, self.play_result)
        }

        fn play_pause_key(&self) -> bool {
            lock(&self.sent).push(Sent::Key);
            self.key_result.unwrap_or(true)
        }
    }

    #[test]
    fn media_1_playing_rule() {
        let track = |is_playing, playback_rate| NowPlaying::Track {
            is_playing,
            playback_rate,
        };
        let cases = [
            ("flag says playing", track(Some(true), None), true),
            (
                "flag wins over a zero rate",
                track(Some(true), Some(0.0)),
                true,
            ),
            (
                "flag says paused despite a rate",
                track(Some(false), Some(1.0)),
                false,
            ),
            ("no flag, positive rate", track(None, Some(1.0)), true),
            ("no flag, zero rate", track(None, Some(0.0)), false),
            ("no flag, negative rate", track(None, Some(-1.0)), false),
            ("neither fact", track(None, None), false),
            ("no track", NowPlaying::NoTrack, false),
            (
                "query failed",
                NowPlaying::Failed {
                    reason: "helper timed out and was stopped".to_owned(),
                },
                false,
            ),
        ];
        for (name, report, want) in cases {
            assert_eq!(is_playing(&report), want, "{name}");
        }
    }

    const A: SessionId = SessionId(1);
    const B: SessionId = SessionId(2);

    async fn settle() {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    async fn eventually(what: &str, cond: impl Fn() -> bool) {
        for _ in 0..2_000 {
            if cond() {
                return;
            }
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("timed out waiting for {what}");
    }

    #[tokio::test(start_paused = true)]
    async fn pause_and_resume_outcomes() {
        use Sent::{Key, Pause, Play};
        struct Case {
            name: &'static str,
            fake: Fake,
            ends: usize,
            want: Vec<Sent>,
        }
        let cases = [
            Case {
                name: "not playing: no commands",
                fake: Fake::playing(&[false]),
                ends: 1,
                want: vec![],
            },
            Case {
                name: "resumes only after a successful pause",
                fake: Fake::playing(&[true, false]),
                ends: 1,
                want: vec![Pause, Play],
            },
            Case {
                name: "failed pause and failed key: nothing to resume",
                fake: Fake {
                    pause_result: Some(false),
                    key_result: Some(false),
                    ..Fake::playing(&[true])
                },
                ends: 1,
                want: vec![Pause, Key],
            },
            Case {
                name: "failed pause falls back to the key directly",
                fake: Fake {
                    pause_result: Some(false),
                    ..Fake::playing(&[true])
                },
                ends: 1,
                want: vec![Pause, Key, Play],
            },
            Case {
                name: "still playing after pause: key fallback, resume with play",
                fake: Fake::playing(&[true, true]),
                ends: 1,
                want: vec![Pause, Key, Play],
            },
            Case {
                name: "a failed play is not retried",
                fake: Fake {
                    play_result: Some(false),
                    ..Fake::playing(&[true, false])
                },
                ends: 2,
                want: vec![Pause, Play],
            },
        ];
        for tc in cases {
            let fake = Arc::new(tc.fake);
            let media = Media::spawn(fake.clone());
            media.begin(A);
            settle().await;
            for _ in 0..tc.ends {
                media.end(A);
            }
            media.flush().await;
            assert_eq!(fake.sent(), tc.want, "{}", tc.name);
        }
    }

    // testFinishingBeforePlaybackQueryReturnsPreventsLatePause
    #[tokio::test(start_paused = true)]
    async fn media_3_query_returning_after_the_session_ended_never_pauses() {
        let (release, held) = std_mpsc::channel();
        let fake = Arc::new(Fake::default());
        *lock(&fake.hold_query) = Some(held);
        let media = Media::spawn(fake.clone());
        media.begin(A);
        eventually("the query", || *lock(&fake.queries) == 1).await;
        media.end(A);
        let _ = release.send(true);
        media.flush().await;
        assert_eq!(fake.sent(), vec![]);
    }

    #[tokio::test(start_paused = true)]
    async fn media_1_a_query_slower_than_500_ms_counts_as_not_playing() {
        let (release, held) = std_mpsc::channel();
        let fake = Arc::new(Fake::default());
        *lock(&fake.hold_query) = Some(held);
        let media = Media::spawn(fake.clone());
        media.begin(A);
        eventually("the query", || *lock(&fake.queries) == 1).await;
        tokio::time::advance(QUERY_TIMEOUT + Duration::from_millis(1)).await;
        // the abandoned query still occupies the queue until it returns
        // (MEDIA-5); afterwards nothing was paused
        let _ = release.send(true);
        media.end(A);
        media.flush().await;
        assert_eq!(fake.sent(), vec![]);
    }

    // testFinishingDuringPauseCommandUnpausesStaleSession
    #[tokio::test(start_paused = true)]
    async fn media_3_pause_landing_after_its_session_ended_is_undone() {
        let (release, held) = std_mpsc::channel();
        let fake = Arc::new(Fake::playing(&[true]));
        *lock(&fake.hold_send) = Some(held);
        let media = Media::spawn(fake.clone());
        media.begin(A);
        eventually("the pause", || fake.sent() == vec![Sent::Pause]).await;
        media.end(A);
        let _ = release.send(true);
        media.flush().await;
        assert_eq!(fake.sent(), vec![Sent::Pause, Sent::Play]);
    }

    // testStaleResumeAbortsWhenNewSessionStartedBeforePlayLands and
    // testAbortedStaleResumeHandsPauseOwnershipToNewSession
    #[tokio::test(start_paused = true)]
    async fn media_3_resume_hands_ownership_to_a_session_already_starting() {
        let fake = Arc::new(Fake::playing(&[true, false, false]));
        let media = Media::spawn(fake.clone());
        media.begin(A);
        settle().await;
        media.end(A);
        media.begin(B);
        media.flush().await;
        assert_eq!(fake.sent(), vec![Sent::Pause], "no play between sessions");
        media.end(B);
        media.flush().await;
        assert_eq!(fake.sent(), vec![Sent::Pause, Sent::Play]);
    }

    // testPauseLandingAfterNewSessionStartsTransfersOwnershipInsteadOfUnpausing
    #[tokio::test(start_paused = true)]
    async fn media_3_pause_landing_after_a_new_session_started_is_kept() {
        let (release, held) = std_mpsc::channel();
        let fake = Arc::new(Fake::playing(&[true, false]));
        *lock(&fake.hold_send) = Some(held);
        let media = Media::spawn(fake.clone());
        media.begin(A);
        eventually("the pause", || fake.sent() == vec![Sent::Pause]).await;
        media.end(A);
        media.begin(B);
        let _ = release.send(true);
        media.flush().await;
        assert_eq!(fake.sent(), vec![Sent::Pause]);
        media.end(B);
        media.flush().await;
        assert_eq!(fake.sent(), vec![Sent::Pause, Sent::Play]);
    }

    // testStaleResumeRepausesWhenNewSessionStartsDuringPlay
    #[tokio::test(start_paused = true)]
    async fn media_3_play_landing_after_a_new_session_started_pauses_again() {
        let fake = Arc::new(Fake::playing(&[true, false]));
        let media = Media::spawn(fake.clone());
        media.begin(A);
        settle().await;
        let (release, held) = std_mpsc::channel();
        *lock(&fake.hold_send) = Some(held);
        media.end(A);
        eventually("the play", || fake.sent().len() == 2).await;
        media.begin(B);
        let _ = release.send(true);
        media.flush().await;
        assert_eq!(fake.sent(), vec![Sent::Pause, Sent::Play, Sent::Pause]);
        media.end(B);
        media.flush().await;
        assert_eq!(
            fake.sent(),
            vec![Sent::Pause, Sent::Play, Sent::Pause, Sent::Play],
            "B owns the re-pause"
        );
    }
}
