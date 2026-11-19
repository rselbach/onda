//! Settings through `onda-settings`.

use std::sync::Arc;

use onda_app::seams::{SettingChange, Settings as SessionSettings, SettingsSource};
use onda_domain::cleanup::CleanupRequest;
use onda_domain::hotkey::Shortcut;
use onda_domain::language_menu::recent_languages;
use onda_domain::languages::{LanguageSetting, default_language};
use onda_domain::messages::UserMessage;
use onda_domain::migration::{REMOVED_MODEL_IDS, SelectionResolution, resolve_selection};
use onda_domain::models::{Catalog, ModelId};
use onda_domain::onboarding::OnboardingFlags;
use onda_settings::Settings;
use tokio::sync::oneshot;

/// [`SettingsSource`] over the settings file. Computed defaults follow the
/// OS language and the catalog (SET-2a); the hotkey's follows the platform
/// (HOT-2).
pub struct SettingsAdapter {
    settings: Arc<Settings>,
    catalog: Catalog,
    os_language: Option<String>,
    default_hotkey: &'static Shortcut,
}

impl SettingsAdapter {
    /// Reads `settings`; `os_language` is the OS locale's language code
    /// (LANG-2), `catalog` reflects whether the shell has a system engine,
    /// and `default_hotkey` is the platform's default shortcut.
    pub fn new(
        settings: Arc<Settings>,
        catalog: Catalog,
        os_language: Option<String>,
        default_hotkey: &'static Shortcut,
    ) -> Self {
        Self {
            settings,
            catalog,
            os_language,
            default_hotkey,
        }
    }
}

impl SettingsSource for SettingsAdapter {
    fn snapshot(&self) -> SessionSettings {
        let os_language = self.os_language.as_deref();
        self.settings.read(|settings| {
            let language = settings.selected_language(os_language);
            let stored_model = settings.selected_model(&self.catalog, os_language);
            let cleanup_provider = settings.cleanup_provider();
            let selected_model = resolved_model(
                &self.catalog,
                &stored_model,
                REMOVED_MODEL_IDS,
                default_language(os_language),
            );
            let stored_recent_languages = settings.recent_languages();
            SessionSettings {
                activation_mode: settings.activation_mode(),
                output_mode: settings.output_mode(),
                language,
                recent_languages: recent_languages(language, &stored_recent_languages),
                selected_model,
                cleanup_enabled: settings.post_processing_enabled(),
                cleanup_provider,
                cleanup_model: settings.cleanup_model(cleanup_provider.unwrap_or_default()),
                cleanup_prompt: settings.cleanup_prompt(),
                replacement_rules: settings.replacement_rules(),
                history_enabled: settings.history_enabled(),
                keep_audio: settings.history_keeps_audio(),
                pause_media: settings.pause_media_during_recording(),
                sounds: settings.sound_feedback_enabled(),
                hotkey: settings.hotkey(self.default_hotkey),
                first_run_pending: settings.first_run_pending(),
                onboarding: OnboardingFlags {
                    onboarding_completed: settings.onboarding_completed(),
                    did_show_first_run_setup: settings.did_show_first_run_setup(),
                },
                input_device_uid: settings.input_device_uid(),
                history_retention: settings.history_retention(),
                stored_model,
                stored_recent_languages,
                last_notice_key: settings.last_model_migration_notice_key(),
                declined_notice_key: settings.declined_model_migration_notice_key(),
                os_language: self.os_language.clone(),
            }
        })
    }

    fn admit_cleanup(&self, request: CleanupRequest) -> oneshot::Receiver<CleanupRequest> {
        let (handoff, admitted) = oneshot::channel();
        let settings = self.settings.clone();
        tokio::task::spawn_blocking(move || {
            let _ = settings.with_cleanup_admission(request.provider(), || {
                let _ = handoff.send(request);
            });
        });
        admitted
    }

    fn set_selected_model(&self, model: ModelId) {
        if let Err(error) = self.settings.set_selected_model(model) {
            tracing::error!(%error, model = model.as_str(), "could not save the selected model");
        }
    }

    fn set_hotkey(&self, shortcut: &Shortcut) -> Result<(), String> {
        self.settings
            .set_hotkey(shortcut)
            .map_err(|error| error.to_string())
    }

