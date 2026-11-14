//! Media pause and output mute while dictating (MEDIA-1, MEDIA-5).
//!
//! Pause, play, and the play/pause toggle go to the MPRIS players on the
//! session bus. Audio that no player owns is silenced by muting the default
//! sink through `pactl`, which serves PulseAudio and PipeWire alike. The
//! core decides when each call happens and runs them one at a time.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use gtk::{gio, glib, glib::prelude::*};
use onda_app::ports::{MediaControl, NowPlaying};
use serde::{Deserialize, de::DeserializeOwned};

use super::process;

const PLAYER_PREFIX: &str = "org.mpris.MediaPlayer2.";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";

/// How long one player may take to answer. A player that hangs costs this
/// much and is then left out.
const CALL_TIMEOUT: Duration = Duration::from_millis(100);

/// How long the bus work of one port call may take in total. The core gives
/// the playing query 500 ms.
const BUS_BUDGET: Duration = Duration::from_millis(400);

const PACTL_TIMEOUT: Duration = Duration::from_secs(1);

pub struct Media {
    /// bus names of the players this session paused; `play` resumes these
    /// and no others
    paused: Mutex<BTreeSet<String>>,
    mutes: MuteRecord,
}

/// Media control for the data folder `data`. A mute that an earlier run
/// left behind is undone before this returns.
pub fn start(data: &Path) -> Arc<Media> {
    let mutes = MuteRecord(data.join("muted-output"));
    mutes.restore();
    Arc::new(Media {
        paused: Mutex::default(),
        mutes,
    })
}

impl MediaControl for Media {
    fn now_playing(&self) -> NowPlaying {
        let players = match Bus::open().and_then(|bus| bus.players()) {
            Ok(players) => players,
            Err(error) => {
                return NowPlaying::Failed {
                    reason: format!("{error:#}"),
                };
            }
        };
        for player in &players {
            // LOG-2: the player and its state only, never the track
            tracing::info!(player = %player.name, playback = ?player.playback, "media player");
        }
        report(&players)
    }

    fn pause(&self) -> bool {
        self.silence("Pause")
    }

    fn play(&self) -> bool {
        let ours = std::mem::take(&mut *self.paused.lock().unwrap_or_else(|e| e.into_inner()));
        if ours.is_empty() {
            return true;
        }
        let Some((bus, players)) = reach_players() else {
            return false;
        };
        let mut resumed = true;
        // a player the user stopped or restarted meanwhile is left alone
        for player in players
            .iter()
            .filter(|player| player.playback == Playback::Paused && ours.contains(&player.name))
        {
            resumed &= bus.send(&player.name, "Play");
        }
        resumed
    }

    // Linux has no media key that reaches every player, so the toggle goes
    // to each player that is still playing
    fn play_pause_key(&self) -> bool {
        self.silence("PlayPause")
    }

    fn mute_audible_output(&self) -> Option<u32> {
        match self.mutes.mute_default_sink() {
            Ok(muted) => muted,
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "could not mute the audio output");
                None
            }
        }
    }

    fn unmute_output(&self, device: u32) -> bool {
        match self.mutes.unmute(device) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "could not unmute the audio output");
                false
            }
        }
    }
}

impl Media {
    /// Sends `command` to every playing player and remembers the ones that
    /// took it. False when none did.
    fn silence(&self, command: &str) -> bool {
        let Some((bus, players)) = reach_players() else {
            return false;
        };
        let mut paused = self.paused.lock().unwrap_or_else(|e| e.into_inner());
        let mut sent = false;
        for player in players.into_iter().filter(Player::is_playing) {
            if bus.send(&player.name, command) {
                paused.insert(player.name);
                sent = true;
            }
        }
        sent
    }
}

/// the session bus and the players on it; a failure is logged
fn reach_players() -> Option<(Bus, Vec<Player>)> {
    let reached = Bus::open().and_then(|bus| {
        let players = bus.players()?;
        Ok((bus, players))
    });
    match reached {
        Ok(reached) => Some(reached),
        Err(error) => {
            tracing::warn!(error = %format!("{error:#}"), "could not reach the media players");
            None
        }
    }
}

