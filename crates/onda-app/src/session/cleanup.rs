//! The Post-Processing page inside the session actor: the model sheet
//! (each provider's model list, the model choice, and the API key field's
//! load and save rules, CLN-24, CLN-25), the prompt sheet's draft, and the
//! sample test (CLN-27).
//!
//! Every credential-store call goes through one [`KeyQueue`], so a load
//! issued after a save always reads what that save wrote.

use std::collections::HashMap;
use std::sync::Arc;

use onda_domain::cleanup::{
    ApiKey, CleanupError, CleanupModel, CleanupOutcome, CleanupPrompt, CleanupProvider,
    KeyBaseline, KeyWrite, ModelListing, OnDeviceStatus, SYSTEM_PROMPT, TEST_SAMPLE, build_request,
    custom_model_slug, key_write, test_result_line, test_said_line,
};
use tokio::sync::{mpsc, oneshot};
use tokio::task::spawn_blocking;

use super::{Actor, Msg, Task, UiMsg, send_when};
use crate::events::Event;
use crate::pipeline::refine;
use crate::seams::{CleanupTransport, SettingChange};
use crate::views::{
    ApiKeyFieldView, CleanupTestResult, MainWindowSection, ModelsNotice, PostProcessingInputs,
    PostProcessingPage, PromptDraftInputs, ProvidersDialogInputs, models_notice,
    post_processing_page, prompt_saved_text,
};

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

// the prompt sheet's unsaved text; the setting changes only on "Save"
struct PromptDraft {
    text: String,
    save_error: Option<String>,
}

/// Post-Processing page state.
pub(super) struct CleanupPageState {
    key_queue: Option<KeyQueue>,
    // whose key the field holds; every save goes to that provider only
    key_provider: Option<CleanupProvider>,
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
    // the model sheet, open on this provider
    dialog: Option<CleanupProvider>,
    // each provider's model list, fetched once per run
    listed: HashMap<CleanupProvider, Vec<CleanupModel>>,
    listing: Option<(CleanupProvider, Task)>,
    list_generation: u64,
    list_notice: Option<(CleanupProvider, ModelsNotice)>,
    // Apple Intelligence, as last read; refreshed when the page or the
    // sheet shows
    on_device: OnDeviceStatus,
    // the prompt sheet, while open
    prompt_draft: Option<PromptDraft>,
    // bumped whenever the core puts text in the prompt sheet's editor
    prompt_revision: u64,
    // what the last prompt save did, until the sheet opens again or the
    // page is left
    pub(super) prompt_status: Option<&'static str>,
}

impl Default for CleanupPageState {
    fn default() -> Self {
        Self {
            key_queue: None,
            key_provider: None,
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
            dialog: None,
            listed: HashMap::new(),
            listing: None,
            list_generation: 0,
            list_notice: None,
            on_device: OnDeviceStatus::DeviceNotEligible,
            prompt_draft: None,
            prompt_revision: 0,
            prompt_status: None,
        }
    }
}

