//! The floating overlay: the core's recording overlay and its download
//! overlay, on one panel.

mod draw;
mod panel;
#[cfg(test)]
mod x11_tests;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::{cairo, gdk, glib, prelude::*};
use onda_domain::{
    non_dictation_ui::{DownloadOverlayView, OverlayProgress},
    overlay::{OverlayContent, OverlayIcon, OverlayView},
};

use panel::Panel;

// the processing highlight crosses the dots once in this long (OVL-10), and
// so does the bar of a download overlay without a fraction
const SWEEP: Duration = Duration::from_millis(1050);
const SWEEP_FRAME: Duration = Duration::from_millis(33);

pub struct Overlay {
    panel: Rc<Panel>,
    views: Rc<RefCell<Views>>,
    icon: Rc<RefCell<Option<cairo::ImageSurface>>>,
    animated: bool,
    sweep: RefCell<Option<glib::SourceId>>,
}

/// What a press on the panel's button asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// Cancel the dictation: the recording overlay's button.
    Cancel,
    /// Hide the download overlay: its own button.
    Hide,
}

/// What the core last asked each overlay to show. The two arrive in separate
/// events, in either order, and share the panel.
struct Views {
    recording: Option<OverlayView>,
    download: DownloadOverlayView,
}

/// The overlay on the panel.
enum Front<'a> {
    Recording(&'a OverlayView),
    Download {
        status: &'a str,
        progress: OverlayProgress,
    },
}

impl Views {
    /// The recording overlay while it is visible, and otherwise the download
    /// overlay. The core hides the download for a dictation or a notice
    /// (OVL-18) in a later event than the one that shows the recording.
    fn front(&self) -> Option<Front<'_>> {
        if let Some(view) = self.recording.as_ref().filter(|view| view.visible) {
            return Some(Front::Recording(view));
        }
        match &self.download {
            DownloadOverlayView::Shown { status, progress } => Some(Front::Download {
                status,
                progress: *progress,
            }),
            DownloadOverlayView::Hidden => None,
        }
    }
}

impl Front<'_> {
    fn button(&self) -> Option<cairo::RectangleInt> {
        match self {
            Front::Recording(view) => draw::cancel_area(view),
            Front::Download { .. } => Some(draw::button_area()),
        }
    }

    fn press(&self) -> Press {
        match self {
            Front::Recording(_) => Press::Cancel,
            Front::Download { .. } => Press::Hide,
        }
    }

    // whether the drawing changes with time
    fn moves(&self) -> bool {
        match self {
            Front::Recording(view) => matches!(view.content, OverlayContent::Processing { .. }),
            Front::Download { progress, .. } => *progress == OverlayProgress::Indeterminate,
        }
    }

    fn draw(
        &self,
        cr: &cairo::Context,
        icon: Option<&cairo::ImageSurface>,
        sweep: Option<f64>,
    ) -> Result<(), cairo::Error> {
        match self {
            Front::Recording(view) => draw::recording(cr, view, icon, sweep),
            Front::Download { status, progress } => draw::download(cr, status, *progress, sweep),
        }
    }
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
    /// A hidden overlay that reports each press on its button to `pressed`.
    pub fn new(pressed: impl Fn(Press) + 'static) -> Self {
        let views = Rc::new(RefCell::new(Views {
            recording: None,
            download: DownloadOverlayView::Hidden,
        }));
        let icon = Rc::new(RefCell::new(None::<cairo::ImageSurface>));
        let animated =
            gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations());
        let started = Instant::now();
        let (drawn, shown) = (views.clone(), views.clone());
        let shown_icon = icon.clone();
        let panel = Panel::new(
            "onda-recording",
            draw::SIZE,
            move |cr| {
                let views = drawn.borrow();
                let Some(front) = views.front() else {
                    return Ok(());
                };
                let sweep = animated
                    .then(|| (started.elapsed().as_secs_f64() / SWEEP.as_secs_f64()).fract());
                front.draw(cr, shown_icon.borrow().as_ref(), sweep)
            },
            move || {
                let press = shown.borrow().front().map(|front| front.press());
                if let Some(press) = press {
                    pressed(press);
                }
            },
        );
        Self {
            panel: Rc::new(panel),
            views,
            icon,
            animated,
            sweep: RefCell::new(None),
        }
    }

    pub fn available(&self) -> bool {
        self.panel.available()
    }

    /// Takes the core's recording overlay.
    pub fn update(&self, view: &OverlayView) {
        let renamed = self
            .views
            .borrow()
            .recording
            .as_ref()
            .is_none_or(|shown| shown.icon != view.icon);
        if renamed {
            self.icon.replace(icon(&view.icon));
        }
        self.views.borrow_mut().recording = Some(view.clone());
        self.refresh();
    }

    /// Takes the core's download overlay.
    pub fn download(&self, view: &DownloadOverlayView) {
        self.views.borrow_mut().download = view.clone();
        self.refresh();
    }

    fn refresh(&self) {
        let front = self
            .views
            .borrow()
            .front()
            .map(|front| (front.button(), front.moves()));
        let sweeping = self.animated && front.is_some_and(|(_, moves)| moves);
        if !sweeping && let Some(sweep) = self.sweep.take() {
            sweep.remove();
        }
        let Some((button, _)) = front else {
            self.panel.hide();
            return;
        };
        self.panel.show(button);
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

#[cfg(test)]
mod tests {
    use super::*;
    use onda_domain::overlay::Overlay as CoreOverlay;

    #[test]
    fn recording_is_in_front_of_the_download() {
        let recording = |visible| {
            let mut core = CoreOverlay::new();
            core.show_recording(None, Some("Abed's Podcast Mic".to_owned()));
            let mut view = core.view(None);
            view.visible = visible;
            Some(view)
        };
        let downloading = || DownloadOverlayView::Shown {
            status: "Downloading Greendale model… 42%".to_owned(),
            progress: OverlayProgress::Determinate(0.42),
        };
        struct Case {
            name: &'static str,
            recording: Option<OverlayView>,
            download: DownloadOverlayView,
            want: Option<Press>,
        }
        let cases = [
            Case {
                name: "nothing to show",
                recording: None,
                download: DownloadOverlayView::Hidden,
                want: None,
            },
            Case {
                name: "a download alone",
                recording: None,
                download: downloading(),
                want: Some(Press::Hide),
            },
            Case {
                name: "a recording before the core hides the download",
                recording: recording(true),
                download: downloading(),
                want: Some(Press::Cancel),
            },
            Case {
                name: "a recording after the core hid the download",
                recording: recording(true),
                download: DownloadOverlayView::Hidden,
                want: Some(Press::Cancel),
            },
            Case {
                name: "the download returns when the recording overlay hides",
                recording: recording(false),
                download: downloading(),
                want: Some(Press::Hide),
            },
            Case {
                name: "a hidden recording alone",
                recording: recording(false),
                download: DownloadOverlayView::Hidden,
                want: None,
            },
        ];
        for tc in cases {
            let views = Views {
                recording: tc.recording,
                download: tc.download,
            };
            let front = views.front();
            assert_eq!(front.as_ref().map(Front::press), tc.want, "{}", tc.name);
            let button = front.and_then(|front| front.button());
            assert_eq!(button.is_some(), tc.want.is_some(), "{}", tc.name);
        }
    }
}
