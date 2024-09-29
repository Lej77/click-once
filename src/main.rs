// no need to allocate a console for a long-running program that does not output anything
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(not(feature = "std"), no_main)]

#[cfg(all(test, not(feature = "std")))]
core::compile_error!("cargo test is only supported with \"std\" feature");

mod core_runtime {
    //! Reimplement argument parsing and panic handling for `no_std` target.
    #![cfg(not(any(feature = "std", test)))]

    use core::{panic, slice, str};
    use windows_sys::Win32::System::Environment::GetCommandLineA;
    use windows_sys::Win32::System::Threading::ExitProcess;

    // Need to link to some libraries to get required symbols like memcpy:
    // https://users.rust-lang.org/t/unresolved-external-symbol-s-when-trying-to-link-a-no-std-binary-to-a-windows-dll/54306/3
    // https://users.rust-lang.org/t/unresolved-external-symbol-memcpy-memset-memmove-memcmp-strlen-cxxframehandler-cxxthrowexception/58546
    // https://learn.microsoft.com/en-us/cpp/c-runtime-library/crt-library-features?view=msvc-160

    // linkage to CRT library according to crt-static flag set in .cargo/config:
    // [target.x86_64-pc-windows-msvc]
    // rustflags = ["-C", "target-feature=+crt-static"]
    #[cfg(target_feature = "crt-static")]
    #[link(name = "libcmt")]
    extern "C" {}
    #[cfg(target_feature = "crt-static")]
    #[link(name = "libucrt")]
    extern "C" {}

    #[cfg(not(target_feature = "crt-static"))]
    #[link(name = "msvcrt")]
    extern "C" {}
    #[cfg(not(target_feature = "crt-static"))]
    #[link(name = "ucrt")]
    extern "C" {}

    #[link(name = "libvcruntime")]
    extern "C" {}

    // Wine's impl: https://github.com/wine-mirror/wine/blob/7ec5f555b05152dda53b149d5994152115e2c623/dlls/shell32/shell32_main.c#L58
    #[inline(always)]
    pub fn args() -> impl Iterator<Item = &'static str> {
        unsafe {
            const SPACE: u8 = b' ';
            const TAB: u8 = b'\t';
            const QUOTE: u8 = b'"';
            const NULL: u8 = b'\0';

            let mut pcmdline = GetCommandLineA();
            if *pcmdline == QUOTE {
                pcmdline = pcmdline.add(1);
                while *pcmdline != NULL {
                    if *pcmdline == QUOTE {
                        break;
                    }
                    pcmdline = pcmdline.add(1);
                }
            } else {
                while *pcmdline != NULL && *pcmdline != SPACE && *pcmdline != TAB {
                    pcmdline = pcmdline.add(1);
                }
            }
            pcmdline = pcmdline.add(1);
            while *pcmdline == SPACE || *pcmdline == TAB {
                pcmdline = pcmdline.add(1);
            }
            let pcmdline_s = pcmdline;
            while *pcmdline != NULL {
                pcmdline = pcmdline.add(1);
            }

            slice::from_raw_parts(pcmdline_s, pcmdline.offset_from(pcmdline_s) as usize)
                .split(|p| p == &SPACE)
                .map(|v| str::from_utf8(v).unwrap_or_else(|_| ExitProcess(1)))
        }
    }

    #[no_mangle]
    fn _start() {
        crate::program_start();
    }

    #[panic_handler]
    fn panic(_info: &panic::PanicInfo) -> ! {
        unsafe { ExitProcess(1) }
    }
}

use core::sync::atomic::{AtomicU32, Ordering::Relaxed};
use core::*;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN,
    WM_RBUTTONUP,
};

static THRESHOLD_LM: AtomicU32 = AtomicU32::new(0);
static THRESHOLD_RM: AtomicU32 = AtomicU32::new(0);

const WM_LBUTTONDOWNU: usize = WM_LBUTTONDOWN as _;
const WM_LBUTTONUPU: usize = WM_LBUTTONUP as _;
const WM_RBUTTONDOWNU: usize = WM_RBUTTONDOWN as _;
const WM_RBUTTONUPU: usize = WM_RBUTTONUP as _;

