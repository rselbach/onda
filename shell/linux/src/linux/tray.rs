//! Optional StatusNotifierItem. The launcher remains usable without a tray.

use super::{Message, send};
use ksni::blocking::TrayMethods;

pub struct Tray {
    events: async_channel::Sender<Message>,
    pub label: String,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        super::APP_ID.into()
    }
    fn title(&self) -> String {
        self.label.clone()
    }
    fn icon_name(&self) -> String {
        "audio-input-microphone-symbolic".into()
    }
    fn activate(&mut self, _: i32, _: i32) {
        send(&self.events, Message::Action("show"));
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        [
            ("Open Onda", "show"),
            ("Start / Stop Dictation", "toggle"),
            ("Cancel Dictation", "cancel"),
            ("Paste Last Transcript", "paste"),
            ("Quit", "quit"),
        ]
        .into_iter()
        .map(|(label, action)| {
            ksni::menu::StandardItem {
                label: label.into(),
                activate: Box::new(move |tray: &mut Self| {
                    send(&tray.events, Message::Action(action))
                }),
                ..Default::default()
            }
            .into()
        })
        .collect()
    }
}

pub fn start(events: async_channel::Sender<Message>) -> Option<ksni::blocking::Handle<Tray>> {
    match (Tray {
        events,
        label: "Onda".into(),
    })
    .spawn()
    {
        Ok(handle) => Some(handle),
        Err(error) => {
            tracing::debug!(%error, "tray unavailable");
            None
        }
    }
}
