use adw::prelude::*;
use onda_app::views::MainWindowSection;

pub(super) fn name(section: MainWindowSection) -> &'static str {
    use MainWindowSection::*;
    match section {
        GettingStarted => "dictation",
        General => "general",
        Hotkey => "hotkey",
        Speech => "speech",
        PostProcessing => "post-processing",
        Replacements => "replacements",
        History => "history",
        About => "about",
    }
}

pub(super) fn section(name_: &str) -> MainWindowSection {
    MainWindowSection::ALL
        .into_iter()
        .find(|section| name(*section) == name_)
        .unwrap_or(MainWindowSection::GettingStarted)
}

pub(super) fn sidebar(stack: &gtk::Stack, title: &adw::WindowTitle) -> gtk::ScrolledWindow {
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    list.set_size_request(210, -1);
    let mut rows = Vec::new();
    for group in onda_app::views::sidebar() {
        let label = gtk::Label::builder()
            .label(&group.title)
            .xalign(0.0)
            .margin_top(20)
            .margin_bottom(6)
            .margin_start(12)
            .build();
        label.add_css_class("dim-label");
        label.add_css_class("caption-heading");
        let heading = gtk::ListBoxRow::builder()
            .child(&label)
            .selectable(false)
            .activatable(false)
            .build();
        list.append(&heading);
        for page in group.pages {
            use MainWindowSection::*;
            let icon = match page.section {
                GettingStarted => "go-home-symbolic",
                General => "preferences-system-symbolic",
                Hotkey => "preferences-desktop-keyboard-shortcuts-symbolic",
                Speech => "audio-input-microphone-symbolic",
                PostProcessing => "tools-check-spelling-symbolic",
                Replacements => "edit-find-replace-symbolic",
                History => "document-open-recent-symbolic",
                About => "help-about-symbolic",
            };
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            content.set_margin_top(6);
            content.set_margin_bottom(6);
            content.append(&gtk::Image::from_icon_name(icon));
            content.append(&gtk::Label::new(Some(&page.title)));
            let row = gtk::ListBoxRow::builder().child(&content).build();
            row.set_widget_name(name(page.section));
            list.append(&row);
            rows.push(row);
        }
    }
    list.select_row(rows.first());
    let stack_ = stack.downgrade();
    list.connect_row_selected(move |_, row| {
        if let Some(row) = row
            && let Some(stack) = stack_.upgrade()
        {
            stack.set_visible_child_name(&row.widget_name());
        }
    });
    let title = title.clone();
    let list_ = list.clone();
    stack.connect_visible_child_name_notify(move |stack| {
        if let Some(name) = stack.visible_child_name() {
            title.set_title(section(&name).title());
            if let Some(row) = rows.iter().find(|row| row.widget_name() == name) {
                list_.select_row(Some(row));
            }
        }
    });
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build()
}
