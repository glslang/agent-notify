use crate::icons::{IconSet, load_icon_set};
use crate::settings::BridgeConfig;
use crate::worker::{BridgeCommand, BridgeRuntimeState, IconSink, IconState, run_bridge_worker};
use anyhow::Context;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;
use tracing::error;
use tray_icon::{
    TrayIcon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
};

#[derive(Debug)]
enum TrayUserEvent {
    Menu(tray_icon::menu::MenuEvent),
    SetIcon(IconState),
}

struct TrayMenuLoop {
    command_tx: mpsc::UnboundedSender<BridgeCommand>,
    paused: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pause_item: std::rc::Rc<CheckMenuItem>,
    tray: TrayIcon,
    icons: IconSet,
    quit_id: tray_icon::menu::MenuId,
    test_id: tray_icon::menu::MenuId,
    dismiss_id: tray_icon::menu::MenuId,
    reconnect_id: tray_icon::menu::MenuId,
    pause_id: tray_icon::menu::MenuId,
}

impl winit::application::ApplicationHandler<TrayUserEvent> for TrayMenuLoop {
    fn resumed(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {}

    fn window_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        _event: winit::event::WindowEvent,
    ) {
    }

    fn user_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        event: TrayUserEvent,
    ) {
        let menu_event = match event {
            TrayUserEvent::Menu(menu_event) => menu_event,
            TrayUserEvent::SetIcon(state) => {
                if let Err(err) = self.tray.set_icon(Some(self.icons.get(state))) {
                    error!(?err, ?state, "failed to set tray icon");
                }
                return;
            }
        };
        let id = menu_event.id().clone();
        if id == self.quit_id {
            let _ = self.command_tx.send(BridgeCommand::Quit);
            event_loop.exit();
            return;
        }
        if id == self.test_id {
            let _ = self.command_tx.send(BridgeCommand::Test);
        } else if id == self.dismiss_id {
            let _ = self.command_tx.send(BridgeCommand::Dismiss);
        } else if id == self.reconnect_id {
            let _ = self.command_tx.send(BridgeCommand::Reconnect);
        } else if id == self.pause_id {
            let paused = self.pause_item.is_checked();
            self.paused.store(paused, Ordering::Relaxed);
            let _ = self.command_tx.send(BridgeCommand::SetPaused(paused));
        }
    }

    fn exiting(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        let _ = self.command_tx.send(BridgeCommand::Quit);
    }
}

pub fn run_windows_tray(config: BridgeConfig) -> anyhow::Result<()> {
    let event_loop = winit::event_loop::EventLoop::<TrayUserEvent>::with_user_event()
        .build()
        .context("failed to create winit event loop")?;

    let event_proxy = event_loop.create_proxy();
    tray_icon::menu::MenuEvent::set_event_handler(Some(move |event| {
        let _ = event_proxy.send_event(TrayUserEvent::Menu(event));
    }));

    let menu = Menu::new();
    let pause_item = CheckMenuItem::new("Pause notifications", true, false, None);
    let test_item = MenuItem::new("Send test notification", true, None);
    let dismiss_item = MenuItem::new("Dismiss notification", true, None);
    let reconnect_item = MenuItem::new("Reconnect", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    menu.append(&pause_item)?;
    menu.append(&test_item)?;
    menu.append(&dismiss_item)?;
    menu.append(&reconnect_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit_item)?;

    let icons = load_icon_set(crate::icons::detect_theme())?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Agent Notify")
        .with_icon(icons.get(IconState::Idle))
        .build()?;

    let pause_item = std::rc::Rc::new(pause_item);
    let pause_id = pause_item.id().clone();
    let quit_id = quit_item.id().clone();
    let test_id = test_item.id().clone();
    let dismiss_id = dismiss_item.id().clone();
    let reconnect_id = reconnect_item.id().clone();

    // The worker thread reports state changes here; forward them to the event
    // loop so the icon is swapped on the UI thread that owns the tray.
    let icon_proxy = event_loop.create_proxy();
    let icon_sink: IconSink = Box::new(move |state| {
        let _ = icon_proxy.send_event(TrayUserEvent::SetIcon(state));
    });

    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let state = BridgeRuntimeState::new();
    let worker_state = state.clone();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                error!(?err, "failed to create tokio runtime");
                return;
            }
        };
        if let Err(err) = runtime.block_on(run_bridge_worker(
            config,
            worker_state,
            command_rx,
            icon_sink,
        )) {
            error!(?err, "bridge worker stopped");
        }
    });

    let mut tray_loop = TrayMenuLoop {
        command_tx,
        paused: state.paused,
        pause_item,
        tray,
        icons,
        quit_id,
        test_id,
        dismiss_id,
        reconnect_id,
        pause_id,
    };

    let run_result = event_loop.run_app(&mut tray_loop);
    tray_icon::menu::MenuEvent::set_event_handler(Option::<fn(tray_icon::menu::MenuEvent)>::None);
    run_result.context("windows tray event loop failed")?;

    Ok(())
}
