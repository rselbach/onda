//! Which model loads are running, and which download one waits on (ASR-1,
//! ASR-2, ASR-9, DL-11).

use std::num::NonZeroUsize;

use crate::models::ModelId;

/// The loads the session has started and not finished, plus the download a
/// load or redownload is waiting for. Only the methods here change it, so a
/// load and a download wait cannot stay active together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelLoads {
    state: State,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Idle {
        // a load was announced before the service reported it, so the tray
        // shows loading at once (ASR-1)
        announced: bool,
    },
    // the download a load or redownload waits for (ASR-2, DL-11)
    WaitingForDownload(ModelId),
    // loads started and not yet finished; two can overlap across a switch
    Loading {
        running: NonZeroUsize,
        announced: bool,
    },
}

impl Default for State {
    fn default() -> Self {
        Self::Idle { announced: false }
    }
}

impl ModelLoads {
    /// No load running.
    pub fn new() -> Self {
        Self::default()
    }

    /// A load runs, or waits for a download.
    pub fn in_flight(&self) -> bool {
        matches!(
            self.state,
            State::WaitingForDownload(_) | State::Loading { .. }
        )
    }

    /// The tray shows loading before the service has reported a state.
    pub fn announced(&self) -> bool {
        match self.state {
            State::Idle { announced } | State::Loading { announced, .. } => announced,
            State::WaitingForDownload(_) => false,
        }
    }

    /// A load is about to start and should show as loading now (ASR-1).
    pub fn announce(&mut self) {
        self.state = match self.state {
            State::Idle { .. } | State::WaitingForDownload(_) => State::Idle { announced: true },
            State::Loading { running, .. } => State::Loading {
                running,
                announced: true,
            },
        };
    }

    /// A load started, replacing any download wait. Follow it with
    /// [`Self::finished`] or wait on a download.
    pub fn started(&mut self) {
        self.state = match self.state {
            State::Idle { announced } => State::Loading {
                running: NonZeroUsize::MIN,
                announced,
            },
            State::WaitingForDownload(_) => State::Loading {
                running: NonZeroUsize::MIN,
                announced: false,
            },
            State::Loading { running, announced } => State::Loading {
                running: running.checked_add(1).unwrap_or(running),
                announced,
            },
        };
    }

    /// A load finished. The announcement ends with the last one.
    pub fn finished(&mut self) {
        if let State::Loading { running, announced } = self.state {
            self.state = match NonZeroUsize::new(running.get() - 1) {
                Some(running) => State::Loading { running, announced },
                None => State::Idle { announced: false },
            };
        }
    }

    /// The service reported a state, so it now speaks for the load.
    pub fn reported(&mut self) {
        if let State::Idle { announced } | State::Loading { announced, .. } = &mut self.state {
            *announced = false;
        }
    }

    /// The model whose download a load or redownload waits on, if any.
    pub fn waiting_for(&self) -> Option<ModelId> {
        match self.state {
            State::WaitingForDownload(model) => Some(model),
            State::Idle { .. } | State::Loading { .. } => None,
        }
    }

    /// Cancels a pending download wait without touching an active load.
    pub fn clear_wait(&mut self) {
        if matches!(self.state, State::WaitingForDownload(_)) {
            self.state = State::Idle { announced: false };
        }
    }

    /// The download a load or redownload waits on (ASR-2, DL-11).
    pub fn wait_for_download(&mut self, model: ModelId) {
        self.state = State::WaitingForDownload(model);
    }

    /// `model`'s download ended, by finishing, failing, or cancelling. True
    /// when a load was waiting on it.
    pub fn download_ended(&mut self, model: ModelId) -> bool {
        if self.state != State::WaitingForDownload(model) {
            return false;
        }
        self.state = State::Idle { announced: false };
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

    #[test]
    fn starting_a_load_clears_a_download_wait() {
        let mut loads = ModelLoads::new();
        loads.wait_for_download(model());
        loads.started();
        assert!(!loads.download_ended(model()));
        loads.finished();
        assert!(!loads.in_flight());
        assert!(!loads.announced());
    }

    #[test]
    fn announcing_clears_a_download_wait() {
        let mut loads = ModelLoads::new();
        loads.wait_for_download(model());
        loads.announce();
        assert!(loads.announced());
        assert!(!loads.download_ended(model()));
        loads.started();
        assert!(loads.announced());
        assert!(loads.in_flight());
    }
}
