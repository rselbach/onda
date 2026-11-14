//! A floating panel at the bottom of the screen. It never takes keyboard
//! focus and passes the pointer through, except on one button.
//!
//! A layer-shell surface where the compositor offers one, and otherwise an
//! override-redirect X11 window, which is what GNOME shows through XWayland.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{cairo, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        shape::{self, ConnectionExt as _},
        xproto::*,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use super::draw::RADIUS;

const BOTTOM_MARGIN: i32 = 40;

// how often a shown X11 panel reads its presses and exposures
const X11_EVENTS: Duration = Duration::from_millis(30);

type Paint = dyn Fn(&cairo::Context) -> Result<(), cairo::Error>;

pub struct Panel(Backend);

enum Backend {
    Layer {
        window: gtk::Window,
        area: gtk::DrawingArea,
        button: Rc<Cell<Option<cairo::RectangleInt>>>,
    },
    X11(Rc<X11>),
    Unavailable,
}

impl Panel {
    /// A hidden panel of `size` pixels that draws itself with `paint` and
    /// calls `pressed` for a press on its button.
    pub fn new(
        name: &str,
        size: (i32, i32),
        paint: impl Fn(&cairo::Context) -> Result<(), cairo::Error> + 'static,
        pressed: impl Fn() + 'static,
    ) -> Self {
        if crate::linux::process::wayland() && gtk4_layer_shell::is_supported() {
            return Self(layer(name, size, paint, pressed));
        }
        match X11::new(name, size, Box::new(paint), Box::new(pressed)) {
            Ok(x11) => Self(Backend::X11(x11)),
            Err(error) => {
                tracing::warn!(%error, name, "floating panel unavailable");
                Self(Backend::Unavailable)
            }
        }
    }

    pub fn available(&self) -> bool {
        !matches!(self.0, Backend::Unavailable)
    }

    /// Puts the panel on screen with its current drawing. Only `button`
    /// takes presses; the rest of the panel passes them through.
    pub fn show(&self, button: Option<cairo::RectangleInt>) {
        match &self.0 {
            Backend::Layer {
                window,
                area,
                button: pressable,
            } => {
                pressable.set(button);
                window.set_visible(true);
                if let Some(surface) = window.surface() {
                    let region = match button {
                        Some(button) => cairo::Region::create_rectangle(&button),
                        None => cairo::Region::create(),
                    };
                    surface.set_input_region(Some(&region));
                }
                area.queue_draw();
            }
            Backend::X11(x11) => x11.log(x11.show(button)),
            Backend::Unavailable => {}
        }
    }

    /// Draws a shown panel again, for an animation.
    pub fn redraw(&self) {
        match &self.0 {
            Backend::Layer { area, .. } => area.queue_draw(),
            Backend::X11(x11) => x11.log(x11.draw()),
            Backend::Unavailable => {}
        }
    }

    pub fn hide(&self) {
        match &self.0 {
            Backend::Layer { window, .. } => window.set_visible(false),
            Backend::X11(x11) => x11.log(x11.hide()),
            Backend::Unavailable => {}
        }
    }
}

fn contains(area: cairo::RectangleInt, x: f64, y: f64) -> bool {
    let (left, top) = (f64::from(area.x()), f64::from(area.y()));
    (left..left + f64::from(area.width())).contains(&x)
        && (top..top + f64::from(area.height())).contains(&y)
}