// what the test sends with: the stored key, none for a provider without
// one, or why it cannot run
async fn test_key(
    provider: CleanupProvider,
    transport: &Arc<dyn CleanupTransport>,
    stored: Option<oneshot::Receiver<Result<Option<String>, String>>>,
) -> Result<Option<ApiKey>, String> {
    if provider.on_device() {
        let transport = transport.clone();
        let status = spawn_blocking(move || transport.on_device_status())
            .await
            .unwrap_or(OnDeviceStatus::DeviceNotEligible);
        if let Some(message) = status.message() {
            return Err(message.to_owned());
        }
    }
    let Some(stored) = stored else {
        return Ok(None);
    };
    let stored = stored.await.unwrap_or_else(lost)?;
    ApiKey::new(stored.as_deref().unwrap_or_default())
        .map(Some)
        .map_err(|error| error.describe(provider))
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

    /// The page appeared: list the provider's models for their names,
    /// unless listing needs the key, which the page never reads (CLN-25).
    pub(super) fn post_processing_appeared(&mut self) {
        self.refresh_on_device();
        if self.settings.cleanup_enabled {
            self.list_public_models_once();
        }
    }

    fn list_public_models_once(&mut self) {
        let provider = self.settings.cleanup_provider;
        if provider.model_listing() == ModelListing::Public {
            self.list_models_once(provider);
        }
    }

    // a cheap OS read; the user can turn Apple Intelligence on or off at any
    // time
    fn refresh_on_device(&mut self) {
        self.ui.cleanup.on_device = self.ports.cleanup.on_device_status();
    }

    // the sheet's key field exists only for providers that use a key
    fn show_provider(&mut self, provider: CleanupProvider) {
        self.ui.cleanup.dialog = Some(provider);
        if provider.uses_key() {
            self.load_api_key(provider);
        }
        self.list_models_once(provider);
    }

    /// The "Post-process transcripts" toggle (CLN-26). Turning it off hides
    /// the details, which closes the model sheet and discards a prompt
    /// draft; the saved prompt stays.
    pub(super) fn set_cleanup_enabled(&mut self, on: bool) {
        self.apply_setting(SettingChange::CleanupEnabled(on));
        match on {
            true if self.ui.is_visible(MainWindowSection::PostProcessing) => {
                self.list_public_models_once();
            }
            true => {}
            false => {
                self.close_providers_dialog();
                self.cancel_prompt_editor();
            }
        }
    }

    /// The "Prompt" row: opens the prompt sheet on the prompt in use.
    pub(super) fn open_prompt_editor(&mut self) {
        let visible = self.ui.is_visible(MainWindowSection::PostProcessing);
        let page = &mut self.ui.cleanup;
        let sheet_open = page.dialog.is_some() || page.prompt_draft.is_some();
        if !self.settings.cleanup_enabled || !visible || sheet_open {
            return;
        }
        page.prompt_draft = Some(PromptDraft {
            text: self.settings.cleanup_prompt.text().to_owned(),
            save_error: None,
        });
        page.prompt_revision += 1;
        page.prompt_status = None;
    }

    /// Typing in the prompt sheet. Only the draft changes.
    pub(super) fn prompt_edited(&mut self, text: String) {
        if let Some(draft) = &mut self.ui.cleanup.prompt_draft {
            draft.text = text;
            draft.save_error = None;
        }
    }

    /// "Restore Default": Onda's prompt becomes the draft, still unsaved.
    pub(super) fn restore_default_prompt(&mut self) {
        let page = &mut self.ui.cleanup;
        if let Some(draft) = &mut page.prompt_draft {
            SYSTEM_PROMPT.clone_into(&mut draft.text);
            draft.save_error = None;
            page.prompt_revision += 1;
        }
    }

    /// "Cancel", Escape, or anything that hides the prompt sheet: the draft
    /// is discarded, a restored default included.
    pub(super) fn cancel_prompt_editor(&mut self) {
        self.ui.cleanup.prompt_draft = None;
    }

    /// "Save": the draft becomes the prompt for every provider and model,
    /// and the sheet closes. A blank draft, or one equal to the prompt in
    /// use, saves nothing and stays open; so does one that could not be
    /// stored, which says why.
    pub(super) fn save_prompt(&mut self) {
        let Some(draft) = &self.ui.cleanup.prompt_draft else {
            return;
        };
        let Ok(prompt) = CleanupPrompt::from_text(&draft.text) else {
            return;
        };
        if prompt == self.settings.cleanup_prompt {
            return;
        }
        let saved = self
            .ports
            .settings
            .apply(SettingChange::CleanupPrompt(prompt.clone()));
        self.refresh_settings();
        match saved {
            Ok(()) => {
                let page = &mut self.ui.cleanup;
                page.prompt_draft = None;
                page.prompt_status = Some(prompt_saved_text(&prompt));
                // a result, shown or on its way, is the old prompt's (CLN-27)
                page.test_result = None;
                self.cancel_cleanup_test();
            }
            Err(error) => {
                tracing::error!(%error, "could not save the cleanup prompt");
                if let Some(draft) = &mut self.ui.cleanup.prompt_draft {
                    draft.save_error = Some(format!("Couldn't save the prompt: {error}"));
                }
            }
        }
    }

    /// The "Model" row: opens the sheet on the provider in use. The
    /// key field exists only while the sheet is open (CLN-25), so only then
    /// is the keychain read, which can prompt and pull Onda forward.
    pub(super) fn open_providers_dialog(&mut self) {
        let visible = self.ui.is_visible(MainWindowSection::PostProcessing);
        let page = &self.ui.cleanup;
        let sheet_open = page.dialog.is_some() || page.prompt_draft.is_some();
        if !self.settings.cleanup_enabled || !visible || sheet_open {
            return;
        }
        self.refresh_on_device();
        self.show_provider(self.settings.cleanup_provider);
    }

    /// "Done", or anything that hides the sheet: saves the key field
    /// (CLN-25) and abandons a load in flight.
    pub(super) fn close_providers_dialog(&mut self) {
        if self.ui.cleanup.dialog.take().is_some() {
            self.save_api_key();
            self.cancel_key_load();
        }
    }

    /// A provider in the sheet's sidebar.
    pub(super) fn select_dialog_provider(&mut self, provider: CleanupProvider) {
        if self.ui.cleanup.dialog.is_none_or(|open| open == provider) {
            return;
        }
        // the typed key belongs to the provider it was typed for
        self.save_api_key();
        self.cancel_key_load();
        self.refresh_on_device();
        self.show_provider(provider);
    }

    fn cancel_key_load(&mut self) {
        let page = &mut self.ui.cleanup;
        if page.loading {
            page.loading = false;
            page.load_generation += 1;
        }
    }

    // CLN-25: load asynchronously, after every save already issued
    fn load_api_key(&mut self, provider: CleanupProvider) {
        let secrets = self.ports.secrets.clone();
        let loaded = self.key_queue().run(move || secrets.api_key(provider));
        let page = &mut self.ui.cleanup;
        page.key_provider = Some(provider);
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
                tracing::error!(%error, "could not read the cleanup API key");
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
        let Some(provider) = page.key_provider else {
            return;
        };
        let Some(write) = key_write(&page.baseline, &page.text) else {
            return;
        };
        let value = written_value(write);
        let secrets = self.ports.secrets.clone();
        let saving = value.clone();
        let saved = self
            .key_queue()
            .run(move || secrets.save_api_key(provider, &saving));
        tokio::spawn(send_when(self.messages.clone(), async move {
            let result = saved.await.unwrap_or_else(lost);
            Msg::Ui(UiMsg::KeySaved {
                provider,
                value,
                result,
            })
        }));
    }

    pub(super) fn key_saved(
        &mut self,
        provider: CleanupProvider,
        value: String,
        result: Result<(), String>,
    ) {
        let page = &mut self.ui.cleanup;
        // the field moved on to another provider's key
        if page.key_provider != Some(provider) {
            if let Err(error) = result {
                tracing::error!(%error, provider = provider.id(), "could not save a cleanup API key");
            }
            return;
        }
        match result {
            Ok(()) => {
                // show the trimmed value saved, unless typing moved on (CLN-24)
                if page.text != value && page.text.trim() == value {
                    page.text.clone_from(&value);
                    page.revision += 1;
                }
                page.baseline = KeyBaseline::Known(value);
                page.error = None;
                // a new key may unlock, or change, what the provider lists
                let with_key = provider.model_listing() == ModelListing::WithKey;
                if with_key && page.dialog == Some(provider) {
                    self.list_cleanup_models(provider);
                }
            }
            Err(error) => {
                tracing::error!(%error, provider = provider.id(), "could not save a cleanup API key");
                page.error = Some(error);
            }
        }
    }

    /// The save quitting must finish first (CLN-25), if one is due. It is
    /// queued now, behind every call already issued; the returned closure
    /// blocks until it is done.
    pub(super) fn pending_key_save(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
        let page = &self.ui.cleanup;
        let provider = page.key_provider?;
        let value = written_value(key_write(&page.baseline, &page.text)?);
        let secrets = self.ports.secrets.clone();
        let saved = self
            .key_queue()
            .run(move || secrets.save_api_key(provider, &value));
        Some(Box::new(move || {
            if let Err(error) = saved.blocking_recv().unwrap_or_else(lost) {
                tracing::error!(%error, provider = provider.id(), "could not save a cleanup API key at quit");
            }
        }))
    }

    /// A model row or the use-by-ID field in the sheet: `provider` and its
    /// model become the ones in use. A blank or spaced ID is ignored.
    pub(super) fn select_cleanup_model(&mut self, provider: CleanupProvider, raw: &str) {
        let Some(slug) = custom_model_slug(raw) else {
            tracing::warn!("blank or malformed cleanup model slug from the shell");
            return;
        };
        if provider != self.settings.cleanup_provider {
            self.apply_setting(SettingChange::CleanupProvider(provider));
        }
        let model = provider.resolve_model(Some(&slug));
        self.apply_setting(SettingChange::CleanupModel(provider, model));
        // the result resets when the model changes (CLN-27)
        self.cancel_cleanup_test();
        self.ui.cleanup.test_result = None;
        self.ui.cleanup.test_error = None;
    }

    // lists once per provider per run; a failed listing retries when asked
    // again
    fn list_models_once(&mut self, provider: CleanupProvider) {
        if provider.model_listing() == ModelListing::None {
            return;
        }
        let page = &self.ui.cleanup;
        let busy = page.listing.as_ref().is_some_and(|(p, _)| *p == provider);
        if !page.listed.contains_key(&provider) && !busy {
            self.list_cleanup_models(provider);
        }
    }

    /// "Refresh Models" in the sheet.
    pub(super) fn refresh_cleanup_models(&mut self) {
        if let Some(provider) = self.ui.cleanup.dialog {
            self.list_cleanup_models(provider);
        }
    }

    // replaces a listing in flight
    fn list_cleanup_models(&mut self, provider: CleanupProvider) {
        // the key is read behind every save already issued (CLN-25)
        let stored = (provider.model_listing() == ModelListing::WithKey).then(|| {
            let secrets = self.ports.secrets.clone();
            self.key_queue().run(move || secrets.api_key(provider))
        });
        let transport = self.ports.cleanup.clone();
        let page = &mut self.ui.cleanup;
        page.list_generation += 1;
        page.list_notice = None;
        let generation = page.list_generation;
        let task = self.spawn(async move {
            let key = match stored {
                Some(stored) => match stored.await.unwrap_or_else(lost) {
                    Ok(key) => key.and_then(|key: String| ApiKey::new(&key).ok()),
                    Err(error) => {
                        tracing::warn!(%error, provider = provider.id(), "could not read the key to list models");
                        None
                    }
                },
                None => None,
            };
            let result = transport.list_models(provider, key).await;
            Msg::Ui(UiMsg::ModelsListed {
                generation,
                provider,
                result,
            })
        });
        self.ui.cleanup.listing = Some((provider, task));
    }

    pub(super) fn models_listed(
        &mut self,
        generation: u64,
        provider: CleanupProvider,
        result: Result<Vec<CleanupModel>, CleanupError>,
    ) {
        let page = &mut self.ui.cleanup;
        if generation != page.list_generation {
            return;
        }
        page.listing = None;
        match result {
            Ok(models) => {
                page.listed.insert(provider, models);
            }
            Err(error) => page.list_notice = Some((provider, models_notice(provider, &error))),
        }
    }

    // CLN-27: save a just-typed key before testing the selected provider
    pub(super) fn test_cleanup(&mut self) {
        if self.ui.cleanup.test.is_some() {
            return;
        }
        let page = &mut self.ui.cleanup;
        page.test_result = None;
        page.test_error = None;
        page.test_generation += 1;
        let generation = page.test_generation;
        // a just-typed key is saved for its own provider; the test then uses
        // the selected provider's stored key
        let write = page
            .key_provider
            .zip(key_write(&page.baseline, &page.text).map(written_value));
        let secrets = self.ports.secrets.clone();
        let transport = self.ports.cleanup.clone();
        let provider = self.settings.cleanup_provider;
        let model = self.settings.cleanup_model.clone();
        // the saved prompt as of now; a save during the test is not picked up
        let prompt = self.settings.cleanup_prompt.clone();
        let messages = self.messages.clone();
        let queue = self.key_queue();
        let saved = write.map(|(saved_for, value)| {
            let saver = secrets.clone();
            let saving = value.clone();
            let saved = queue.run(move || saver.save_api_key(saved_for, &saving));
            (saved_for, value, saved)
        });
        let stored = provider
            .uses_key()
            .then(|| queue.run(move || secrets.api_key(provider)));
        let task = self.spawn(async move {
            if let Some((saved_for, value, saved)) = saved {
                let result = saved.await.unwrap_or_else(lost);
                let _ = messages.send(Msg::Ui(UiMsg::KeySaved {
                    provider: saved_for,
                    value,
                    result,
                }));
            }
            let result = match test_key(provider, &transport, stored).await {
                Ok(key) => {
                    let request =
                        build_request(provider, key.as_ref(), &model, &prompt, TEST_SAMPLE, None);
                    match refine(transport.as_ref(), request, &prompt, TEST_SAMPLE).await {
                        CleanupOutcome::Cleaned { text } => Ok(text),
                        CleanupOutcome::FellBack { error, .. } => Err(error),
                        CleanupOutcome::NotRun => Ok(TEST_SAMPLE.to_owned()),
                    }
                }
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
        let listed = |provider| page.listed.get(&provider).map_or(&[][..], Vec::as_slice);
        let key = page
            .dialog
            .and_then(ApiKeyFieldView::for_provider)
            .map(|field| ApiKeyFieldView {
                text: page.text.clone(),
                revision: page.revision,
                loading: page.loading,
                error: page.error.clone(),
                ..field
            });
        let provider = self.settings.cleanup_provider;
        // Apple Intelligence is offered only on a Mac that can run it,
        // unless it is already in use
        let offered: Vec<CleanupProvider> = CleanupProvider::ALL
            .into_iter()
            .filter(|p| {
                !p.on_device()
                    || page.on_device != OnDeviceStatus::DeviceNotEligible
                    || *p == provider
            })
            .collect();
        let dialog = page.dialog.map(|shown| ProvidersDialogInputs {
            provider: shown,
            offered: &offered,
            status_message: shown
                .on_device()
                .then(|| page.on_device.message())
                .flatten(),
            listed_models: listed(shown),
            models_loading: page.listing.as_ref().is_some_and(|(p, _)| *p == shown),
            models_notice: page
                .list_notice
                .as_ref()
                .filter(|(p, _)| *p == shown)
                .map(|(_, notice)| notice),
            key: key.as_ref(),
        });
        post_processing_page(PostProcessingInputs {
            enabled: self.settings.cleanup_enabled,
            provider,
            model: &self.settings.cleanup_model,
            listed_models: listed(provider),
            dialog,
            prompt: &self.settings.cleanup_prompt,
            prompt_draft: page.prompt_draft.as_ref().map(|draft| PromptDraftInputs {
                text: &draft.text,
                revision: page.prompt_revision,
                save_error: draft.save_error.as_deref(),
            }),
            prompt_status: page.prompt_status,
            test_running: page.test.is_some(),
            test_result: page.test_result.as_ref(),
            test_error: page.test_error.as_deref(),
        })
    }
}
