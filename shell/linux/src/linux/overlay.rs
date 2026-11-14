//! The recording overlay: the core's `OverlayView` on a floating panel.

mod draw;
mod panel;
#[cfg(test)]
mod x11_tests;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use onda_domain::overlay::{OverlayContent, OverlayView};

use panel::Panel;

// the processing highlight crosses the dots once in this long (OVL-10)
const SWEEP: Duration = Duration::from_millis(1050);
const SWEEP_FRAME: Duration = Duration::from_millis(33);

pub struct Overlay {
    panel: Rc<Panel>,
    view: Rc<RefCell<Option<OverlayView>>>,
    animated: bool,
    sweep: RefCell<Option<glib::SourceId>>,
}

/// The main window's one-line summary of the overlay, and the latest level.
pub fn content(view: &OverlayView) -> (String, f64) {
    let (text, level) = match &view.content {
        OverlayContent::Notice(notice) => (format!("{}: {}", notice.title, notice.message), 0.0),
        OverlayContent::Processing { label, .. } => ((*label).into(), 0.0),
        OverlayContent::SilentMicrophone { title, .. } => (title.clone(), 0.0),
        OverlayContent::Preview { text, levels } => {
            (text.clone(), levels.last().copied().unwrap_or(0.0) as f64)
        }
        OverlayContent::Waveform { levels } => (
            "Recording".into(),
            levels.last().copied().unwrap_or(0.0) as f64,
        ),
    };
    let clock = view
        .trailing
        .as_ref()
        .and_then(|trailing| trailing.clock.as_ref())
        .map(|clock| clock.text.as_str())
        .unwrap_or("");
    (format!("{text}  {clock}"), level)
}

impl Overlay {
    /// A hidden overlay whose cancel button calls `cancel`.
    pub fn new(cancel: impl Fn() + 'static) -> Self {
        let view = Rc::new(RefCell::new(None::<OverlayView>));
        let animated =
            gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations());
        let started = Instant::now();
        let shown = view.clone();
        let panel = Panel::new(
            "onda-recording",
            draw::SIZE,
            move |cr| {
                let Some(view) = &*shown.borrow() else {
                    return Ok(());
                };
                let sweep = animated
                    .then(|| (started.elapsed().as_secs_f64() / SWEEP.as_secs_f64()).fract());
                draw::recording(cr, view, sweep)
            },
            cancel,
        );
        Self {
            panel: Rc::new(panel),
            view,
            animated,
            sweep: RefCell::new(None),
        }
    }

    pub fn available(&self) -> bool {
        self.panel.available()
    }

    pub fn update(&self, view: &OverlayView) {
        let sweeping = view.visible
            && self.animated
            && matches!(view.content, OverlayContent::Processing { .. });
        if !sweeping && let Some(sweep) = self.sweep.take() {
            sweep.remove();
        }
        if !view.visible {
            self.panel.hide();
            return;
        }
        self.view.replace(Some(view.clone()));
        self.panel.show(draw::cancel_area(view));
        if sweeping && self.sweep.borrow().is_none() {
            let panel = self.panel.clone();
            let sweep = glib::timeout_add_local(SWEEP_FRAME, move || {
                panel.redraw();
                glib::ControlFlow::Continue
            });
            self.sweep.replace(Some(sweep));
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        if let Some(sweep) = self.sweep.take() {
            sweep.remove();
        }
    }
}