unsafe extern "system" fn low_level_mouse_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    static LAST_DOWN_L: AtomicU32 = AtomicU32::new(0);
    static LAST_UP_L: AtomicU32 = AtomicU32::new(0);
    static LAST_DOWN_R: AtomicU32 = AtomicU32::new(0);
    static LAST_UP_R: AtomicU32 = AtomicU32::new(0);

    if code >= 0 {
        match wparam {
            WM_LBUTTONDOWNU => {
                let tick = GetTickCount();
                if !(tick - LAST_DOWN_L.load(Relaxed) >= THRESHOLD_LM.load(Relaxed)
                    && tick - LAST_UP_L.load(Relaxed) >= THRESHOLD_LM.load(Relaxed))
                {
                    return 1;
                } else {
                    LAST_DOWN_L.store(tick, Relaxed);
                }
            }
            WM_LBUTTONUPU => {
                let tick = GetTickCount();
                if !(tick - LAST_UP_L.load(Relaxed) >= THRESHOLD_LM.load(Relaxed)) {
                    return 1;
                } else {
                    LAST_UP_L.store(tick, Relaxed);
                }
            }
            WM_RBUTTONDOWNU => {
                let tick = GetTickCount();
                if !(tick - LAST_DOWN_R.load(Relaxed) >= THRESHOLD_RM.load(Relaxed)
                    && tick - LAST_UP_R.load(Relaxed) >= THRESHOLD_RM.load(Relaxed))
                {
                    return 1;
                } else {
                    LAST_DOWN_R.store(tick, Relaxed);
                }
            }
            WM_RBUTTONUPU => {
                let tick = GetTickCount();
                if !(tick - LAST_UP_R.load(Relaxed) >= THRESHOLD_RM.load(Relaxed)) {
                    return 1;
                } else {
                    LAST_UP_R.store(tick, Relaxed);
                }
            }
            _ => (),
        }
    }

    CallNextHookEx(ptr::null_mut(), code, wparam, lparam)
}

fn parse_and_save_args() {
    #[cfg(not(feature = "std"))]
    let args = core_runtime::args();
    #[cfg(feature = "std")]
    let args = std::env::args();

    let mut args = args.filter_map(|arg| arg.parse::<u32>().ok());

    if let Some(arg_lm) = args.next() {
        THRESHOLD_LM.store(arg_lm, Relaxed);
    }
    if let Some(arg_rm) = args.next() {
        THRESHOLD_RM.store(arg_rm, Relaxed);
    }
}

fn program_start() {
    parse_and_save_args();

    unsafe {
        SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), ptr::null_mut(), 0);
    }

    #[cfg(feature = "tray")]
    {
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
            event_loop::{ActiveEventLoop, EventLoop},
            window::WindowId,
        };
        let h_instance = unsafe { GetModuleHandleW(ptr::null()) };

        #[derive(Debug)]
        enum UserEvent {
            Quit,
        }

        let tray_menu = Menu::new();
        let quit_item = MenuItem::new("Quit", true, Some(Accelerator::new(None, Code::KeyQ)));
        tray_menu.append(&quit_item).unwrap();

        let mut tray = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_tooltip("click-once");

        // https://learn.microsoft.com/en-us/windows/deployment/usmt/usmt-recognized-environment-variables
        if let Ok(win_dir) = std::env::var("WINDIR") {
            pub fn to_utf16(s: &str) -> Vec<u16> {
                use std::ffi::OsStr;
                use std::os::windows::ffi::OsStrExt;

                OsStr::new(s)
                    .encode_wide()
                    .chain(core::iter::once(0u16))
                    .collect()
            }
            let icon_path = win_dir + "\\System32\\main.cpl";
            let icon_path = to_utf16(&icon_path);
            let icon_handle = unsafe { ExtractIconW(h_instance, icon_path.as_ptr(), 0) };
            if icon_handle.is_null() {
                #[cfg(debug_assertions)]
                {
                    eprintln!("Failed to extract icon");
                }
            } else {
                tray = tray.with_icon(tray_icon::Icon::from_handle(icon_handle as isize));
            }
        }
        let tray = tray.build().unwrap();
        let event_loop = EventLoop::with_user_event().build().unwrap();

        MenuEvent::set_event_handler(Some({
            let quit_id = quit_item.id().clone();
            let proxy = event_loop.create_proxy();
            move |event: MenuEvent| {
                // Note: this actually runs on the same thread as the main event
                // loop so don't block.

                if event.id == quit_id {
                    proxy.send_event(UserEvent::Quit).unwrap_or_else(|_| {
                        std::process::exit(1);
                    });
                }
            }
        }));

        struct App {
            tray: TrayIcon,
        }
        impl ApplicationHandler<UserEvent> for App {
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
                        let _ = self.tray.set_visible(false);
                        event_loop.exit();
                    }
                }
            }
        }

        event_loop.run_app(&mut App { tray }).unwrap();
    }

    // Simples event loop replacement:
    #[cfg(not(feature = "tray"))]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetMessageW;

        GetMessageW(&mut mem::zeroed(), ptr::null_mut(), 0, 0);
    }
}

#[cfg(feature = "std")]
fn main() {
    program_start();
}
