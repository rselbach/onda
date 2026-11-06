//! Which model loads are running, and which download one waits on (ASR-1,
//! ASR-2, ASR-9, DL-11).

use crate::models::ModelId;

/// The loads the session has started and not finished, plus the download a
/// load or redownload is waiting for. Only the methods here change it, so the
/// rules below hold everywhere.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelLoads {
    // loads started and not yet finished; two can overlap across a switch
    running: usize,
    // a load was announced before the service reported it, so the tray shows
    // loading at once (ASR-1)
    announced: bool,
    // the download a load or redownload waits for (ASR-2, DL-11)
    download: Option<ModelId>,
}

impl ModelLoads {
    /// No load running.
    pub fn new() -> Self {
        Self::default()
    }

    /// A load runs, or waits for a download.
    pub fn in_flight(&self) -> bool {
        self.running > 0 || self.download.is_some()
    }

    /// The tray shows loading before the service has reported a state.
    pub fn announced(&self) -> bool {
        self.announced
    }

    /// A load is about to start and should show as loading now (ASR-1).
    pub fn announce(&mut self) {
        self.announced = true;
    }

    /// A load started. Follow it with [`Self::finished`] or wait on a download.
    pub fn started(&mut self) {
        self.running += 1;
    }

    /// A load finished. The announcement ends with the last one.
    pub fn finished(&mut self) {
        self.running = self.running.saturating_sub(1);
        if self.running == 0 {
            self.announced = false;
        }
    }

    /// The service reported a state, so it now speaks for the load.
    pub fn reported(&mut self) {
        self.announced = false;
    }

    /// The load waits for `model` to download (ASR-2), or a redownload
    /// replaces it (DL-11).
    pub fn wait_for_download(&mut self, model: ModelId) {
        self.announced = false;
        self.download = Some(model);
    }

    /// `model`'s download ended, by finishing, failing, or cancelling. True
    /// when a load was waiting on it.
    pub fn download_ended(&mut self, model: ModelId) -> bool {
        if self.download != Some(model) {
            return false;
        }
        self.download = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> ModelId {
        ModelId::ParakeetTdtV2
    }

    #[test]
    fn announcement_lasts_until_the_last_load_finishes() {
        let mut loads = ModelLoads::new();
        loads.announce();
        loads.started();
        loads.started();
        loads.finished();
        assert!(loads.announced(), "one load still running");
        loads.finished();
        assert!(!loads.announced());
        assert!(!loads.in_flight());
    }

    #[test]
    fn a_report_ends_the_announcement_but_not_the_load() {
        let mut loads = ModelLoads::new();
        loads.announce();
        loads.started();
        loads.reported();
        assert!(!loads.announced());
        assert!(loads.in_flight());
    }

    #[test]
    fn waiting_on_a_download_counts_as_in_flight_without_a_load() {
        let mut loads = ModelLoads::new();
        loads.announce();
        loads.wait_for_download(model());
        assert!(loads.in_flight());
        assert!(
            !loads.announced(),
            "the download, not the announcement, shows progress"
        );
        assert!(!loads.download_ended(ModelId::ParakeetUltra));
        assert!(loads.download_ended(model()));
        assert!(!loads.in_flight());
    }

    #[test]
    fn finishing_loads_never_underflow() {
        let mut loads = ModelLoads::new();
        loads.finished();
        assert_eq!(loads, ModelLoads::new());
    }
}