fn layer(
    name: &str,
    size: (i32, i32),
    paint: impl Fn(&cairo::Context) -> Result<(), cairo::Error> + 'static,
    pressed: impl Fn() + 'static,
) -> Backend {
    let window = gtk::Window::builder()
        .title(name)
        .decorated(false)
        .focusable(false)
        .build();
    window.init_layer_shell();
    window.set_namespace(Some(name));
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_anchor(Edge::Bottom, true);
    window.set_margin(Edge::Bottom, BOTTOM_MARGIN);
    // the panel paints its own rounded background
    window.add_css_class("onda-panel");
    let transparent = gtk::CssProvider::new();
    transparent.load_from_string("window.onda-panel { background: none; }");
    gtk::style_context_add_provider_for_display(
        &WidgetExt::display(&window),
        &transparent,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let area = gtk::DrawingArea::builder()
        .content_width(size.0)
        .content_height(size.1)
        .build();
    area.set_draw_func(move |_, cr, _, _| {
        if let Err(error) = paint(cr) {
            tracing::debug!(%error, "drawing a floating panel");
        }
    });
    let button = Rc::new(Cell::new(None));
    let click = gtk::GestureClick::new();
    let pressable = button.clone();
    click.connect_pressed(move |_, _, x, y| {
        if pressable.get().is_some_and(|button| contains(button, x, y)) {
            pressed();
        }
    });
    area.add_controller(click);
    window.set_child(Some(&area));
    Backend::Layer {
        window,
        area,
        button,
    }
}

struct X11 {
    connection: RustConnection,
    window: Window,
    gc: Gcontext,
    depth: u8,
    size: (u16, u16),
    paint: Box<Paint>,
    pressed: Box<dyn Fn()>,
    button: Cell<Option<cairo::RectangleInt>>,
    // reads the window's events while it is mapped
    events: RefCell<Option<glib::SourceId>>,
}

impl X11 {
    fn new(
        name: &str,
        size: (i32, i32),
        paint: Box<Paint>,
        pressed: Box<dyn Fn()>,
    ) -> anyhow::Result<Rc<Self>> {
        let (connection, screen) = x11rb::connect(None)?;
        let screen = connection.setup().roots[screen].clone();
        let (width, height) = (u16::try_from(size.0)?, u16::try_from(size.1)?);
        let window = connection.generate_id()?;
        let gc = connection.generate_id()?;
        let mut attributes = CreateWindowAux::new()
            .override_redirect(1)
            .background_pixel(0)
            .border_pixel(0)
            .event_mask(EventMask::EXPOSURE | EventMask::BUTTON_PRESS);
        // with an alpha channel, a compositor blends the rounded corners
        let alpha = screen
            .allowed_depths
            .iter()
            .filter(|depth| depth.depth == 32)
            .flat_map(|depth| &depth.visuals)
            .find(|visual| visual.class == VisualClass::TRUE_COLOR);
        let (depth, visual) = match alpha {
            Some(visual) => {
                let colormap = connection.generate_id()?;
                connection.create_colormap(
                    ColormapAlloc::NONE,
                    colormap,
                    screen.root,
                    visual.visual_id,
                )?;
                attributes = attributes.colormap(colormap);
                (32, visual.visual_id)
            }
            None => (screen.root_depth, screen.root_visual),
        };
        connection
            .create_window(
                depth,
                window,
                screen.root,
                (i32::from(screen.width_in_pixels) - size.0).max(0) as i16 / 2,
                (i32::from(screen.height_in_pixels) - size.1 - BOTTOM_MARGIN).max(0) as i16,
                width,
                height,
                0,
                WindowClass::INPUT_OUTPUT,
                visual,
                &attributes,
            )?
            .check()?;
        connection.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            name.as_bytes(),
        )?;
        // without a compositor nothing blends, so the corners are cut
        connection
            .shape_rectangles(
                shape::SO::SET,
                shape::SK::BOUNDING,
                ClipOrdering::UNSORTED,
                window,
                0,
                0,
                &rounded(width, height, RADIUS),
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
            .create_gc(gc, window, &CreateGCAux::new())?
            .check()?;
        connection.flush()?;
        Ok(Rc::new(Self {
            connection,
            window,
            gc,
            depth,
            size: (width, height),
            paint,
            pressed,
            button: Cell::new(None),
            events: RefCell::new(None),
        }))
    }

    fn show(self: &Rc<Self>, button: Option<cairo::RectangleInt>) -> anyhow::Result<()> {
        let pressable: Vec<Rectangle> = button
            .iter()
            .map(|button| Rectangle {
                x: button.x() as i16,
                y: button.y() as i16,
                width: button.width() as u16,
                height: button.height() as u16,
            })
            .collect();
        self.connection.shape_rectangles(
            shape::SO::SET,
            shape::SK::INPUT,
            ClipOrdering::UNSORTED,
            self.window,
            0,
            0,
            &pressable,
        )?;
        self.button.set(button);
        if self.events.borrow().is_none() {
            // a window created at startup sits below every window opened since
            self.connection.configure_window(
                self.window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            )?;
            self.connection.map_window(self.window)?;
            let this = Rc::downgrade(self);
            let events = glib::timeout_add_local(X11_EVENTS, move || {
                if let Some(this) = this.upgrade() {
                    this.read_events();
                }
                glib::ControlFlow::Continue
            });
            self.events.replace(Some(events));
        }
        self.draw()
    }

    fn draw(&self) -> anyhow::Result<()> {
        let (width, height) = self.size;
        let mut surface =
            cairo::ImageSurface::create(cairo::Format::ARgb32, width.into(), height.into())?;
        (self.paint)(&cairo::Context::new(&surface)?)?;
        self.connection.put_image(
            ImageFormat::Z_PIXMAP,
            self.window,
            self.gc,
            width,
            height,
            0,
            0,
            0,
            self.depth,
            &surface.data()?,
        )?;
        self.connection.flush()?;
        Ok(())
    }

    fn hide(&self) -> anyhow::Result<()> {
        if let Some(events) = self.events.take() {
            events.remove();
            self.connection.unmap_window(self.window)?;
            self.connection.flush()?;
        }
        Ok(())
    }

    fn read_events(&self) {
        loop {
            match self.connection.poll_for_event() {
                Ok(Some(Event::Expose(expose))) if expose.count == 0 => self.log(self.draw()),
                Ok(Some(Event::ButtonPress(press)))
                    if press.detail == 1
                        && self.button.get().is_some_and(|button| {
                            contains(button, press.event_x.into(), press.event_y.into())
                        }) =>
                {
                    (self.pressed)()
                }
                Ok(Some(Event::Error(error))) => {
                    tracing::debug!(?error, "floating panel request")
                }
                Ok(Some(_)) => {}
                Ok(None) => return,
                Err(error) => {
                    tracing::debug!(%error, "reading floating panel events");
                    return;
                }
            }
        }
    }

    fn log(&self, result: anyhow::Result<()>) {
        if let Err(error) = result {
            tracing::debug!(%error, "drawing a floating panel");
        }
    }
}

impl Drop for X11 {
    fn drop(&mut self) {
        if let Some(events) = self.events.take() {
            events.remove();
        }
    }
}

/// One rectangle per row of a `width` by `height` window whose corners are
/// rounded by `radius`. Every pixel the rounded background touches is inside.
fn rounded(width: u16, height: u16, radius: f64) -> Vec<Rectangle> {
    (0..height)
        .map(|row| {
            let from_edge = f64::from(row.min(height - 1 - row)) + 0.5;
            let rise = (radius - from_edge).max(0.0);
            let inset = (radius - (radius * radius - rise * rise).sqrt()).floor() as u16;
            Rectangle {
                x: inset as i16,
                y: row as i16,
                width: width - 2 * inset,
                height: 1,
            }
        })
        .collect()
}
