//! Cairo and Pango drawing for the floating panels: the recording overlay's
//! states, and the parts any panel is built from.

use std::f64::consts::{FRAC_PI_2, PI};

use gtk::{cairo, pango};
use onda_domain::{
    level::{BAR_SPACING, BAR_WIDTH, bar_heights},
    overlay::{OverlayContent, OverlayView},
};

/// Size of the recording overlay in pixels, as on macOS.
pub const SIZE: (i32, i32) = (400, 104);

/// Corner radius of a panel's background.
pub const RADIUS: f64 = 12.0;

/// Side of the target application's icon in pixels, as on macOS.
pub const ICON: f64 = 20.0;

#[derive(Clone, Copy)]
pub struct Color(f64, f64, f64, f64);

pub const PRIMARY: Color = Color(1.0, 1.0, 1.0, 0.95);
pub const SECONDARY: Color = Color(1.0, 1.0, 1.0, 0.62);
const PANEL: Color = Color(0.13, 0.13, 0.14, 1.0);
const BORDER: Color = Color(1.0, 1.0, 1.0, 0.12);
const ACCENT: Color = Color(0.384, 0.627, 0.918, 1.0);
const RED: Color = Color(1.0, 0.482, 0.388, 1.0);
const ORANGE: Color = Color(1.0, 0.639, 0.282, 1.0);

impl Color {
    fn set(self, cr: &cairo::Context) {
        cr.set_source_rgba(self.0, self.1, self.2, self.3);
    }
}

#[derive(Clone, Copy)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Area {
    pub fn inset(self, horizontal: f64, vertical: f64) -> Self {
        Self {
            x: self.x + horizontal,
            y: self.y + vertical,
            width: (self.width - 2.0 * horizontal).max(0.0),
            height: (self.height - 2.0 * vertical).max(0.0),
        }
    }
}

fn rounded(cr: &cairo::Context, area: Area, radius: f64) {
    let (right, bottom) = (area.x + area.width, area.y + area.height);
    cr.new_sub_path();
    cr.arc(right - radius, area.y + radius, radius, -FRAC_PI_2, 0.0);
    cr.arc(right - radius, bottom - radius, radius, 0.0, FRAC_PI_2);
    cr.arc(area.x + radius, bottom - radius, radius, FRAC_PI_2, PI);
    cr.arc(
        area.x + radius,
        area.y + radius,
        radius,
        PI,
        3.0 * FRAC_PI_2,
    );
    cr.close_path();
}

/// Paints a panel's background and clips what follows to it.
pub fn background(cr: &cairo::Context, width: f64, height: f64) -> Result<(), cairo::Error> {
    let panel = Area {
        x: 0.0,
        y: 0.0,
        width,
        height,
    };
    rounded(cr, panel, RADIUS);
    PANEL.set(cr);
    cr.fill_preserve()?;
    cr.clip();
    rounded(cr, panel.inset(0.5, 0.5), RADIUS - 0.5);
    BORDER.set(cr);
    cr.set_line_width(1.0);
    cr.stroke()
}

/// Lays `text` out at `size` pixels in at most `lines` lines no wider than
/// `width`, with an ellipsis where it is cut. Pango picks a font for each
/// script.
pub fn text(
    cr: &cairo::Context,
    text: &str,
    size: f64,
    weight: pango::Weight,
    width: f64,
    lines: i32,
) -> pango::Layout {
    let layout = pangocairo::functions::create_layout(cr);
    let mut font = pango::FontDescription::from_string("Sans");
    font.set_absolute_size(size * f64::from(pango::SCALE));
    font.set_weight(weight);
    layout.set_font_description(Some(&font));
    layout.set_width((width * f64::from(pango::SCALE)) as i32);
    layout.set_wrap(pango::WrapMode::WordChar);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    layout.set_height(-lines);
    // Pango counts the limit for each paragraph, so the text is made one
    layout.set_text(&text.replace(['\n', '\r'], " "));
    layout
}

pub fn show(cr: &cairo::Context, layout: &pango::Layout, x: f64, y: f64, color: Color) {
    color.set(cr);
    cr.move_to(x, y);
    pangocairo::functions::show_layout(cr, layout);
}

