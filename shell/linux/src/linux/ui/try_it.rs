//! The field a user dictates into to try Onda (WIN-6, ONB-9).

use adw::prelude::*;
use gtk::accessible;

/// A plain local text field: what lands in it goes nowhere. Its placeholder
/// shows while it is empty.
pub(super) struct TryIt {
    pub widget: gtk::Frame,
    pub text: gtk::TextView,
    placeholder: gtk::Label,
}

impl TryIt {
    pub fn new() -> Self {
        let text = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(10)
            .bottom_margin(10)
            .left_margin(12)
            .right_margin(12)
            .build();
        let placeholder = gtk::Label::builder()
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_top(10)
            .margin_start(12)
            .can_target(false)
            .css_classes(["dim-label"])
            .build();
        let shown = placeholder.clone();
        text.buffer()
            .connect_changed(move |buffer| shown.set_visible(buffer.char_count() == 0));
        let field = gtk::Overlay::builder()
            .child(&gtk::ScrolledWindow::builder().child(&text).build())
            .build();
        field.add_overlay(&placeholder);
        let widget = gtk::Frame::builder()
            .child(&field)
            .height_request(96)
            .build();
        Self {
            widget,
            text,
            placeholder,
        }
    }

    pub fn set_placeholder(&self, placeholder: &str) {
        self.placeholder.set_label(placeholder);
        self.text
            .update_property(&[accessible::Property::Label(placeholder)]);
    }
}
