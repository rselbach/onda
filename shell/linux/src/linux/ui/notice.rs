//! The model update notice (DL-14): a modal dialog over the main window.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::glib;
use onda_app::views::UpdateNotice;
use onda_domain::migration::{NoticeChoice, NoticeShortcut};

/// The notice dialog while it is open. An answer and a close from the core
/// both empty it, so the first one wins and the user's choice is sent once.
#[derive(Default)]
pub struct Notice {
    open: Rc<RefCell<Option<adw::AlertDialog>>>,
}

impl Notice {
    /// Shows `view` over `parent` and passes the user's answer to `choose`.
    pub fn show(
        &self,
        parent: &impl IsA<gtk::Widget>,
        view: &UpdateNotice,
        choose: impl Fn(NoticeChoice) + 'static,
    ) {
        self.close();
        let dialog = adw::AlertDialog::builder()
            .heading(&view.title)
            .body(markup(&view.body))
            .body_use_markup(true)
            .build();
        for (index, button) in view.buttons.iter().enumerate() {
            let id = index.to_string();
            dialog.add_response(&id, &button.title);
            match button.shortcut {
                Some(NoticeShortcut::Return) => {
                    dialog.set_default_response(Some(&id));
                    dialog.set_response_appearance(&id, adw::ResponseAppearance::Suggested);
                }
                Some(NoticeShortcut::Escape) => dialog.set_close_response(&id),
                None => {}
            }
        }
        let choices: Vec<NoticeChoice> = view.buttons.iter().map(|button| button.choice).collect();
        let open = Rc::downgrade(&self.open);
        dialog.connect_response(None, move |dialog, response| {
            let Some(open) = open.upgrade() else {
                return;
            };
            // the core closed this dialog, which also emits a response
            if open.borrow().as_ref() != Some(dialog) {
                return;
            }
            open.take();
            // closing the dialog counts as Not Now (DL-14)
            let choice = response
                .parse::<usize>()
                .ok()
                .and_then(|index| choices.get(index).copied())
                .unwrap_or(NoticeChoice::NotNow);
            choose(choice);
        });
        dialog.present(Some(parent));
        self.open.replace(Some(dialog));
    }

    /// Closes the dialog without an answer.
    pub fn close(&self) {
        if let Some(dialog) = self.open.take() {
            dialog.force_close();
        }
    }
}