/// The core's waveform (OVL-8): capsule bars on the middle line of `area`,
/// newest on the right. `sweep`, from 0 to 1, moves a highlight across them.
pub fn bars(
    cr: &cairo::Context,
    levels: &[f32],
    area: Area,
    sweep: Option<f64>,
) -> Result<(), cairo::Error> {
    let heights = bar_heights(levels, area.width, area.height);
    let span = heights.len() as f64 * (BAR_WIDTH + BAR_SPACING) - BAR_SPACING;
    let mut x = area.x + (area.width - span) / 2.0;
    cr.new_path();
    for height in heights {
        let bar = Area {
            x,
            y: area.y + (area.height - height) / 2.0,
            width: BAR_WIDTH,
            height,
        };
        rounded(cr, bar, BAR_WIDTH / 2.0);
        x += BAR_WIDTH + BAR_SPACING;
    }
    ACCENT.set(cr);
    let Some(sweep) = sweep else {
        return cr.fill();
    };
    cr.fill_preserve()?;
    cr.save()?;
    cr.clip();
    let band = area.width * 0.35;
    let start = area.x - band + sweep * (area.width + band);
    let highlight = cairo::LinearGradient::new(start, 0.0, start + band, 0.0);
    highlight.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.0);
    highlight.add_color_stop_rgba(0.5, 1.0, 1.0, 1.0, 0.9);
    highlight.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    cr.set_source(&highlight)?;
    cr.paint()?;
    cr.restore()
}

/// A round button with a cross in the middle of `area`, the region a panel
/// takes presses on.
pub fn close(cr: &cairo::Context, area: cairo::RectangleInt) -> Result<(), cairo::Error> {
    let x = f64::from(area.x()) + f64::from(area.width()) / 2.0;
    let y = f64::from(area.y()) + f64::from(area.height()) / 2.0;
    cr.new_path();
    cr.arc(x, y, 10.0, 0.0, 2.0 * PI);
    SECONDARY.set(cr);
    cr.fill()?;
    cr.move_to(x - 3.5, y - 3.5);
    cr.line_to(x + 3.5, y + 3.5);
    cr.move_to(x + 3.5, y - 3.5);
    cr.line_to(x - 3.5, y + 3.5);
    PANEL.set(cr);
    cr.set_line_width(1.8);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.stroke()
}

fn warning(cr: &cairo::Context, x: f64, y: f64) -> Result<(), cairo::Error> {
    cr.new_path();
    cr.move_to(x, y - 7.5);
    cr.line_to(x + 8.0, y + 6.5);
    cr.line_to(x - 8.0, y + 6.5);
    cr.close_path();
    RED.set(cr);
    cr.set_line_width(2.5);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.fill_preserve()?;
    cr.stroke()?;
    cr.move_to(x, y - 3.0);
    cr.line_to(x, y + 1.5);
    cr.move_to(x, y + 4.5);
    cr.line_to(x, y + 4.6);
    PANEL.set(cr);
    cr.set_line_width(2.0);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.stroke()
}

fn muted_microphone(cr: &cairo::Context, x: f64, y: f64) -> Result<(), cairo::Error> {
    cr.new_path();
    let capsule = Area {
        x: x - 3.0,
        y: y - 8.5,
        width: 6.0,
        height: 11.0,
    };
    rounded(cr, capsule, 3.0);
    ORANGE.set(cr);
    cr.fill()?;
    cr.arc(x, y - 1.0, 6.0, 0.0, PI);
    cr.move_to(x, y + 5.0);
    cr.line_to(x, y + 8.0);
    cr.move_to(x - 3.0, y + 8.0);
    cr.line_to(x + 3.0, y + 8.0);
    cr.set_line_width(1.5);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.stroke()?;
    // the slash sits in a gap cut through the microphone
    for (color, width) in [(PANEL, 4.5), (ORANGE, 1.8)] {
        cr.move_to(x - 7.0, y - 8.0);
        cr.line_to(x + 7.0, y + 8.0);
        color.set(cr);
        cr.set_line_width(width);
        cr.stroke()?;
    }
    Ok(())
}

type Symbol = fn(&cairo::Context, f64, f64) -> Result<(), cairo::Error>;

fn message(
    cr: &cairo::Context,
    area: Area,
    symbol: Symbol,
    title: &str,
    body: &str,
    lines: i32,
) -> Result<(), cairo::Error> {
    let area = area.inset(18.0, 12.0);
    symbol(cr, area.x + 9.0, area.y + area.height / 2.0)?;
    let left = area.x + 28.0;
    let width = area.x + area.width - left;
    let title = text(cr, title, 14.0, pango::Weight::Semibold, width, 1);
    let body = text(cr, body, 12.0, pango::Weight::Normal, width, lines);
    let (title_height, body_height) = (title.pixel_size().1, body.pixel_size().1);
    let top = area.y + (area.height - f64::from(title_height + 2 + body_height)) / 2.0;
    show(cr, &title, left, top, PRIMARY);
    show(
        cr,
        &body,
        left,
        top + f64::from(title_height + 2),
        SECONDARY,
    );
    Ok(())
}

