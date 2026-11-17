//! StatusNotifierItem tray icon (KDE, and GNOME with the AppIndicator
//! extension). Invariant 10: closing the window hides it and the app keeps
//! running, so the tray is how the window comes back.

use crate::bridge::{request_lock, request_quit, request_show};
use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use ksni::MenuItem;
use std::sync::OnceLock;

struct MaineTray;

impl ksni::Tray for MaineTray {
    fn id(&self) -> String {
        "maine".into()
    }

    fn title(&self) -> String {
        "Maine".into()
    }

    fn icon_name(&self) -> String {
        // Our own icon when installed (scripts/install.sh), else a stock one.
        let installed = std::env::var_os("HOME")
            .map(|h| {
                std::path::Path::new(&h)
                    .join(".local/share/icons/hicolor/scalable/apps/maine.svg")
                    .exists()
            })
            .unwrap_or(false)
            || std::path::Path::new("/usr/share/icons/hicolor/scalable/apps/maine.svg").exists();
        if installed {
            "maine"
        } else {
            "utilities-terminal"
        }
        .into()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        request_show();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Show Maine".into(),
                activate: Box::new(|_| request_show()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Lock vault".into(),
                activate: Box::new(|_| request_lock()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| request_quit()),
                ..Default::default()
            }
            .into(),
        ]
    }
}

static HANDLE: OnceLock<ksni::blocking::Handle<MaineTray>> = OnceLock::new();

/// Starts the tray icon once. Returns false when no StatusNotifier host is
/// available, in which case closing the window quits the app instead of hiding
/// it where nobody can bring it back.
pub fn start() -> bool {
    if HANDLE.get().is_some() {
        return true;
    }
    match MaineTray.spawn() {
        Ok(handle) => {
            let _ = HANDLE.set(handle);
            true
        }
        Err(_) => false,
    }
}