/// What the players say together. One that is playing settles it. Short of
/// that, a player that did not answer leaves the question open, and the
/// core then neither pauses nor mutes.
fn report(players: &[Player]) -> NowPlaying {
    if players.iter().any(Player::is_playing) {
        return NowPlaying::Track {
            is_playing: Some(true),
            playback_rate: None,
        };
    }
    if let Some(silent) = players
        .iter()
        .find(|player| player.playback == Playback::Unknown)
    {
        return NowPlaying::Failed {
            reason: format!("{} did not report its state", silent.name),
        };
    }
    if players.is_empty() {
        return NowPlaying::NoTrack;
    }
    NowPlaying::Track {
        is_playing: Some(false),
        playback_rate: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Playback {
    Playing,
    Paused,
    Stopped,
    /// the player did not answer
    Unknown,
}

struct Player {
    name: String,
    playback: Playback,
}

impl Player {
    fn is_playing(&self) -> bool {
        self.playback == Playback::Playing
    }
}

/// The session bus for one port call, which ends by its deadline however
/// many players hang.
struct Bus {
    connection: gio::DBusConnection,
    deadline: Instant,
}

impl Bus {
    fn open() -> anyhow::Result<Self> {
        Ok(Self {
            connection: gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?,
            deadline: Instant::now() + BUS_BUDGET,
        })
    }

    /// The MPRIS players, with the state each reports.
    fn players(&self) -> anyhow::Result<Vec<Player>> {
        let reply = self.call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ListNames",
            None,
        )?;
        let (names,) = reply
            .get::<(Vec<String>,)>()
            .context("unexpected ListNames reply")?;
        let mut players = Vec::new();
        for name in names
            .into_iter()
            .filter(|name| name.starts_with(PLAYER_PREFIX))
        {
            let playback = self.playback(&name).unwrap_or_else(|error| {
                tracing::info!(player = %name, error = %format!("{error:#}"), "media player did not report its state");
                Playback::Unknown
            });
            players.push(Player { name, playback });
        }
        Ok(players)
    }

    fn playback(&self, player: &str) -> anyhow::Result<Playback> {
        let reply = self.call(
            player,
            PLAYER_PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&(PLAYER_INTERFACE, "PlaybackStatus").to_variant()),
        )?;
        let (status,) = reply
            .get::<(glib::Variant,)>()
            .context("unexpected PlaybackStatus reply")?;
        Ok(match status.str() {
            Some("Playing") => Playback::Playing,
            Some("Paused") => Playback::Paused,
            _ => Playback::Stopped,
        })
    }

    /// sends a player command that takes no arguments; false when it failed
    fn send(&self, player: &str, command: &str) -> bool {
        match self.call(player, PLAYER_PATH, PLAYER_INTERFACE, command, None) {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(%player, command, error = %format!("{error:#}"), "media command failed");
                false
            }
        }
    }

    fn call(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        method: &str,
        parameters: Option<&glib::Variant>,
    ) -> anyhow::Result<glib::Variant> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        anyhow::ensure!(!left.is_zero(), "{method} on {destination}: out of time");
        let timeout = left.min(CALL_TIMEOUT).as_millis().max(1) as i32;
        Ok(self.connection.call_sync(
            Some(destination),
            path,
            interface,
            method,
            parameters,
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            timeout,
            gio::Cancellable::NONE,
        )?)
    }
}

/// A sink as `pactl` lists it.
#[derive(Debug, Deserialize)]
struct Sink {
    index: u32,
    name: String,
    mute: bool,
}

/// A playback or recording stream as `pactl` lists it.
#[derive(Debug, Deserialize)]
struct Stream {
    /// index of the owning client, as text
    client: String,
    /// the sink a playback stream plays through
    sink: Option<u32>,
    corked: bool,
    #[serde(default)]
    properties: StreamProperties,
}

