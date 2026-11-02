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
        let bytes = gtk::glib::Bytes::from_static(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/AppIcon.appiconset/icon_256x256.png"
        )));
        if let Ok(texture) = gtk::gdk::Texture::from_bytes(&bytes) {
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
        let linux = [
            ("gtk4-rs", "MIT", "https://github.com/gtk-rs/gtk4-rs"),
            (
                "libadwaita-rs",
                "MIT",
                "https://gitlab.gnome.org/World/Rust/libadwaita-rs",
            ),
            (
                "transcribe.cpp",
                "MIT",
                "https://github.com/handy-computer/transcribe.cpp",
            ),
            (
                "Hex desktop integration",
                "MIT",
                "https://github.com/anomalyco/hex",
            ),
        ];
        for (name, license, url) in linux.into_iter().chain(
            page.libraries
                .iter()
                .filter(|library| {
                    !matches!(
                        library.name.as_str(),
                        "FluidAudio"
                            | "keyring"
                            | "mediaremote-adapter"
                            | "objc2"
                            | "security-framework"
                            | "tracing-oslog"
                            | "UniFFI"
                    )
                })
                .map(|library| {
                    (
                        library.name.as_str(),
                        library.license.as_str(),
                        library.url.as_str(),
                    )
                }),
        ) {
            let row = adw::ActionRow::builder()
                .title(name)
                .subtitle(license)
                .use_markup(false)
                .build();
            let link = gtk::LinkButton::with_label(url, "Website");
            link.set_valign(gtk::Align::Center);
            row.add_suffix(&link);
            group.add(&row);
        }
        self.about.append(&group);
        let group = adw::PreferencesGroup::builder().title("Updates").build();
        group.add(
            &adw::ActionRow::builder()
                .title("Installed locally")
                .subtitle("Update Onda by installing a newer Linux build.")
                .build(),
        );
        self.about.append(&group);
    }
}