    fn apply(&self, change: SettingChange) -> Result<(), String> {
        let settings = &self.settings;
        let result = match change {
            SettingChange::SelectedModel(model) => settings.set_selected_model(model),
            SettingChange::Language(language) => settings.set_selected_language(language),
            SettingChange::RecentLanguages(languages) => settings.set_recent_languages(&languages),
            SettingChange::InputDevice(uid) => settings.set_input_device_uid(&uid),
            SettingChange::OutputMode(mode) => settings.set_output_mode(mode),
            SettingChange::ActivationMode(mode) => settings.set_activation_mode(mode),
            SettingChange::PauseMedia(on) => settings.set_pause_media_during_recording(on),
            SettingChange::Sounds(on) => settings.set_sound_feedback_enabled(on),
            SettingChange::HistoryEnabled(on) => settings.set_history_enabled(on),
            SettingChange::HistoryKeepsAudio(on) => settings.set_history_keeps_audio(on),
            SettingChange::HistoryRetention(retention) => settings.set_history_retention(retention),
            SettingChange::ReplacementRules(rules) => settings.set_replacement_rules(&rules),
            SettingChange::CleanupEnabled(on) => settings.set_post_processing_enabled(on),
            SettingChange::CleanupModel(provider, model) => {
                settings.select_cleanup_model(provider, &model)
            }
            SettingChange::CleanupPrompt(prompt) => settings.set_cleanup_prompt(&prompt),
            SettingChange::Onboarding(flags) => settings
                .set_onboarding_flags(flags.onboarding_completed, flags.did_show_first_run_setup),
            SettingChange::LastNoticeKey(key) => settings.set_last_model_migration_notice_key(&key),
            SettingChange::DeclinedNoticeKey(key) => {
                settings.set_declined_model_migration_notice_key(&key)
            }
        };
        result.map_err(|error| error.to_string())
    }

    fn reset_invalid_language(&self) -> Option<UserMessage> {
        match self.settings.reset_invalid_language() {
            Ok(warning) => warning,
            Err(error) => {
                tracing::error!(%error, "could not reset the language setting");
                None
            }
        }
    }
}

