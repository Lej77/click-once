#[cfg(feature = "logging")]
use {
    crate::{log, logging},
    tray_icon::menu::CheckMenuItem,
    windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK},
};

use crate::log_error;
use core::sync::atomic::Ordering::Relaxed;
use tray_icon::{
    menu::{
        accelerator::{Accelerator, Code},
        Menu, MenuEvent, MenuItem,
    },
    TrayIcon, TrayIconBuilder,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::ExtractIconW;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    window::WindowId,
};

fn to_utf16(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    OsStr::new(s)
        .encode_wide()
        .chain(core::iter::once(0u16))
        .collect()
}

#[derive(Debug)]
pub enum UserEvent {
    Quit,
    #[cfg(feature = "logging")]
    ToggleLogging,
    #[cfg(feature = "logging")]
    ShowStats,
}

pub struct TrayApp {
    tray: TrayIcon,
    #[cfg(feature = "logging")]
    logging_item: CheckMenuItem,
}
impl TrayApp {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let h_instance = unsafe { GetModuleHandleW(core::ptr::null()) };

        let tray_menu = Menu::new();
        let quit_item = MenuItem::new("&Quit", true, Some(Accelerator::new(None, Code::KeyQ)));
        #[cfg(feature = "logging")]
        let logging_item = CheckMenuItem::new(
            "Toggle &Logging",
            true,
            logging::is_logging(),
            Some(Accelerator::new(None, Code::KeyL)),
        );
        #[cfg(feature = "logging")]
        let show_stats: MenuItem = MenuItem::new(
            "View &Statistics",
            true,
            Some(Accelerator::new(None, Code::KeyS)),
        );

        tray_menu
            .append_items(&[
                #[cfg(feature = "logging")]
                &show_stats,
                #[cfg(feature = "logging")]
                &logging_item,
                &quit_item,
            ])
            .expect("Failed to add context menu items");

        let mut tray = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            // Note: there is a max length for the tooltip, more will be truncated
            .with_tooltip({
                use std::fmt::Write;

                let mut tooltip = "click-once".to_owned();
                {
                    tooltip.push_str("\r\nLeft: ");
                    let threshold_left = crate::THRESHOLD_LM.load(Relaxed);
                    if threshold_left == 0 {
                        tooltip.push_str("Disabled");
                    } else {
                        write!(tooltip, "{} ms", threshold_left).unwrap();
                    }
                }
                {
                    tooltip.push_str("\r\nRight: ");
                    let threshold_right = crate::THRESHOLD_RM.load(Relaxed);
                    if threshold_right == 0 {
                        tooltip.push_str("Disabled");
                    } else {
                        write!(tooltip, "{} ms", threshold_right).unwrap();
                    }
                }
                {
                    tooltip.push_str("\r\nMiddle: ");
                    let threshold_middle = crate::THRESHOLD_MM.load(Relaxed);
                    if threshold_middle == 0 {
                        tooltip.push_str("Disabled");
                    } else {
                        write!(tooltip, "{} ms", threshold_middle).unwrap();
                    }
                }
                tooltip
            });

        // https://learn.microsoft.com/en-us/windows/deployment/usmt/usmt-recognized-environment-variables
        match std::env::var("WINDIR") {
            Ok(win_dir) => {
                let icon_path = win_dir + "\\System32\\main.cpl";
                let icon_path = to_utf16(&icon_path);
                let icon_handle = unsafe { ExtractIconW(h_instance, icon_path.as_ptr(), 0) };
                if icon_handle.is_null() {
                    log_error("Failed to extract icon");
                } else {
                    tray = tray.with_icon(tray_icon::Icon::from_handle(icon_handle as isize));
                }
            }
            Err(e) => log_error(format_args!(
                "Failed to get WINDIR environment variable to locate Windows folder: {e}"
            )),
        }
        let tray = tray.build().unwrap();

        MenuEvent::set_event_handler(Some({
            let quit_id = quit_item.id().clone();
            #[cfg(feature = "logging")]
            let logging_id = logging_item.id().clone();
            #[cfg(feature = "logging")]
            let show_stats_id = show_stats.id().clone();
            move |event: MenuEvent| {
                // Note: this actually runs on the same thread as the main event
                // loop so don't block.

                if event.id == quit_id {
                    proxy.send_event(UserEvent::Quit).unwrap_or_else(|_| {
                        std::process::exit(1);
                    });
                }
                #[cfg(feature = "logging")]
                if event.id == logging_id {
                    _ = proxy.send_event(UserEvent::ToggleLogging);
                }
                #[cfg(feature = "logging")]
                if event.id == show_stats_id {
                    _ = proxy.send_event(UserEvent::ShowStats);
                }
            }
        }));

        TrayApp {
            tray,
            #[cfg(feature = "logging")]
            logging_item,
        }
    }
}
impl ApplicationHandler<UserEvent> for TrayApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        _event: WindowEvent,
    ) {
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Quit => {
                // On Windows 10 we need to hide the tray icon when
                // exiting, otherwise it will remain until it is hovered
                // on or otherwise interacted with:
                if let Err(e) = self.tray.set_visible(false) {
                    log_error(e);
                }
                event_loop.exit();
            }
            #[cfg(feature = "logging")]
            UserEvent::ToggleLogging => {
                let enable = !logging::is_logging();
                logging::set_should_log(enable);
                self.logging_item.set_checked(enable);
                log![
                    b"\r\nLogging for click-once!\r\n\r\n\
                    Warning: closing this console window will terminate the program!\r\n\r\n"
                ];
                logging::log_program_config()
                    .iter()
                    .for_each(|value| value.write());
                logging::stats::log_current_stats(&mut |v| v.write());
            }
            #[cfg(feature = "logging")]
            UserEvent::ShowStats => {
                let title = to_utf16("Statistics for click-once");
                let mut text = String::new();
                {
                    logging::log_program_config()
                        .iter()
                        .for_each(|value| value.write_to_string(&mut text));
                    logging::stats::log_current_stats(&mut |v| v.write_to_string(&mut text));
                }
                let text = to_utf16(&text);
                // https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-messageboxw
                let result = unsafe {
                    MessageBoxW(core::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK)
                };
                if result == 0 {
                    log_error("Failed to open message box");
                }
            }
        }
    }
}

pub fn run_event_loop_with_tray() {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().unwrap();
    let mut app = TrayApp::new(event_loop.create_proxy());
    event_loop.run_app(&mut app).unwrap();
}
