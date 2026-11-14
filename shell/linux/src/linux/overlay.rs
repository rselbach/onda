//! The recording overlay: the core's `OverlayView` on a floating panel.

mod draw;
mod panel;
#[cfg(test)]
mod x11_tests;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::{cairo, gdk, glib, prelude::*};
use onda_domain::overlay::{OverlayContent, OverlayIcon, OverlayView};

use panel::Panel;

// the processing highlight crosses the dots once in this long (OVL-10)
const SWEEP: Duration = Duration::from_millis(1050);
const SWEEP_FRAME: Duration = Duration::from_millis(33);

pub struct Overlay {
    panel: Rc<Panel>,
    view: Rc<RefCell<Option<OverlayView>>>,
    icon: Rc<RefCell<Option<cairo::ImageSurface>>>,
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
        let icon = Rc::new(RefCell::new(None::<cairo::ImageSurface>));
        let animated =
            gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations());
        let started = Instant::now();
        let shown = view.clone();
        let shown_icon = icon.clone();
        let panel = Panel::new(
            "onda-recording",
            draw::SIZE,
            move |cr| {
                let Some(view) = &*shown.borrow() else {
                    return Ok(());
                };
                let sweep = animated
                    .then(|| (started.elapsed().as_secs_f64() / SWEEP.as_secs_f64()).fract());
                draw::recording(cr, view, shown_icon.borrow().as_ref(), sweep)
            },
            cancel,
        );
        Self {
            panel: Rc::new(panel),
            view,
            icon,
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
        let renamed = self
            .view
            .borrow()
            .as_ref()
            .is_none_or(|shown| shown.icon != view.icon);
        if renamed {
            self.icon.replace(icon(&view.icon));
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

/// The picture of the application the core names, at twice the size it is
/// drawn. Onda's own icon is never shown, as on macOS.
fn icon(icon: &OverlayIcon) -> Option<cairo::ImageSurface> {
    // a session without a target has no frontmost application to show
    // either: the shell could not name the focused one when it started
    let OverlayIcon::App(app_id) = icon else {
        return None;
    };
    if app_id == super::APP_ID {
        return None;
    }
    let icon = super::target::icon(app_id)?;
    let side = 2 * draw::ICON as i32;
    let picture = gtk::IconTheme::for_display(&gdk::Display::default()?).lookup_by_gicon(
        &icon,
        draw::ICON as i32,
        2,
        gtk::TextDirection::None,
        gtk::IconLookupFlags::empty(),
    );
    let snapshot = gtk::Snapshot::new();
    picture.snapshot(&snapshot, f64::from(side), f64::from(side));
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, side, side).ok()?;
    snapshot
        .to_node()?
        .draw(&cairo::Context::new(&surface).ok()?);
    Some(surface)
}

impl Drop for Overlay {
    fn drop(&mut self) {
        if let Some(sweep) = self.sweep.take() {
            sweep.remove();
        }
    }
}