/// The body as Pango markup: `**text**` is bold and the rest is escaped. An
/// unpaired `**` stays as written.
fn markup(body: &str) -> String {
    let mut parts = body.split("**").map(glib::markup_escape_text);
    let mut markup = parts.next().map(String::from).unwrap_or_default();
    while let Some(bold) = parts.next() {
        match parts.next() {
            Some(plain) => markup += &format!("<b>{bold}</b>{plain}"),
            None => markup += &format!("**{bold}"),
        }
    }
    markup
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::BTreeMap, path::PathBuf, rc::Rc};

    use adw::prelude::*;
    use gtk::pango;
    use onda_app::{Event, views::update_notice};
    use onda_domain::{
        migration::{Migration, NoticeChoice},
        models::{Catalog, ModelId},
    };

    use super::super::{
        Window,
        integration_tests::{NOTICE_KEY, Start, button, capture, launch, wait_for, widgets},
    };
    use super::{Notice, markup};
    use crate::linux::Message;

    #[test]
    fn markup_bolds_the_model_name_and_escapes_the_rest() {
        let cases = BTreeMap::from([
            (
                "the core's copy",
                (
                    "we've switched you to **Parakeet TDT v2**.",
                    "we&apos;ve switched you to <b>Parakeet TDT v2</b>.",
                ),
            ),
            ("no Markdown", ("Onda was updated", "Onda was updated")),
            (
                "markup characters in plain and bold text",
                (
                    "<i>Troy</i> & **<b>Abed</b> & Annie**",
                    "&lt;i&gt;Troy&lt;/i&gt; &amp; <b>&lt;b&gt;Abed&lt;/b&gt; &amp; Annie</b>",
                ),
            ),
            (
                "two bold runs",
                ("**a** and **b**", "<b>a</b> and <b>b</b>"),
            ),
            ("an unpaired marker", ("a ** b", "a ** b")),
            (
                "an unpaired marker after a pair",
                ("**a** b ** c", "<b>a</b> b ** c"),
            ),
        ]);
        for (name, (body, want)) in cases {
            assert_eq!(markup(body), want, "{name}");
        }
    }

    fn quit(owner: &Window) {
        owner.core.runtime.block_on(owner.core.app.shutdown_async());
        owner.window.close();
    }

    fn open_dialog(window: &adw::ApplicationWindow) -> adw::AlertDialog {
        wait_for(|| window.visible_dialog().is_some());
        window.visible_dialog().unwrap().downcast().unwrap()
    }

    fn button_titles(dialog: &adw::AlertDialog) -> Vec<String> {
        widgets::<gtk::Button>(dialog)
            .iter()
            .filter_map(|button| button.label())
            .map(String::from)
            .collect()
    }

    // the runs of text that the dialog draws in bold
    fn bold_text(dialog: &adw::AlertDialog) -> Vec<String> {
        widgets::<gtk::Label>(dialog)
            .iter()
            .filter(|label| label.uses_markup())
            .flat_map(|label| {
                let text = label.text();
                let attributes = label.layout().attributes().unwrap_or_default();
                attributes
                    .attributes()
                    .iter()
                    .filter(|attribute| {
                        attribute
                            .downcast_ref::<pango::AttrInt>()
                            .is_some_and(|weight| {
                                attribute.type_() == pango::AttrType::Weight
                                    && weight.value() >= 700
                            })
                    })
                    .map(|attribute| {
                        text[attribute.start_index() as usize..attribute.end_index() as usize]
                            .to_owned()
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn stored(key: &str) -> Option<String> {
        let path = PathBuf::from(std::env::var_os("ONDA_DATA_DIR").unwrap()).join("settings.json");
        let settings: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        settings.get(key)?.as_str().map(str::to_owned)
    }

    #[test]
    #[ignore = "requires an isolated GTK display, D-Bus session, and ONDA_DATA_DIR"]
    fn real_gui_closing_the_update_notice_declines_the_download() {
        let owner = launch(Start::NoticeDue);
        assert!(!owner.window.is_visible());
        let dialog = open_dialog(&owner.window);
        assert!(
            owner.window.is_visible(),
            "the notice must bring the main window forward"
        );
        assert_eq!(dialog.heading().as_deref(), Some("Onda was updated"));
        assert_eq!(
            button_titles(&dialog),
            ["Not Now", "Choose Another", "Download"]
        );
        assert_eq!(bold_text(&dialog), ["Parakeet Ultra"]);
        capture(&owner, "update-notice");
        dialog.close();
        wait_for(|| stored("declinedModelMigrationNoticeKey").as_deref() == Some(NOTICE_KEY));
        wait_for(|| owner.window.visible_dialog().is_none());
        assert_eq!(stored("selectedModel").as_deref(), Some("parakeet-ja"));
        quit(&owner);
    }

    #[test]
    #[ignore = "requires an isolated GTK display, D-Bus session, and ONDA_DATA_DIR"]
    fn real_gui_choose_another_opens_the_speech_page() {
        let owner = launch(Start::NoticeDue);
        let dialog = open_dialog(&owner.window);
        assert_eq!(
            owner.stack.visible_child_name().as_deref(),
            Some("dictation")
        );
        button(&dialog, "Choose Another").emit_clicked();
        wait_for(|| owner.stack.visible_child_name().as_deref() == Some("speech"));
        wait_for(|| owner.window.visible_dialog().is_none());
        wait_for(|| stored("lastModelMigrationNoticeKey").as_deref() == Some(NOTICE_KEY));
        assert_eq!(
            stored("declinedModelMigrationNoticeKey").filter(|key| !key.is_empty()),
            None
        );
        assert_eq!(stored("selectedModel").as_deref(), Some("parakeet-ja"));
        quit(&owner);
    }

    #[test]
    #[ignore = "requires an isolated GTK display, D-Bus session, and ONDA_DATA_DIR"]
    fn real_gui_shows_a_critical_alert_as_a_modal_dialog() {
        let owner = launch(Start::Declined);
        owner.present();
        owner.window.close();
        assert!(!owner.window.is_visible());
        owner.event(Message::Core(Event::ShowAlert {
            title: "Greendale is closed".into(),
            message: "Dean Pelton cancelled every class.".into(),
            button: "OK",
        }));
        assert!(
            owner.window.is_visible(),
            "an alert must bring the main window forward"
        );
        let dialog = open_dialog(&owner.window);
        assert_eq!(dialog.heading().as_deref(), Some("Greendale is closed"));
        assert_eq!(dialog.body(), "Dean Pelton cancelled every class.");
        assert_eq!(button_titles(&dialog), ["OK"]);
        capture(&owner, "critical-alert");
        button(&dialog, "OK").emit_clicked();
        wait_for(|| owner.window.visible_dialog().is_none());
        quit(&owner);
    }

    #[test]
    #[ignore = "requires a GTK display"]
    fn real_gui_update_notice_sends_each_answer_once() {
        struct Case {
            answer: fn(&Notice, &adw::AlertDialog),
            want: &'static [NoticeChoice],
        }
        let cases = BTreeMap::from([
            (
                "the Download button",
                Case {
                    answer: |_, dialog| button(dialog, "Download").emit_clicked(),
                    want: &[NoticeChoice::Download],
                },
            ),
            (
                "the Choose Another button",
                Case {
                    answer: |_, dialog| button(dialog, "Choose Another").emit_clicked(),
                    want: &[NoticeChoice::ChooseAnother],
                },
            ),
            (
                "the Not Now button",
                Case {
                    answer: |_, dialog| button(dialog, "Not Now").emit_clicked(),
                    want: &[NoticeChoice::NotNow],
                },
            ),
            (
                "Return",
                Case {
                    answer: |_, dialog| dialog.activate_default(),
                    want: &[NoticeChoice::Download],
                },
            ),
            (
                "Escape or closing the dialog",
                Case {
                    answer: |_, dialog| assert!(dialog.close()),
                    want: &[NoticeChoice::NotNow],
                },
            ),
            (
                "a close from the core",
                Case {
                    answer: |notice, _| notice.close(),
                    want: &[],
                },
            ),
        ]);
        adw::init().unwrap();
        let window = adw::ApplicationWindow::builder().build();
        window.present();
        let view = update_notice(
            &Catalog::gguf(),
            &Migration {
                from: "parakeet-ja".into(),
                to: ModelId::ParakeetUltra,
            },
        );
        for (name, tc) in cases {
            let notice = Notice::default();
            let got = Rc::new(RefCell::new(Vec::new()));
            let sent = got.clone();
            notice.show(&window, &view, move |choice| sent.borrow_mut().push(choice));
            let dialog = open_dialog(&window);
            // keys reach the dialog once it holds the focus
            wait_for(|| dialog.focus().is_some());
            (tc.answer)(&notice, &dialog);
            // a button activated from the keyboard clicks after a delay
            wait_for(|| got.borrow().len() == tc.want.len());
            // the core closes the notice after every answer
            notice.close();
            wait_for(|| window.visible_dialog().is_none());
            assert_eq!(*got.borrow(), tc.want, "{name}");
        }
    }
}
