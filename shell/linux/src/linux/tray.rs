//! Optional StatusNotifierItem. The launcher remains usable without a tray.

use super::{Message, send};
use ksni::blocking::TrayMethods;

pub struct Tray {
    events: async_channel::Sender<Message>,
    pub label: String,
    pub update: onda_domain::updates::UpdateControl,
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
        let mut items: Vec<_> = [
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
        .collect();
        let action = self.update.action;
        items.insert(
            4,
            ksni::menu::StandardItem {
                label: self.update.title.clone(),
                enabled: self.update.enabled,
                activate: Box::new(move |tray: &mut Self| {
                    send(&tray.events, Message::Update(action))
                }),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

pub fn start(events: async_channel::Sender<Message>) -> Option<ksni::blocking::Handle<Tray>> {
    match (Tray {
        events,
        label: "Onda".into(),
        update: onda_domain::updates::update_control(
            &onda_domain::updates::UpdateStatus::Idle,
            false,
        ),
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