#[derive(Debug, Default, Deserialize)]
struct StreamProperties {
    #[serde(rename = "media.role")]
    role: Option<String>,
}

impl Stream {
    /// Running, and not an event sound such as Onda's own start cue.
    fn active(&self) -> bool {
        !self.corked && self.properties.role.as_deref() != Some("event")
    }
}

/// The default sink when muting it is right: an application is playing
/// through it and it is not muted already. Nothing is muted while an
/// application records and plays at once, such as a call, since muting
/// would cut it off.
fn sink_to_mute<'a>(
    default: &str,
    sinks: &'a [Sink],
    playing: &[Stream],
    recording: &[Stream],
) -> Option<&'a Sink> {
    let playing: Vec<&Stream> = playing.iter().filter(|stream| stream.active()).collect();
    let in_call = recording
        .iter()
        .filter(|stream| stream.active() && stream.client.parse::<u32>().is_ok())
        .any(|recorder| {
            playing
                .iter()
                .any(|player| player.client == recorder.client)
        });
    if in_call {
        tracing::info!("not muting: an application is recording and playing");
        return None;
    }
    let sink = sinks.iter().find(|sink| sink.name == default)?;
    // a sink the user muted stays theirs to unmute
    if sink.mute {
        return None;
    }
    playing
        .iter()
        .any(|stream| stream.sink == Some(sink.index))
        .then_some(sink)
}

fn pactl(args: &[&str]) -> anyhow::Result<Vec<u8>> {
    process::run_for("pactl", args, &[], PACTL_TIMEOUT)
}

fn list<T: DeserializeOwned>(kind: &str) -> anyhow::Result<Vec<T>> {
    serde_json::from_slice(&pactl(&["--format=json", "list", kind])?)
        .with_context(|| format!("unexpected pactl list of {kind}"))
}

fn set_mute(sink: &str, muted: bool) -> anyhow::Result<()> {
    pactl(&["set-sink-mute", sink, if muted { "1" } else { "0" }])?;
    Ok(())
}

/// File in the data folder that lists, one per line, the names of the sinks
/// Onda muted, so a run that crashed or was killed mid-session is undone at
/// the next launch.
struct MuteRecord(PathBuf);

