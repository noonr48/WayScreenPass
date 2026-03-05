//! System tray implementation using ksni

use ksni::{Tray, menu::StandardItem, menu::SubMenu, MenuItem};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::Sender;

use crate::config::HostConfig;

#[derive(Debug, Clone)]
pub enum TrayMessage {
    Connect { index: usize },
    AddHost,
    RemoveHost { index: usize },
    Exit,
}

pub struct RemoteDesktopTray {
    pub hosts: Arc<Mutex<Vec<HostConfig>>>,
    pub tx: Sender<TrayMessage>,
    pub client_binary: String,
}

impl Tray for RemoteDesktopTray {
    fn id(&self) -> String { "remote-desktop-tray".into() }
    fn icon_name(&self) -> String { "computer".into() }
    fn title(&self) -> String { "Remote Desktop".into() }
    fn status(&self) -> ksni::Status { ksni::Status::Active }
    fn icon_theme_path(&self) -> String { String::new() }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let hosts = self.hosts.lock().unwrap();
        let mut items: Vec<MenuItem<Self>> = Vec::new();

        if hosts.is_empty() {
            items.push(StandardItem {
                label: "No saved hosts".into(),
                enabled: false,
                ..Default::default()
            }.into());
        } else {
            for (i, host) in hosts.iter().enumerate() {
                let tx = self.tx.clone();
                items.push(StandardItem {
                    label: format!("{} ({})", host.name, host.hostname),
                    icon_name: "network-server".into(),
                    activate: Box::new(move |_| { let _ = tx.send(TrayMessage::Connect { index: i }); }),
                    ..Default::default()
                }.into());
            }
        }

        items.push(MenuItem::Separator);

        let tx = self.tx.clone();
        items.push(StandardItem {
            label: "Add Host...".into(),
            icon_name: "list-add".into(),
            activate: Box::new(move |_| { let _ = tx.send(TrayMessage::AddHost); }),
            ..Default::default()
        }.into());

        if !hosts.is_empty() {
            let submenu: Vec<MenuItem<Self>> = hosts.iter().enumerate().map(|(i, host)| {
                let tx = self.tx.clone();
                StandardItem {
                    label: host.name.clone(),
                    icon_name: "list-remove".into(),
                    activate: Box::new(move |_| { let _ = tx.send(TrayMessage::RemoveHost { index: i }); }),
                    ..Default::default()
                }.into()
            }).collect();
            items.push(SubMenu {
                label: "Remove Host".into(),
                icon_name: "edit-delete".into(),
                submenu: submenu,
                ..Default::default()
            }.into());
        }

        items.push(MenuItem::Separator);

        let tx = self.tx.clone();
        items.push(StandardItem {
            label: "Quit".into(),
            icon_name: "application-exit".into(),
            activate: Box::new(move |_| { let _ = tx.send(TrayMessage::Exit); }),
            ..Default::default()
        }.into());

        items
    }
}

impl RemoteDesktopTray {
    pub fn new(hosts: Arc<Mutex<Vec<HostConfig>>>, tx: Sender<TrayMessage>, client_binary: String) -> Self {
        Self { hosts, tx, client_binary }
    }
}
