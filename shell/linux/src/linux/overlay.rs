//! A passive recording indicator, including GNOME's XWayland fallback.

use adw::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use onda_domain::overlay::{OverlayContent, OverlayView};
use x11rb::{
    connection::Connection,
    protocol::{
        shape::{self, ConnectionExt as _},
        xproto::*,
    },
    rust_connection::RustConnection,
};

pub enum Overlay {
    Layer {
        window: gtk::Window,
        label: gtk::Label,
        meter: gtk::LevelBar,
    },
    X11 {
        connection: Box<RustConnection>,
        window: u32,
        gc: u32,
    },
    Unavailable,
}

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
    pub fn new() -> Self {
        if super::process::wayland() && gtk4_layer_shell::is_supported() {
            let window = gtk::Window::builder()
                .title("Onda recording")
                .decorated(false)
                .focusable(false)
                .default_width(360)
                .build();
            window.init_layer_shell();
            window.set_namespace(Some("onda-recording"));
            window.set_layer(Layer::Overlay);
            window.set_keyboard_mode(KeyboardMode::None);
            window.set_anchor(Edge::Bottom, true);
            window.set_margin(Edge::Bottom, 36);
            let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
            content.set_margin_top(14);
            content.set_margin_bottom(14);
            content.set_margin_start(22);
            content.set_margin_end(22);
            let label = gtk::Label::new(Some("Recording"));
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_max_width_chars(60);
            let meter = gtk::LevelBar::new();
            content.append(&label);
            content.append(&meter);
            window.set_child(Some(&content));
            window.connect_realize(|window| {
                if let Some(surface) = window.surface() {
                    surface.set_input_region(Some(&gtk::cairo::Region::create()));
                }
            });
            return Self::Layer {
                window,
                label,
                meter,
            };
        }
        match Self::x11() {
            Ok(overlay) => overlay,
            Err(error) => {
                tracing::warn!(%error, "recording overlay unavailable");
                Self::Unavailable
            }
        }
    }

    fn x11() -> anyhow::Result<Self> {
        let (connection, screen) = x11rb::connect(None)?;
        let screen = &connection.setup().roots[screen];
        let window = connection.generate_id()?;
        let gc = connection.generate_id()?;
        connection
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                window,
                screen.root,
                (screen.width_in_pixels.saturating_sub(420) / 2) as i16,
                screen.height_in_pixels.saturating_sub(95) as i16,
                420,
                52,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new()
                    .override_redirect(1)
                    .background_pixel(screen.black_pixel),
            )?
            .check()?;
        connection
            .shape_rectangles(
                shape::SO::SET,
                shape::SK::INPUT,
                ClipOrdering::UNSORTED,
                window,
                0,
                0,
                &[],
            )?
            .check()?;
        connection
            .create_gc(
                gc,
                window,
                &CreateGCAux::new()
                    .foreground(screen.white_pixel)
                    .background(screen.black_pixel),
            )?
            .check()?;
        connection.flush()?;
        Ok(Self::X11 {
            connection: Box::new(connection),
            window,
            gc,
        })
    }

    pub fn update(&self, view: &OverlayView) {
        let (text, level) = content(view);
        match self {
            Self::Layer {
                window,
                label,
                meter,
            } => {
                label.set_text(&text);
                meter.set_value(level);
                window.set_visible(view.visible);
            }
            Self::X11 {
                connection,
                window,
                gc,
            } => {
                let draw = || -> anyhow::Result<()> {
                    if !view.visible {
                        connection.unmap_window(*window)?;
                    } else {
                        connection.map_window(*window)?;
                        connection.clear_area(false, *window, 0, 0, 0, 0)?;
                        let text: Vec<u8> = text
                            .chars()
                            .take(65)
                            .map(|c| if c.is_ascii() { c as u8 } else { b' ' })
                            .collect();
                        connection.image_text8(*window, *gc, 14, 22, &text)?;
                        connection.poly_fill_rectangle(
                            *window,
                            *gc,
                            &[Rectangle {
                                x: 14,
                                y: 35,
                                width: (390.0 * level.clamp(0.0, 1.0)) as u16,
                                height: 3,
                            }],
                        )?;
                    }
                    connection.flush()?;
                    Ok(())
                };
                if let Err(error) = draw() {
                    tracing::debug!(%error, "drawing recording overlay");
                }
            }
            Self::Unavailable => {}
        }
    }
}