impl MuteRecord {
    /// Unmutes the sinks an earlier run muted and never unmuted. A sink
    /// stays listed until it is seen unmuted, so one that is unplugged now
    /// is unmuted at a later launch.
    fn restore(&self) {
        let recorded = self.read();
        if recorded.is_empty() {
            return;
        }
        let sinks = match list::<Sink>("sinks") {
            Ok(sinks) => sinks,
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "could not check the audio outputs an earlier run left muted");
                return;
            }
        };
        let pending: Vec<String> = recorded
            .into_iter()
            .filter(|name| !restored(name, &sinks))
            .collect();
        if let Err(error) = self.write(&pending) {
            tracing::warn!(%error, "could not update the muted-output record");
        }
    }

    /// Mutes the default sink when `sink_to_mute` says so and returns its
    /// index for `unmute`.
    fn mute_default_sink(&self) -> anyhow::Result<Option<u32>> {
        let playing: Vec<Stream> = list("sink-inputs")?;
        if !playing.iter().any(Stream::active) {
            return Ok(None);
        }
        let recording: Vec<Stream> = list("source-outputs")?;
        let sinks: Vec<Sink> = list("sinks")?;
        let default = String::from_utf8(pactl(&["get-default-sink"])?)?;
        let Some(sink) = sink_to_mute(default.trim(), &sinks, &playing, &recording) else {
            return Ok(None);
        };
        // remembered first, so a crash right after the mute is undone too;
        // a mute that could not be undone later is not made at all
        let before = self.read();
        if !before.contains(&sink.name) {
            let mut names = before.clone();
            names.push(sink.name.clone());
            self.write(&names)
                .context("could not remember the muted output")?;
        }
        if let Err(error) = set_mute(&sink.name, true) {
            if let Err(error) = self.write(&before) {
                tracing::warn!(%error, "could not update the muted-output record");
            }
            return Err(error);
        }
        tracing::info!(sink = %sink.name, "muted the audio output: an application is playing");
        Ok(Some(sink.index))
    }

    fn unmute(&self, device: u32) -> anyhow::Result<()> {
        let sinks: Vec<Sink> = list("sinks")?;
        let recorded = self.read();
        // an index can name another sink after the audio server restarted
        let sink = sinks
            .iter()
            .find(|sink| sink.index == device && recorded.contains(&sink.name))
            .with_context(|| format!("sink {device} is not one Onda muted"))?;
        // the record stays when this fails, so the next launch tries again
        set_mute(&sink.name, false)?;
        let rest: Vec<String> = recorded
            .into_iter()
            .filter(|name| *name != sink.name)
            .collect();
        if let Err(error) = self.write(&rest) {
            tracing::warn!(%error, "could not update the muted-output record");
        }
        Ok(())
    }

    fn read(&self) -> Vec<String> {
        std::fs::read_to_string(&self.0)
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    /// writes `names`, or deletes the record when there are none
    fn write(&self, names: &[String]) -> std::io::Result<()> {
        if names.is_empty() {
            return match std::fs::remove_file(&self.0) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        // renamed into place, so a crash never leaves half a record
        let partial = self.0.with_extension("partial");
        std::fs::write(&partial, names.join("\n"))?;
        std::fs::rename(partial, &self.0)
    }
}

/// whether the sink `name` is known unmuted now
fn restored(name: &str, sinks: &[Sink]) -> bool {
    let Some(sink) = sinks.iter().find(|sink| sink.name == name) else {
        tracing::info!(sink = %name, "audio output left muted by an earlier run is not available");
        return false;
    };
    if !sink.mute {
        return true;
    }
    if let Err(error) = set_mute(name, false) {
        tracing::warn!(sink = %name, error = %format!("{error:#}"), "could not unmute an audio output left muted by an earlier run");
        return false;
    }
    tracing::info!(sink = %name, "unmuted an audio output that an earlier run left muted");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // trimmed from `pactl --format=json list` on PipeWire 1.6
    const SINKS: &str = r#"[
        {"index": 51, "state": "SUSPENDED", "name": "greendale-mic", "mute": false},
        {"index": 56, "state": "RUNNING", "name": "greendale-speakers", "mute": false}
    ]"#;
    const MUSIC: &str = r#"{"index": 67, "client": "66", "sink": 56, "corked": false,
        "properties": {"application.name": "pw-play", "media.role": "music"}}"#;
    const PAUSED_VIDEO: &str = r#"{"index": 68, "client": "70", "sink": 56, "corked": true,
        "properties": {"application.name": "Firefox"}}"#;
    const CUE: &str = r#"{"index": 77, "client": "76", "sink": 56, "corked": false,
        "properties": {"application.name": "canberra-gtk-play", "media.role": "event"}}"#;
    const ELSEWHERE: &str = r#"{"index": 65, "client": "64", "sink": 51, "corked": false,
        "properties": {"application.name": "paplay"}}"#;
    const CALL_AUDIO: &str = r#"{"index": 80, "client": "90", "sink": 56, "corked": false,
        "properties": {"application.name": "Hawthorne Meet"}}"#;
    const CALL_MICROPHONE: &str = r#"{"index": 81, "client": "90", "source": 3, "corked": false,
        "properties": {"application.name": "Hawthorne Meet"}}"#;
    const DICTATION: &str = r#"{"index": 82, "client": "n/a", "source": 3, "corked": false,
        "properties": {"application.name": "onda"}}"#;
    const UNOWNED: &str = r#"{"index": 83, "client": "n/a", "sink": 56, "corked": false,
        "properties": {}}"#;

    fn streams(entries: &[&str]) -> Vec<Stream> {
        serde_json::from_str(&format!("[{}]", entries.join(","))).unwrap()
    }

    #[test]
    fn mutes_the_default_sink_only_for_audio_an_application_plays_through_it() {
        struct Case {
            name: &'static str,
            default: &'static str,
            muted_already: bool,
            playing: &'static [&'static str],
            recording: &'static [&'static str],
            want: Option<u32>,
        }
        let cases = [
            Case {
                name: "music through the default sink",
                default: "greendale-speakers",
                muted_already: false,
                playing: &[MUSIC],
                recording: &[DICTATION],
                want: Some(56),
            },
            Case {
                name: "nothing plays",
                default: "greendale-speakers",
                muted_already: false,
                playing: &[],
                recording: &[DICTATION],
                want: None,
            },
            Case {
                name: "a paused stream is not audible",
                default: "greendale-speakers",
                muted_already: false,
                playing: &[PAUSED_VIDEO],
                recording: &[],
                want: None,
            },
            Case {
                name: "the start cue alone is not media",
                default: "greendale-speakers",
                muted_already: false,
                playing: &[CUE],
                recording: &[],
                want: None,
            },
            Case {
                name: "audio through another sink",
                default: "greendale-speakers",
                muted_already: false,
                playing: &[ELSEWHERE],
                recording: &[],
                want: None,
            },
            Case {
                name: "the user muted the sink",
                default: "greendale-speakers",
                muted_already: true,
                playing: &[MUSIC],
                recording: &[],
                want: None,
            },
            Case {
                name: "a call is never cut off",
                default: "greendale-speakers",
                muted_already: false,
                playing: &[MUSIC, CALL_AUDIO],
                recording: &[CALL_MICROPHONE],
                want: None,
            },
            Case {
                name: "streams without a client are not one application",
                default: "greendale-speakers",
                muted_already: false,
                playing: &[UNOWNED],
                recording: &[DICTATION],
                want: Some(56),
            },
            Case {
                name: "no default sink",
                default: "",
                muted_already: false,
                playing: &[MUSIC],
                recording: &[],
                want: None,
            },
        ];
        for tc in cases {
            let mut sinks: Vec<Sink> = serde_json::from_str(SINKS).unwrap();
            sinks[1].mute = tc.muted_already;
            let got = sink_to_mute(
                tc.default,
                &sinks,
                &streams(tc.playing),
                &streams(tc.recording),
            );
            assert_eq!(got.map(|sink| sink.index), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn a_player_that_did_not_answer_leaves_playing_unknown() {
        use Playback::{Paused, Playing, Stopped, Unknown};
        let track = |is_playing| NowPlaying::Track {
            is_playing: Some(is_playing),
            playback_rate: None,
        };
        let unknown = NowPlaying::Failed {
            reason: "org.mpris.MediaPlayer2.player0 did not report its state".to_owned(),
        };
        let cases: [(&str, &[Playback], NowPlaying); 6] = [
            ("no players", &[], NowPlaying::NoTrack),
            ("one playing", &[Paused, Playing], track(true)),
            ("none playing", &[Paused, Stopped], track(false)),
            ("a silent player alone", &[Unknown], unknown.clone()),
            (
                "a silent player beside a paused one",
                &[Unknown, Paused],
                unknown,
            ),
            (
                "a playing player settles it",
                &[Unknown, Playing],
                track(true),
            ),
        ];
        for (name, states, want) in cases {
            let players: Vec<Player> = states
                .iter()
                .enumerate()
                .map(|(index, playback)| Player {
                    name: format!("org.mpris.MediaPlayer2.player{index}"),
                    playback: *playback,
                })
                .collect();
            assert_eq!(report(&players), want, "{name}");
        }
    }

    #[test]
    fn the_mute_record_lists_sinks_until_none_is_left() {
        let folder = tempfile::tempdir().unwrap();
        let record = MuteRecord(folder.path().join("muted-output"));
        assert!(record.read().is_empty());
        let names = ["greendale-speakers".to_owned(), "troy-headset".to_owned()];
        record.write(&names).unwrap();
        assert_eq!(record.read(), names);
        record.write(&[]).unwrap();
        assert!(!record.0.exists(), "an empty record is deleted");
        record.write(&[]).unwrap();
    }
}
