//! The overlay on a real X server, driven by the core's overlay state.
//! `scripts/test-linux-overlay.sh` runs it in Xvfb.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::{cairo, gdk, glib, prelude::*};
use onda_domain::overlay::{ClockDisplay, Notice, Overlay as CoreOverlay, OverlayView};
use x11rb::{
    connection::Connection,
    protocol::{Event, xproto::*, xtest::ConnectionExt as _},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use super::{Overlay, draw};

const JAPANESE: &str = "Troy Barnes が Greendale で話す";
const DESKTOP: (u8, u8, u8) = (0x20, 0x40, 0x60);
const KEY_A: u8 = 38;

struct Screen {
    connection: RustConnection,
    root: Window,
    target: Window,
}

#[derive(Debug, Default, PartialEq)]
struct Received {
    presses: usize,
    keys: usize,
}

fn settle() {
    let until = Instant::now() + Duration::from_millis(200);
    let context = glib::MainContext::default();
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

impl Screen {
    // the application the user dictates into: a focused window under the overlay
    fn new() -> Self {
        let (connection, screen) = x11rb::connect(None).unwrap();
        let screen = connection.setup().roots[screen].clone();
        let target = connection.generate_id().unwrap();
        let (red, green, blue) = DESKTOP;
        let background = u32::from_be_bytes([0, red, green, blue]);
        connection
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                target,
                screen.root,
                0,
                0,
                screen.width_in_pixels,
                screen.height_in_pixels,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new()
                    .background_pixel(background)
                    .event_mask(EventMask::BUTTON_PRESS | EventMask::KEY_PRESS),
            )
            .unwrap();
        connection.map_window(target).unwrap();
        connection
            .set_input_focus(InputFocus::POINTER_ROOT, target, x11rb::CURRENT_TIME)
            .unwrap();
        connection.sync().unwrap();
        Self {
            connection,
            root: screen.root,
            target,
        }
    }

    fn overlay_window(&self) -> Window {
        let children = self
            .connection
            .query_tree(self.root)
            .unwrap()
            .reply()
            .unwrap()
            .children;
        children
            .into_iter()
            .find(|window| {
                self.connection
                    .get_property(false, *window, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 64)
                    .unwrap()
                    .reply()
                    .is_ok_and(|name| name.value == b"onda-recording")
            })
            .expect("the overlay window")
    }

    fn origin(&self) -> (i16, i16) {
        let geometry = self
            .connection
            .get_geometry(self.overlay_window())
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(
            (i32::from(geometry.width), i32::from(geometry.height)),
            draw::SIZE
        );
        (geometry.x, geometry.y)
    }

    fn mapped(&self) -> bool {
        let attributes = self
            .connection
            .get_window_attributes(self.overlay_window())
            .unwrap()
            .reply()
            .unwrap();
        assert!(
            attributes.override_redirect,
            "no window manager may focus it"
        );
        attributes.map_state == MapState::VIEWABLE
    }

    // what the user sees where the overlay sits, as B, G, R, unused
    fn picture(&self, name: &str) -> Vec<u8> {
        let (x, y) = self.origin();
        let (width, height) = (draw::SIZE.0 as u16, draw::SIZE.1 as u16);
        let image = |x, y, width, height| {
            self.connection
                .get_image(ImageFormat::Z_PIXMAP, self.root, x, y, width, height, !0)
                .unwrap()
                .reply()
                .unwrap()
                .data
        };
        if let Some(directory) = std::env::var_os("ONDA_TEST_SCREENSHOT_DIR") {
            let (width, height) = (width + 48, height + 48);
            let mut pixels = image(x - 24, y - 24, width, height);
            pixels
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .for_each(|pixel| pixel[3] = 0xff);
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            gdk::MemoryTexture::new(
                width.into(),
                height.into(),
                gdk::MemoryFormat::B8g8r8a8,
                &glib::Bytes::from_owned(pixels),
                usize::from(width) * 4,
            )
            .save_to_png(directory.join(format!("{name}.png")))
            .unwrap();
        }
        image(x, y, width, height)
    }

    fn press(&self, x: i32, y: i32) {
        let (left, top) = self.origin();
        let fake = |kind, detail, x, y| {
            self.connection
                .xtest_fake_input(kind, detail, x11rb::CURRENT_TIME, self.root, x, y, 0)
                .unwrap();
        };
        fake(MOTION_NOTIFY_EVENT, 0, left + x as i16, top + y as i16);
        fake(BUTTON_PRESS_EVENT, 1, 0, 0);
        fake(BUTTON_RELEASE_EVENT, 1, 0, 0);
        self.connection.sync().unwrap();
        settle();
    }

    fn type_a(&self) {
        for kind in [KEY_PRESS_EVENT, KEY_RELEASE_EVENT] {
            self.connection
                .xtest_fake_input(kind, KEY_A, x11rb::CURRENT_TIME, self.root, 0, 0, 0)
                .unwrap();
        }
        self.connection.sync().unwrap();
    }

    // what reached the target application since the last call
    fn received(&self) -> Received {
        let mut received = Received::default();
        while let Some(event) = self.connection.poll_for_event().unwrap() {
            match event {
                Event::ButtonPress(_) => received.presses += 1,
                Event::KeyPress(_) => received.keys += 1,
                _ => {}
            }
        }
        received
    }

    fn focused(&self) -> bool {
        self.connection
            .get_input_focus()
            .unwrap()
            .reply()
            .unwrap()
            .focus
            == self.target
    }
}

fn drawn(view: &OverlayView) -> Vec<u8> {
    let mut surface =
        cairo::ImageSurface::create(cairo::Format::ARgb32, draw::SIZE.0, draw::SIZE.1).unwrap();
    draw::recording(&cairo::Context::new(&surface).unwrap(), view, None).unwrap();
    surface.data().unwrap().to_vec()
}

// the screen shows the drawing where it is opaque and the desktop where the
// rounded corners leave it empty
#[track_caller]
fn assert_on_screen(screen: &[u8], view: &OverlayView, name: &str) {
    let (red, green, blue) = DESKTOP;
    let (mut opaque, mut empty) = (0, 0);
    let drawn = drawn(view);
    for (seen, want) in screen
        .as_chunks::<4>()
        .0
        .iter()
        .zip(drawn.as_chunks::<4>().0)
    {
        match want[3] {
            0xff => {
                assert_eq!(seen[..3], want[..3], "{name}: a drawn pixel");
                opaque += 1;
            }
            0 => {
                assert_eq!(seen[..3], [blue, green, red], "{name}: a corner pixel");
                empty += 1;
            }
            _ => {}
        }
    }
    assert!(
        opaque > 40_000 && empty > 80,
        "{name}: {opaque} drawn, {empty} empty"
    );
}

#[test]
#[ignore = "needs an X11 display of its own; run scripts/test-linux-overlay.sh"]
fn x11_overlay_shows_each_state_and_cancels_without_taking_focus() {
    gtk::init().unwrap();
    let cancelled = Rc::new(Cell::new(0));
    let overlay = Overlay::new({
        let cancelled = cancelled.clone();
        move || cancelled.set(cancelled.get() + 1)
    });
    assert!(overlay.available());
    // opened after the overlay, as most applications are
    let screen = Screen::new();
    let clock = |text: &str, warning| ClockDisplay {
        text: text.to_owned(),
        warning,
    };
    let (cancel_x, cancel_y) = (draw::SIZE.0 - 22, draw::SIZE.1 / 2);
    let show = |view: &OverlayView| {
        overlay.update(view);
        settle();
    };

    let mut core = CoreOverlay::new();
    core.show_recording(None, Some(JAPANESE.to_owned()));
    for step in 0..60 {
        let wave = (f64::from(step) * 0.45).sin().abs() * (f64::from(step) * 0.11).sin().abs();
        core.push_level(0.004 + 0.09 * wave as f32);
    }
    let waveform = core.view(Some(&clock("0:07", false)));
    show(&waveform);
    assert!(screen.mapped());
    assert_on_screen(&screen.picture("waveform"), &waveform, "waveform");
    assert!(screen.focused(), "showing the overlay left the focus alone");

    screen.received();
    screen.press(120, draw::SIZE.1 / 2);
    assert_eq!(cancelled.get(), 0);
    assert_eq!(
        screen.received().presses,
        1,
        "a press on the waveform passes through"
    );

    let warning = core.view(Some(&clock("Stops in 0:10", true)));
    show(&warning);
    assert_on_screen(&screen.picture("clock-warning"), &warning, "clock warning");

    core.set_preview(format!(
        "{JAPANESE}。Abed is filming the study group in the library."
    ));
    let preview = core.view(Some(&clock("0:12", false)));
    show(&preview);
    assert_on_screen(&screen.picture("preview"), &preview, "preview");

    core.set_silent(true);
    let silent = core.view(Some(&clock("0:14", false)));
    show(&silent);
    assert_on_screen(
        &screen.picture("silent-microphone"),
        &silent,
        "silent microphone",
    );

    screen.press(cancel_x, cancel_y);
    assert_eq!(cancelled.get(), 1, "a press on the button cancels");
    assert_eq!(
        screen.received().presses,
        0,
        "the press stayed on the button"
    );
    assert!(screen.focused(), "the press left the focus alone");
    screen.type_a();
    settle();
    assert_eq!(screen.received().keys, 1, "typing still reaches the target");

    core.enter_transcribing(None);
    let processing = core.view(None);
    show(&processing);
    let early = screen.picture("processing");
    settle();
    assert!(
        early != screen.picture("processing-later"),
        "the highlight moves"
    );
    screen.press(cancel_x, cancel_y);
    assert_eq!(cancelled.get(), 2, "processing can be cancelled too");

    core.show_notice(
        Notice {
            title: "Couldn't Save Recording".to_owned(),
            message: "Abed asked the study group to wait in the library until the \
                Dreamatorium is clean, and then Troy Barnes started a recording that \
                Greendale could not save."
                .to_owned(),
        },
        Duration::ZERO,
    );
    let notice = core.view(None);
    show(&notice);
    assert_on_screen(&screen.picture("notice"), &notice, "notice");
    screen.received();
    screen.press(cancel_x, cancel_y);
    assert_eq!(cancelled.get(), 2, "a notice has no button");
    assert_eq!(screen.received().presses, 1, "so the press passes through");

    core.end_session();
    core.tick(Duration::from_secs(4));
    show(&core.view(None));
    assert!(
        !screen.mapped(),
        "a hidden view takes the overlay off screen"
    );
    assert!(screen.focused());
}
