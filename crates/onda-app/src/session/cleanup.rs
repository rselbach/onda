//! The Post-Processing page inside the session actor: the API key field's
//! load and save rules (CLN-24, CLN-25) and the sample test (CLN-27).
//!
//! Every credential-store call goes through one [`KeyQueue`], so a load
//! issued after a save always reads what that save wrote.

use onda_domain::cleanup::{
    ApiKey, CleanupModel, KeyBaseline, KeyWrite, TEST_SAMPLE, build_request, key_write,
    test_result_line, test_said_line,
};
use tokio::sync::{mpsc, oneshot};
use tokio::task::spawn_blocking;

use super::{Actor, Msg, Task, UiMsg, send_when};
use crate::events::Event;
use crate::seams::SettingChange;
use crate::views::{
    ApiKeyFieldView, CleanupTestResult, MainWindowSection, PostProcessingInputs,
    PostProcessingPage, post_processing_page,
};

/// The API key field's placeholder (CLN-26).
const KEY_PLACEHOLDER: &str = "OpenRouter API key";

// a credential-store call waiting its turn
type KeyJob = Box<dyn FnOnce() + Send>;

/// Runs credential-store calls one at a time, in the order they were issued
/// (CLN-25).
#[derive(Clone)]
struct KeyQueue(mpsc::UnboundedSender<KeyJob>);

impl KeyQueue {
    // must be called inside the runtime
    fn start() -> Self {
        let (jobs, mut queue) = mpsc::unbounded_channel::<KeyJob>();
        tokio::spawn(async move {
            while let Some(job) = queue.recv().await {
                if let Err(error) = spawn_blocking(job).await {
                    tracing::error!(%error, "a credential-store call failed");
                }
            }
        });
        Self(jobs)
    }

    /// Queues `work` behind every call issued before it.
    fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> oneshot::Receiver<T> {
        let (done, result) = oneshot::channel();
        let job: KeyJob = Box::new(move || {
            let _ = done.send(work());
        });
        // the worker lives as long as the runtime; without it, run unordered
        if let Err(mpsc::error::SendError(job)) = self.0.send(job) {
            tracing::warn!("the credential-store queue is gone");
            spawn_blocking(job);
        }
        result
    }
}

// a queued call that never reported back
fn lost<T>(_: oneshot::error::RecvError) -> Result<T, String> {
    Err("The credential-store call did not finish.".to_owned())
}

/// Post-Processing page state.
pub(super) struct CleanupPageState {
    key_queue: Option<KeyQueue>,
    baseline: KeyBaseline,
    text: String,
    revision: u64,
    loading: bool,
    error: Option<String>,
    load_generation: u64,
    test: Option<Task>,
    test_generation: u64,
    test_result: Option<CleanupTestResult>,
    pub(super) test_error: Option<String>,
}

impl Default for CleanupPageState {
    fn default() -> Self {
        Self {
            key_queue: None,
            // nothing loaded yet, so nothing may be saved
            baseline: KeyBaseline::Loading,
            text: String::new(),
            revision: 0,
            loading: false,
            error: None,
            load_generation: 0,
            test: None,
            test_generation: 0,
            test_result: None,
            test_error: None,
        }
    }
}

// the value a write leaves stored
fn written_value(write: KeyWrite) -> String {
    match write {
        KeyWrite::Save(key) => key,
        KeyWrite::Delete => String::new(),
    }
}

impl Actor {
    fn key_queue(&mut self) -> KeyQueue {
        self.ui
            .cleanup
            .key_queue
            .get_or_insert_with(KeyQueue::start)
            .clone()
    }

    /// The page appeared (CLN-25). The key field exists only while cleanup
    /// is on (CLN-26), so only then is the keychain read, which can prompt
    /// and pull Onda forward.
    pub(super) fn post_processing_appeared(&mut self) {
        if self.settings.cleanup_enabled {
            self.load_api_key();
        }
    }

    /// The "Post-process transcripts" toggle (CLN-26). Turning it on shows
    /// the key field, which loads; turning it off hides the field, which
    /// abandons a load in flight (the shell commits the field as it goes).
    pub(super) fn set_cleanup_enabled(&mut self, on: bool) {
        self.apply_setting(SettingChange::CleanupEnabled(on));
        match on {
            true if self.ui.is_visible(MainWindowSection::PostProcessing) => self.load_api_key(),
            true => {}
            false => self.cancel_key_load(),
        }
    }

    fn cancel_key_load(&mut self) {
        let page = &mut self.ui.cleanup;
        if page.loading {
            page.loading = false;
            page.load_generation += 1;
        }
    }

    // CLN-25: load asynchronously, after every save already issued
    fn load_api_key(&mut self) {
        let secrets = self.ports.secrets.clone();
        let loaded = self.key_queue().run(move || secrets.openrouter_key());
        let page = &mut self.ui.cleanup;
        page.load_generation += 1;
        page.baseline = KeyBaseline::Loading;
        page.loading = true;
        page.error = None;
        let generation = page.load_generation;
        tokio::spawn(send_when(self.messages.clone(), async move {
            let result = loaded.await.unwrap_or_else(lost);
            Msg::Ui(UiMsg::KeyLoaded { generation, result })
        }));
    }

    pub(super) fn key_loaded(&mut self, generation: u64, result: Result<Option<String>, String>) {
        let page = &mut self.ui.cleanup;
        // typing during the load cancelled it (CLN-25)
        if generation != page.load_generation || !page.loading {
            return;
        }
        page.loading = false;
        match result {
            Ok(key) => {
                let key = key.unwrap_or_default();
                page.baseline = KeyBaseline::Known(key.clone());
                page.text = key;
                page.revision += 1;
            }
            Err(error) => {
                tracing::error!(%error, "could not read the OpenRouter key");
                // a blank field must not delete a key we could not read
                page.baseline = KeyBaseline::Unknown;
                page.error = Some(error);
                // never show, or save again, a key from an earlier load
                page.text.clear();
                page.revision += 1;
            }
        }
        // a Keychain prompt can push Onda behind other apps (CLN-25)
        self.events.emit(Event::ActivateApp);
    }

