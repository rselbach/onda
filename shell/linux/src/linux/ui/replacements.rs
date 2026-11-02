use super::{Window, clear, label, page};
use adw::prelude::*;
use onda_app::views::ReplacementsPage;
use std::{cell::Cell, rc::Rc};

pub(super) struct Page {
    pub widget: gtk::Box,
    rows: gtk::Box,
    add: gtk::Button,
    revision: Cell<Option<u64>>,
}

impl Page {
    pub fn new() -> Self {
        let widget = page();
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 16);
        widget.append(&rows);
        let add = gtk::Button::builder()
            .label("Add Rule")
            .halign(gtk::Align::Start)
            .build();
        widget.append(&add);
        let footer = label(&onda_app::views::replacements_page(Vec::new(), 0).footer);
        footer.set_xalign(0.0);
        footer.add_css_class("dim-label");
        widget.append(&footer);
        Self {
            widget,
            rows,
            add,
            revision: Cell::new(None),
        }
    }

    pub fn connect(&self, owner: &Rc<Window>) {
        let weak = Rc::downgrade(owner);
        self.add.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.add_replacement_rule();
            }
        });
    }

    pub fn update(&self, owner: &Rc<Window>, view: ReplacementsPage) {
        if self.revision.replace(Some(view.revision)) == Some(view.revision) {
            return;
        }
        clear(&self.rows);
        if let Some(caption) = view.empty_caption {
            let group = adw::PreferencesGroup::builder().title(&view.title).build();
            group.add(
                &adw::ActionRow::builder()
                    .title("No rules yet")
                    .subtitle(&caption)
                    .build(),
            );
            self.rows.append(&group);
        }
        for (index, rule) in view.rows.into_iter().enumerate() {
            let group = adw::PreferencesGroup::builder()
                .title(format!("Rule {}", index + 1))
                .build();
            let delete = gtk::Button::from_icon_name("user-trash-symbolic");
            delete.add_css_class("flat");
            delete.set_tooltip_text(Some(&view.delete_title));
            let weak = Rc::downgrade(owner);
            let id = rule.id.clone();
            delete.connect_clicked(move |_| {
                if let Some(owner) = weak.upgrade() {
                    owner.core.app.delete_replacement_rule(&id);
                }
            });
            group.set_header_suffix(Some(&delete));
            let triggers = adw::EntryRow::builder()
                .title(&view.replace_label)
                .text(&rule.triggers)
                .build();
            triggers.set_tooltip_text(Some(&view.triggers_placeholder));
            let weak = Rc::downgrade(owner);
            let id = rule.id.clone();
            triggers.connect_changed(move |row| {
                if let Some(owner) = weak.upgrade() {
                    owner
                        .core
                        .app
                        .set_replacement_triggers(&id, row.text().as_str());
                }
            });
            group.add(&triggers);
            let replacement = adw::EntryRow::builder()
                .title(&view.with_label)
                .text(&rule.replacement)
                .build();
            replacement.set_tooltip_text(Some(&view.replacement_placeholder));
            let weak = Rc::downgrade(owner);
            replacement.connect_changed(move |row| {
                if let Some(owner) = weak.upgrade() {
                    owner
                        .core
                        .app
                        .set_replacement_text(&rule.id, row.text().as_str());
                }
            });
            group.add(&replacement);
            self.rows.append(&group);
        }
    }
}