/// Where the recording overlay takes presses: its cancel button, shown with
/// the trailing controls.
pub fn cancel_area(view: &OverlayView) -> Option<cairo::RectangleInt> {
    view.trailing
        .as_ref()
        .map(|_| cairo::RectangleInt::new(SIZE.0 - 36, (SIZE.1 - 28) / 2, 28, 28))
}

/// Draws the recording overlay as the macOS shell lays it out (OVL-5 to
/// OVL-10). `icon` is the target application's, a square picture of any
/// size. `sweep`, from 0 to 1, animates the processing state.
pub fn recording(
    cr: &cairo::Context,
    view: &OverlayView,
    icon: Option<&cairo::ImageSurface>,
    sweep: Option<f64>,
) -> Result<(), cairo::Error> {
    let (width, height) = (f64::from(SIZE.0), f64::from(SIZE.1));
    background(cr, width, height)?;
    let mut content = Area {
        x: 0.0,
        y: 0.0,
        width,
        height,
    };
    if let (Some(trailing), Some(cancel)) = (&view.trailing, cancel_area(view)) {
        close(cr, cancel)?;
        let mut left = f64::from(cancel.x()) + 4.0;
        if let Some(clock) = &trailing.clock {
            let layout = text(cr, &clock.text, 11.0, pango::Weight::Normal, 160.0, 1);
            // digits of one width keep the clock, and the waveform beside
            // it, from shifting every second
            let digits = pango::AttrList::new();
            digits.insert(pango::AttrFontFeatures::new("tnum"));
            layout.set_attributes(Some(&digits));
            let (clock_width, clock_height) = layout.pixel_size();
            left -= 8.0 + f64::from(clock_width);
            let color = if clock.warning { ORANGE } else { SECONDARY };
            let top = (height - f64::from(clock_height)) / 2.0;
            show(cr, &layout, left, top, color);
        }
        content.width = left - 10.0;
    }
    if let Some(icon) = icon {
        let square = Area {
            x: 10.0,
            y: (height - ICON) / 2.0,
            width: ICON,
            height: ICON,
        };
        cr.save()?;
        rounded(cr, square, 4.0);
        cr.clip();
        cr.translate(square.x, square.y);
        let scale = ICON / f64::from(icon.width().max(1));
        cr.scale(scale, scale);
        cr.set_source_surface(icon, 0.0, 0.0)?;
        cr.paint()?;
        cr.restore()?;
        let taken = square.x + ICON + 10.0;
        content.x = taken;
        content.width -= taken;
    }
    match &view.content {
        OverlayContent::Notice(notice) => {
            message(cr, content, warning, &notice.title, &notice.message, 3)
        }
        OverlayContent::SilentMicrophone {
            title,
            message: body,
        } => message(cr, content, muted_microphone, title, body, 2),
        OverlayContent::Processing { label, preview } => {
            let area = content.inset(18.0, 12.0);
            let label = text(cr, label, 12.0, pango::Weight::Normal, area.width, 1);
            label.set_alignment(pango::Alignment::Center);
            let preview = preview
                .as_ref()
                .map(|preview| text(cr, preview, 11.0, pango::Weight::Normal, area.width, 2));
            let label_height = f64::from(label.pixel_size().1);
            let preview_height = preview
                .as_ref()
                .map_or(0.0, |preview| 8.0 + f64::from(preview.pixel_size().1));
            let total = 16.0 + 8.0 + label_height + preview_height;
            let top = area.y + (area.height - total) / 2.0;
            let dots = Area {
                y: top,
                height: 16.0,
                ..area
            };
            bars(cr, &[], dots, sweep)?;
            show(cr, &label, area.x, top + 24.0, SECONDARY);
            if let Some(preview) = &preview {
                let below = top + 24.0 + label_height + 8.0;
                show(cr, preview, area.x, below, SECONDARY);
            }
            Ok(())
        }
        OverlayContent::Preview {
            text: words,
            levels,
        } => {
            let area = content.inset(18.0, 12.0);
            let words = text(cr, words, 13.0, pango::Weight::Normal, area.width, 2);
            let words_height = f64::from(words.pixel_size().1);
            let top = area.y + (area.height - (words_height + 8.0 + 24.0)) / 2.0;
            show(cr, &words, area.x, top, PRIMARY);
            let meter = Area {
                y: top + words_height + 8.0,
                height: 24.0,
                ..area
            };
            bars(cr, levels, meter, None)
        }
        OverlayContent::Waveform { levels } => bars(cr, levels, content.inset(20.0, 15.0), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onda_domain::overlay::{ClockDisplay, Notice, Overlay};

    const JAPANESE: &str = "Troy Barnes が Greendale で話す";

    type Pixel = (u8, u8, u8);

    fn render(view: &OverlayView, sweep: Option<f64>) -> Vec<u8> {
        render_with(view, None, sweep)
    }

    fn render_with(
        view: &OverlayView,
        icon: Option<&cairo::ImageSurface>,
        sweep: Option<f64>,
    ) -> Vec<u8> {
        let mut surface =
            cairo::ImageSurface::create(cairo::Format::ARgb32, SIZE.0, SIZE.1).unwrap();
        recording(&cairo::Context::new(&surface).unwrap(), view, icon, sweep).unwrap();
        surface.data().unwrap().to_vec()
    }

    fn count(pixels: &[u8], columns: std::ops::Range<i32>, matches: fn(Pixel) -> bool) -> usize {
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .filter(|(index, _)| columns.contains(&(*index as i32 % SIZE.0)))
            .filter(|(_, pixel)| matches((pixel[2], pixel[1], pixel[0])))
            .count()
    }

    fn accent((red, green, blue): Pixel) -> bool {
        red < 130 && (130..190).contains(&green) && blue > 200
    }

    fn orange((red, green, blue): Pixel) -> bool {
        red > 230 && (140..190).contains(&green) && blue < 110
    }

    fn red((red, green, blue): Pixel) -> bool {
        red > 230 && (100..140).contains(&green) && (80..120).contains(&blue)
    }

    fn light((red, green, blue): Pixel) -> bool {
        red > 140 && green > 140 && blue > 140
    }

    // a test of drawn text proves nothing where no font is installed
    fn fonts() -> bool {
        use pango::prelude::*;
        let installed = !pangocairo::FontMap::default().list_families().is_empty();
        if !installed {
            eprintln!("skipped: no fonts are installed");
        }
        installed
    }

    fn recording_overlay() -> Overlay {
        let mut overlay = Overlay::new();
        overlay.show_recording(None, Some("Abed's Podcast Mic".to_owned()));
        overlay
    }

    fn clock(warning: bool) -> ClockDisplay {
        ClockDisplay {
            text: "0:07".to_owned(),
            warning,
        }
    }

    fn states() -> Vec<(&'static str, OverlayView)> {
        let mut overlay = recording_overlay();
        overlay.push_level(0.05);
        let waveform = overlay.view(Some(&clock(false)));
        overlay.set_preview("Troy Barnes studies at Greendale");
        let preview = overlay.view(Some(&clock(false)));
        overlay.set_silent(true);
        let silent = overlay.view(Some(&clock(false)));
        overlay.enter_transcribing(None);
        let processing = overlay.view(None);
        overlay.show_notice(
            Notice {
                title: "Paste Failed".to_owned(),
                message: "The transcript is on the clipboard.".to_owned(),
            },
            std::time::Duration::ZERO,
        );
        let notice = overlay.view(None);
        vec![
            ("waveform", waveform),
            ("preview", preview),
            ("silent microphone", silent),
            ("processing", processing),
            ("notice", notice),
        ]
    }

    #[test]
    fn each_state_has_its_own_picture() {
        let pictures: Vec<_> = states()
            .into_iter()
            .map(|(name, view)| (name, render(&view, None)))
            .collect();
        for (index, (name, picture)) in pictures.iter().enumerate() {
            for (other, earlier) in &pictures[..index] {
                assert!(picture != earlier, "{name} looks like {other}");
            }
            let want_red = *name == "notice";
            assert_eq!(count(picture, 0..SIZE.0, red) > 0, want_red, "{name}");
            let want_orange = *name == "silent microphone";
            assert_eq!(count(picture, 0..SIZE.0, orange) > 0, want_orange, "{name}");
        }
    }

    #[test]
    fn application_icon_stands_before_the_content() {
        fn green((red, green, blue): Pixel) -> bool {
            red < 20 && green > 230 && blue < 20
        }
        let icon = cairo::ImageSurface::create(cairo::Format::ARgb32, 40, 40).unwrap();
        let cr = cairo::Context::new(&icon).unwrap();
        cr.set_source_rgb(0.0, 1.0, 0.0);
        cr.paint().unwrap();
        let view = recording_overlay().view(None);
        let without = render(&view, None);
        let with = render_with(&view, Some(&icon), None);

        let leading = 0..40;
        assert_eq!(count(&without, 0..SIZE.0, green), 0);
        assert!(count(&without, leading.clone(), accent) > 0);
        let drawn = count(&with, 10..30, green);
        assert!(drawn > 300, "a 40 pixel picture fills the 20 pixel square");
        assert_eq!(count(&with, 0..SIZE.0, green), drawn);
        assert_eq!(count(&with, leading, accent), 0, "the waveform moved over");
        assert!(count(&with, 40..SIZE.0, accent) > 0);
    }

    #[test]
    fn waveform_bars_follow_the_levels() {
        let mut overlay = recording_overlay();
        let silence = render(&overlay.view(None), None);
        for _ in 0..30 {
            overlay.push_level(0.0);
        }
        overlay.push_level(0.2);
        let speech = render(&overlay.view(None), None);

        let dots = count(&silence, 0..SIZE.0, accent);
        assert!(dots > 0, "silence idles as a row of dots");
        let (older, newest) = (
            count(&speech, 0..SIZE.0 / 2, accent),
            count(&speech, SIZE.0 / 2..SIZE.0, accent),
        );
        assert_eq!(older, count(&silence, 0..SIZE.0 / 2, accent));
        assert!(newest > older + 100, "the newest level stands on the right");
    }

    #[test]
    fn processing_highlight_moves_across_the_dots() {
        let (_, processing) = states().remove(3);
        let still = render(&processing, None);
        let early = render(&processing, Some(0.3));
        let late = render(&processing, Some(0.7));
        assert!(early != still && early != late);
    }

    #[test]
    fn cancel_button_comes_with_the_trailing_controls() {
        for (name, view) in states() {
            let picture = render(&view, None);
            let button = SIZE.0 - 36..SIZE.0 - 8;
            let drawn = count(&picture, button, light) > 0;
            assert_eq!(drawn, view.trailing.is_some(), "{name}");
            assert_eq!(cancel_area(&view).is_some(), drawn, "{name}");
        }
    }

    #[test]
    fn clock_is_orange_when_it_warns() {
        if !fonts() {
            return;
        }
        let overlay = recording_overlay();
        let calm = render(&overlay.view(Some(&clock(false))), None);
        let warning = render(&overlay.view(Some(&clock(true))), None);
        assert_eq!(count(&calm, 0..SIZE.0, orange), 0);
        assert!(count(&warning, 0..SIZE.0, orange) > 0);
        let without = render(&overlay.view(None), None);
        assert!(count(&calm, 0..SIZE.0, light) > count(&without, 0..SIZE.0, light));
    }

    #[test]
    fn text_in_any_script_is_drawn() {
        if !fonts() {
            return;
        }
        let mut overlay = recording_overlay();
        overlay.set_preview(JAPANESE);
        let whole = render(&overlay.view(None), None);
        let ascii: String = JAPANESE
            .chars()
            .map(|c| if c.is_ascii() { c } else { ' ' })
            .collect();
        overlay.set_preview(ascii);
        let blanked = render(&overlay.view(None), None);
        assert!(count(&whole, 0..SIZE.0, light) > count(&blanked, 0..SIZE.0, light) + 40);
    }

    #[test]
    fn notice_shows_a_title_and_three_lines_of_message() {
        if !fonts() {
            return;
        }
        let message = "Abed asked the study group to wait in the library until the \
            Dreamatorium is clean, and then Troy Barnes started a recording that \
            Greendale could not save.";
        let notice = |message: &str| {
            let mut overlay = Overlay::new();
            overlay.show_notice(
                Notice {
                    title: "Couldn't Save Recording".to_owned(),
                    message: message.to_owned(),
                },
                std::time::Duration::ZERO,
            );
            overlay.view(None)
        };
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, SIZE.0, SIZE.1).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        let body = text(&cr, message, 12.0, pango::Weight::Normal, 336.0, 3);
        assert_eq!(body.line_count(), 3, "the message fills three lines");
        assert!(!body.is_ellipsized());
        let broken = "Troy\nAbed\nAnnie\nBritta\nShirley";
        let capped = text(&cr, broken, 12.0, pango::Weight::Normal, 336.0, 2);
        assert!(
            capped.line_count() <= 2,
            "line breaks do not lift the limit"
        );

        let whole = render(&notice(message), None);
        let cut = render(&notice(message.trim_end_matches("could not save.")), None);
        assert!(whole != cut, "the end of the third line is drawn");
        let title_only = render(&notice(""), None);
        assert!(count(&whole, 0..SIZE.0, light) > count(&title_only, 0..SIZE.0, light));
    }
}
