use super::{Window, clear};
use adw::prelude::*;
use onda_app::views::AboutPage;

impl Window {
    pub(super) fn about(&self, page: AboutPage) {
        clear(&self.about);
        let group = adw::PreferencesGroup::builder()
            .description(&page.tagline)
            .build();
        let row = adw::ActionRow::builder()
            .title(&page.app_name)
            .subtitle(&page.version)
            .build();
        let icon = gtk::Image::from_icon_name("com.rselbach.onda");
        if let Some(texture) = super::app_icon() {
            icon.set_paintable(Some(&texture));
        }
        icon.set_pixel_size(48);
        row.add_prefix(&icon);
        group.add(&row);
        self.about.append(&group);
        let group = adw::PreferencesGroup::builder()
            .title(&page.models_title)
            .build();
        for model in page.models {
            let row = adw::ActionRow::builder()
                .title(&model.name)
                .subtitle(&model.attribution)
                .use_markup(false)
                .build();
            let link = gtk::LinkButton::with_label(&model.license_url, &model.license_name);
            link.set_valign(gtk::Align::Center);
            row.add_suffix(&link);
            group.add(&row);
        }
        self.about.append(&group);
        let group = adw::PreferencesGroup::builder()
            .title(&page.libraries_title)
            .build();
        for library in &page.libraries {
            let row = adw::ActionRow::builder()
                .title(&library.name)
                .subtitle(&library.license)
                .use_markup(false)
                .build();
            let link = gtk::LinkButton::with_label(&library.url, "Website");
            link.set_valign(gtk::Align::Center);
            row.add_suffix(&link);
            group.add(&row);
        }
        self.about.append(&group);
    }
}