// ASR-4, DL-12: an unknown or removed id resolves to its mapped
// replacement in `removed`, else the recommended model, through the same
// resolution the launch flow switches to or offers
fn resolved_model(
    catalog: &Catalog,
    stored: &str,
    removed: &[(&str, ModelId)],
    default_language: LanguageSetting,
) -> ModelId {
    // installed models only decide whether to ask first, not the model
    match resolve_selection(catalog, stored, removed, default_language, &[]) {
        SelectionResolution::Current(model) => model,
        SelectionResolution::SwitchSilently(migration)
        | SelectionResolution::NeedsDownload(migration) => migration.to,
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::time::Duration;

    use super::*;
    use onda_domain::cleanup::{CleanupPrompt, CleanupProvider, build_request};
    use onda_domain::routing::default_model;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::time::timeout;

    fn cleanup_settings() -> (tempfile::TempDir, SettingsAdapter) {
        let dir = tempfile::tempdir().expect("settings directory");
        let settings = Arc::new(Settings::open(dir.path().join("settings.json")));
        settings
            .set_post_processing_enabled(true)
            .expect("enable cleanup");
        let adapter = SettingsAdapter::new(
            settings,
            Catalog::new(None),
            None,
            &Shortcut::CONTROL_ALT_SPACE,
        );
        (dir, adapter)
    }

    fn writer_lock(dir: &tempfile::TempDir) -> std::fs::File {
        OpenOptions::new()
            .write(true)
            .open(dir.path().join("settings.json.lock"))
            .expect("open writer lock")
    }

    fn cleanup_request() -> CleanupRequest {
        let provider = CleanupProvider::OpenRouter;
        build_request(
            provider,
            None,
            provider.default_model(),
            &CleanupPrompt::default(),
            "cool cool cool",
            None,
        )
    }

    #[tokio::test]
    async fn busy_writer_denies_cleanup_without_waiting_for_unlock() {
        let (dir, adapter) = cleanup_settings();
        let held = writer_lock(&dir);
        held.lock().expect("lock for writing");
        let result = timeout(
            Duration::from_secs(1),
            adapter.admit_cleanup(cleanup_request()),
        )
        .await
        .expect("admission returned while locked");
        assert!(result.is_err());
        drop(held);
    }

    #[tokio::test]
    async fn writer_before_cleanup_admission_suppresses_handoff() {
        let (dir, adapter) = cleanup_settings();
        let writer = Settings::open(dir.path().join("settings.json"));
        writer
            .set_post_processing_enabled(false)
            .expect("disable cleanup");
        assert!(adapter.admit_cleanup(cleanup_request()).await.is_err());
    }

    #[tokio::test]
    async fn writer_after_cleanup_admission_completes_before_response() {
        const POST: &[u8] =
            b"POST /cleanup HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n";
        const REPLY: &str =
            "HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\nCool cool cool.";

        let (dir, adapter) = cleanup_settings();
        let path = dir.path().join("settings.json");
        let writer = Settings::open(&path);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let request = cleanup_request();
        let admission = adapter.admit_cleanup(request.clone());
        let executor = tokio::spawn(async move {
            assert_eq!(admission.await.expect("request admitted"), request);
            let mut stream = TcpStream::connect(address).await.expect("connect");
            stream.write_all(POST).await.expect("send request");
            let mut response = String::new();
            stream
                .read_to_string(&mut response)
                .await
                .expect("read response");
            response
        });
        let (mut connection, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .expect("transport connected")
            .expect("accept");
        let mut received = [0; POST.len()];
        timeout(Duration::from_secs(5), connection.read_exact(&mut received))
            .await
            .expect("request arrived")
            .expect("read request");
        assert_eq!(received, POST);
        let write = tokio::task::spawn_blocking(move || writer.set_post_processing_enabled(false));
        timeout(Duration::from_secs(5), write)
            .await
            .expect("writer completed before the response")
            .expect("writer task")
            .expect("disable cleanup");
        assert!(!executor.is_finished());
        assert!(!Settings::open(path).post_processing_enabled());
        connection
            .write_all(REPLY.as_bytes())
            .await
            .expect("send response");
        drop(connection);
        let response = timeout(Duration::from_secs(5), executor)
            .await
            .expect("transport received response")
            .expect("cleanup task");
        assert_eq!(response, REPLY);
    }

    #[test]
    fn cancelling_before_handoff_never_starts_transport() {
        let (_dir, adapter) = cleanup_settings();
        tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .build()
            .expect("runtime")
            .block_on(async {
                let (occupy, occupied) = oneshot::channel();
                let (release, released) = std::sync::mpsc::channel();
                let blocker = tokio::task::spawn_blocking(move || {
                    occupy.send(()).expect("blocking worker occupied");
                    let _ = released.recv();
                });
                occupied.await.expect("blocking worker occupied");
                let (wait, waiting) = oneshot::channel();
                let (start, started) = oneshot::channel();
                let executor = tokio::spawn(async move {
                    let admission = adapter.admit_cleanup(cleanup_request());
                    wait.send(()).expect("admission queued");
                    if let Ok(request) = admission.await {
                        start.send(request).expect("transport started");
                    }
                });
                waiting.await.expect("admission queued");
                executor.abort();
                assert!(executor.await.expect_err("caller aborted").is_cancelled());
                release.send(()).expect("release blocking worker");
                blocker.await.expect("blocking task");
                assert!(started.await.is_err());
            });
    }

    const TEST_REMOVED: &[(&str, ModelId)] = &[
        ("parakeet-tdt-v3", ModelId::ParakeetUltra),
        ("chang-speech", ModelId::SystemSpeech),
    ];

    #[test]
    fn snapshot_model_follows_the_removed_id_map_dl_12() {
        let catalog = Catalog::new(None);
        let english = default_language(Some("en"));
        struct Case {
            name: &'static str,
            stored: &'static str,
            want: ModelId,
        }
        let cases = [
            Case {
                name: "a current id is kept",
                stored: "parakeet-unified",
                want: ModelId::ParakeetUnified,
            },
            Case {
                name: "a removed id maps to its replacement",
                stored: "parakeet-tdt-v3",
                want: ModelId::ParakeetUltra,
            },
            Case {
                name: "a replacement missing from this catalog falls back",
                stored: "chang-speech",
                want: default_model(&catalog, english),
            },
            Case {
                name: "an unknown id falls back (ASR-4)",
                stored: "greendale-asr",
                want: default_model(&catalog, english),
            },
        ];
        for tc in cases {
            assert_eq!(
                resolved_model(&catalog, tc.stored, TEST_REMOVED, english),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn snapshot_reads_consistent_fields() {
        let (_dir, adapter) = cleanup_settings();
        adapter
            .settings
            .select_cleanup_model(
                CleanupProvider::DeepSeek,
                CleanupProvider::DeepSeek.default_model(),
            )
            .expect("select cleanup model");
        adapter
            .settings
            .set_onboarding_flags(true, true)
            .expect("set onboarding");
        let snapshot = adapter.snapshot();
        assert!(snapshot.cleanup_enabled);
        assert_eq!(snapshot.cleanup_provider, Some(CleanupProvider::DeepSeek));
        assert_eq!(
            snapshot.cleanup_model,
            CleanupProvider::DeepSeek.default_model().clone()
        );
        assert!(!snapshot.first_run_pending);
    }
}