    pub(super) fn api_key_edited(&mut self, text: String) {
        self.cancel_key_load();
        let page = &mut self.ui.cleanup;
        page.baseline =
            std::mem::replace(&mut page.baseline, KeyBaseline::Unknown).after_user_edit();
        page.text = text;
    }

    // Return, end of editing, page disappear, window close (CLN-25)
    pub(super) fn save_api_key(&mut self) {
        let page = &self.ui.cleanup;
        let Some(write) = key_write(&page.baseline, &page.text) else {
            return;
        };
        let value = written_value(write);
        let secrets = self.ports.secrets.clone();
        let saving = value.clone();
        let saved = self
            .key_queue()
            .run(move || secrets.save_openrouter_key(&saving));
        tokio::spawn(send_when(self.messages.clone(), async move {
            let result = saved.await.unwrap_or_else(lost);
            Msg::Ui(UiMsg::KeySaved { value, result })
        }));
    }

    pub(super) fn key_saved(&mut self, value: String, result: Result<(), String>) {
        let page = &mut self.ui.cleanup;
        match result {
            Ok(()) => {
                // show the trimmed value saved, unless typing moved on (CLN-24)
                if page.text != value && page.text.trim() == value {
                    page.text.clone_from(&value);
                    page.revision += 1;
                }
                page.baseline = KeyBaseline::Known(value);
                page.error = None;
            }
            Err(error) => {
                tracing::error!(%error, "could not save the OpenRouter key");
                page.error = Some(error);
            }
        }
    }

    /// The save quitting must finish first (CLN-25), if one is due. It is
    /// queued now, behind every call already issued; the returned closure
    /// blocks until it is done.
    pub(super) fn pending_key_save(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
        let page = &self.ui.cleanup;
        let value = written_value(key_write(&page.baseline, &page.text)?);
        let secrets = self.ports.secrets.clone();
        let saved = self
            .key_queue()
            .run(move || secrets.save_openrouter_key(&value));
        Some(Box::new(move || {
            if let Err(error) = saved.blocking_recv().unwrap_or_else(lost) {
                tracing::error!(%error, "could not save the OpenRouter key at quit");
            }
        }))
    }

    pub(super) fn set_cleanup_model(&mut self, model: &'static CleanupModel) {
        self.apply_setting(SettingChange::CleanupModel(model));
        // the result resets when the model changes (CLN-27)
        self.ui.cleanup.test_result = None;
    }

    // CLN-27: save a just-typed key, then send the sample through the same
    // request path with no guardrails and no app name
    pub(super) fn test_cleanup(&mut self) {
        if self.ui.cleanup.test.is_some() {
            return;
        }
        let page = &mut self.ui.cleanup;
        page.test_result = None;
        page.test_error = None;
        page.test_generation += 1;
        let generation = page.test_generation;
        let write = key_write(&page.baseline, &page.text).map(written_value);
        let secrets = self.ports.secrets.clone();
        let transport = self.ports.cleanup.clone();
        let model = self.settings.cleanup_model;
        let messages = self.messages.clone();
        let queue = self.key_queue();
        let saved = write.map(|value| {
            let saver = secrets.clone();
            let saving = value.clone();
            (value, queue.run(move || saver.save_openrouter_key(&saving)))
        });
        let stored = queue.run(move || secrets.openrouter_key());
        let task = self.spawn(async move {
            if let Some((value, saved)) = saved {
                let result = saved.await.unwrap_or_else(lost);
                let _ = messages.send(Msg::Ui(UiMsg::KeySaved { value, result }));
            }
            let stored = stored.await.unwrap_or_else(lost);
            let result = match stored {
                Ok(stored) => match ApiKey::new(stored.as_deref().unwrap_or_default()) {
                    Ok(key) => transport
                        .send(build_request(&key, model, TEST_SAMPLE, None))
                        .await
                        .map_err(|error| error.to_string()),
                    Err(error) => Err(error.to_string()),
                },
                Err(error) => Err(error),
            };
            Msg::Ui(UiMsg::TestFinished { generation, result })
        });
        self.ui.cleanup.test = Some(task);
    }

    pub(super) fn test_finished(&mut self, generation: u64, result: Result<String, String>) {
        let page = &mut self.ui.cleanup;
        if generation != page.test_generation {
            return;
        }
        page.test = None;
        match result {
            Ok(output) => {
                page.test_result = Some(CleanupTestResult {
                    said: test_said_line(),
                    result: test_result_line(&output),
                });
            }
            Err(error) => page.test_error = Some(error),
        }
    }

    // leaving the page cancels a running test silently (CLN-27)
    pub(super) fn cancel_cleanup_test(&mut self) {
        let page = &mut self.ui.cleanup;
        if page.test.take().is_some() {
            page.test_generation += 1;
        }
    }

    pub(super) fn post_processing_view(&self) -> PostProcessingPage {
        let page = &self.ui.cleanup;
        let key = ApiKeyFieldView {
            placeholder: KEY_PLACEHOLDER.to_owned(),
            text: page.text.clone(),
            revision: page.revision,
            loading: page.loading,
            error: page.error.clone(),
        };
        post_processing_page(PostProcessingInputs {
            enabled: self.settings.cleanup_enabled,
            model: self.settings.cleanup_model,
            key: &key,
            test_running: page.test.is_some(),
            test_result: page.test_result.as_ref(),
            test_error: page.test_error.as_deref(),
        })
    }
}
