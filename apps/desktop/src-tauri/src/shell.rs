//! Desktop-shell behavior that must stay independent from the webview.
//!
//! The host owns capture, transport, and virtual-display resources, so closing
//! its only window must not accidentally tear down a running display session.
//! The process remains discoverable through a tray icon; an explicit Quit is
//! the only tray action that terminates it.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use tauri::{
    App, AppHandle, Manager, Runtime, Window, WindowEvent,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::runtime::DesktopRuntime;

const MAIN_WINDOW_LABEL: &str = "main";
const OPEN_MENU_ID: &str = "open";
const DISCONNECT_MENU_ID: &str = "disconnect";
const QUIT_MENU_ID: &str = "quit";
const TRAY_ID: &str = "ladoflow-host";

#[derive(Default)]
struct ShutdownState(AtomicBool);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    Open,
    Disconnect,
    Quit,
    Unknown,
}

impl TrayAction {
    fn from_id(id: &str) -> Self {
        match id {
            OPEN_MENU_ID => Self::Open,
            DISCONNECT_MENU_ID => Self::Disconnect,
            QUIT_MENU_ID => Self::Quit,
            _ => Self::Unknown,
        }
    }
}

pub fn setup<R: Runtime>(app: &mut App<R>) -> tauri::Result<()> {
    app.manage(ShutdownState::default());

    let open = MenuItem::with_id(app, OPEN_MENU_ID, "Open LadoFlow", true, None::<&str>)?;
    let disconnect = MenuItem::with_id(
        app,
        DISCONNECT_MENU_ID,
        "Disconnect display",
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, QUIT_MENU_ID, "Quit LadoFlow", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &disconnect, &separator, &quit_item])?;

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("LadoFlow local display host")
        .show_menu_on_left_click(false)
        .on_menu_event(
            |app, event| match TrayAction::from_id(event.id().as_ref()) {
                TrayAction::Open => show_main_window(app),
                TrayAction::Disconnect => disconnect_display(app),
                TrayAction::Quit => quit(app),
                TrayAction::Unknown => {}
            },
        )
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    let _tray = tray.build(app)?;
    Ok(())
}

pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    let _result = window.unminimize();
    let _result = window.show();
    let _result = window.set_focus();
}

pub fn handle_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW_LABEL {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        let _result = window.hide();
    }
}

fn disconnect_display<R: Runtime>(app: &AppHandle<R>) {
    let runtime = Arc::clone(app.state::<Arc<DesktopRuntime>>().inner());
    let spawn = thread::Builder::new()
        .name("ladoflow-tray-disconnect".to_owned())
        .spawn(move || {
            if let Err(error) = runtime.disconnect_android_usb() {
                eprintln!("LadoFlow tray disconnect failed: {error}");
            }
        });
    if let Err(error) = spawn {
        eprintln!("LadoFlow could not start the tray disconnect worker: {error}");
    }
}

fn quit<R: Runtime>(app: &AppHandle<R>) {
    let shutdown = app.state::<ShutdownState>();
    if shutdown.0.swap(true, Ordering::AcqRel) {
        return;
    }

    let runtime = Arc::clone(app.state::<Arc<DesktopRuntime>>().inner());
    let app = app.clone();
    let fallback = app.clone();
    let spawn = thread::Builder::new()
        .name("ladoflow-graceful-exit".to_owned())
        .spawn(move || {
            if let Err(error) = runtime.disconnect_android_usb() {
                eprintln!("LadoFlow shutdown could not disconnect the display: {error}");
            }
            #[cfg(target_os = "windows")]
            if let Err(error) = crate::platform::disable_virtual_display() {
                eprintln!("LadoFlow shutdown could not disable the virtual display: {error}");
            }
            app.exit(0);
        });
    if let Err(error) = spawn {
        eprintln!("LadoFlow could not start its graceful-exit worker: {error}");
        fallback.exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{TrayAction, TrayAction::Unknown};

    #[test]
    fn tray_menu_routes_only_known_action_ids() {
        assert_eq!(TrayAction::from_id("open"), TrayAction::Open);
        assert_eq!(TrayAction::from_id("disconnect"), TrayAction::Disconnect);
        assert_eq!(TrayAction::from_id("quit"), TrayAction::Quit);
        assert_eq!(TrayAction::from_id("open-untrusted"), Unknown);
    }
}
