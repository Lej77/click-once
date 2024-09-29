// no need to allocate a console for a long-running program that does not output anything
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(not(feature = "std"), no_main)]
// Lints:
#![warn(clippy::allow_attributes_without_reason)]

#[cfg(all(test, not(feature = "std")))]
core::compile_error!("cargo test is only supported with \"std\" feature");

#[cfg(not(any(feature = "std", test)))]
mod std_polyfill {
    //! Reimplement argument parsing and panic handling for `no_std` target.

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

    /// Wine's impl:
    /// <https://github.com/wine-mirror/wine/blob/7ec5f555b05152dda53b149d5994152115e2c623/dlls/shell32/shell32_main.c#L58>
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
                .filter(|p| !p.is_empty())
                .map(|v| str::from_utf8(v).unwrap_or_else(|_| ExitProcess(1)))
        }
    }

    #[inline(always)]
    pub fn exit(code: i32) -> ! {
        unsafe { ExitProcess(code as u32) }
    }

    #[no_mangle]
    fn _start() {
        crate::program_start();
    }

    #[panic_handler]
    fn panic(_info: &panic::PanicInfo) -> ! {
        exit(1)
    }
}

#[cfg(feature = "std")]
mod std_polyfill {
    //! Re-export std functions so that they have the same signatures and
    //! behavior as our `std_polyfill` has on `no_std`.

    pub use std::process::exit;

    /// Wrapper around [`std::env::args`] that skips the first argument (which
    /// would otherwise be the executable's path).
    pub fn args() -> impl Iterator<Item = String> {
        std::env::args().skip(1)
    }
}

#[cfg(feature = "logging")]
mod logging {
    //! Implements logging by writing to a console window, optionally creating
    //! such a window if it doesn't exist (which it only does in debug builds
    //! when `std` is enabled since we then use the default `console` subsystem).

    use crate::{log_error, FgColor};
    use core::sync::atomic::{AtomicBool, Ordering};
    use windows_sys::Win32::System::Console::{
        AllocConsole, FreeConsole, GetStdHandle, SetConsoleTextAttribute, WriteConsoleA,
        FOREGROUND_BLUE, FOREGROUND_GREEN, FOREGROUND_INTENSITY, FOREGROUND_RED, STD_OUTPUT_HANDLE,
    };

    static SHOULD_LOG: AtomicBool = AtomicBool::new(cfg!(all(debug_assertions, feature = "std")));

    pub fn is_logging() -> bool {
        SHOULD_LOG.load(Ordering::Acquire)
    }

    /// Create or destroy a console window.
    ///
    /// # References
    ///
    /// - <https://learn.microsoft.com/en-us/windows/console/allocconsole>
    pub fn set_should_log(enabled: bool) {
        if SHOULD_LOG
            .compare_exchange(!enabled, enabled, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            let result = if enabled {
                unsafe { AllocConsole() }
            } else {
                unsafe { FreeConsole() }
            };
            if result == 0 {
                log_error(format_args!(
                    "Failed to {} console",
                    if enabled { "create" } else { "destroy" }
                ));
            }
        }
    }

    #[derive(Clone, Copy)]
    pub enum LogValue<'a> {
        /// A number.
        Number(u32),
        /// ASCII text.
        Text(&'a [u8]),
        Color(FgColor),
    }
    impl<'a> LogValue<'a> {
        /// Write this value to the console.
        ///
        /// # References
        ///
        /// - <https://stackoverflow.com/questions/28890402/win32-console-write-c-c>
        /// - <https://learn.microsoft.com/en-us/windows/console/writeconsole>
        /// - <https://docs.rs/windows-sys/0.52.0/windows_sys/Win32/System/Console/fn.WriteConsoleA.html>
        pub fn write(self) {
            if !SHOULD_LOG.load(Ordering::Acquire) {
                return;
            }
            let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
            if handle.is_null() {
                log_error("Failed to get handle to console window");
            }

            let mut buffer = itoa::Buffer::new();
            let mut ascii = match self {
                LogValue::Number(number) => buffer.format(number).as_bytes(),
                LogValue::Text(ascii) => ascii,
                LogValue::Color(color) => {
                    let result =
                        unsafe { SetConsoleTextAttribute(handle, color.windows_text_attribute()) };
                    if result == 0 {
                        log_error("Failed to set text color");
                    }
                    return;
                }
            };
            while !ascii.is_empty() {
                let mut written: u32 = 0;
                let result = unsafe {
                    WriteConsoleA(
                        handle,
                        ascii.as_ptr(),
                        ascii.len() as u32,
                        &mut written,
                        core::ptr::null(),
                    )
                };
                if result == 0 {
                    log_error("WriteConsoleA failed");
                    return;
                }
                ascii = &ascii[written as usize..];
            }
        }
    }
    impl<'a> From<&'a [u8]> for LogValue<'a> {
        fn from(value: &'a [u8]) -> Self {
            LogValue::Text(value)
        }
    }
    impl<'a, const N: usize> From<&'a [u8; N]> for LogValue<'a> {
        fn from(value: &'a [u8; N]) -> Self {
            LogValue::Text(value)
        }
    }
    impl From<u32> for LogValue<'_> {
        fn from(value: u32) -> Self {
            LogValue::Number(value)
        }
    }
    impl From<FgColor> for LogValue<'_> {
        fn from(value: FgColor) -> Self {
            LogValue::Color(value)
        }
    }

    impl FgColor {
        const fn to_less_bright(self) -> Self {
            match self {
                FgColor::Reset
                | FgColor::Black
                | FgColor::Red
                | FgColor::Green
                | FgColor::Yellow
                | FgColor::Blue
                | FgColor::Magenta
                | FgColor::Cyan
                | FgColor::White => self,
                FgColor::BrightBlack => Self::Black,
                FgColor::BrightRed => Self::Red,
                FgColor::BrightGreen => Self::Green,
                FgColor::BrightYellow => Self::Yellow,
                FgColor::BrightBlue => Self::Blue,
                FgColor::BrightMagenta => Self::Magenta,
                FgColor::BrightCyan => Self::Cyan,
                FgColor::BrightWhite => Self::White,
            }
        }
        const fn windows_text_attribute(self) -> u16 {
            match self {
                FgColor::Reset => FOREGROUND_RED | FOREGROUND_GREEN | FOREGROUND_BLUE,
                FgColor::Black => 0,
                FgColor::Red => FOREGROUND_RED,
                FgColor::Green => FOREGROUND_GREEN,
                FgColor::Yellow => FOREGROUND_RED | FOREGROUND_GREEN,
                FgColor::Blue => FOREGROUND_BLUE,
                FgColor::Magenta => FOREGROUND_RED | FOREGROUND_BLUE,
                FgColor::Cyan => FOREGROUND_GREEN | FOREGROUND_BLUE,
                FgColor::White => FOREGROUND_RED | FOREGROUND_GREEN | FOREGROUND_BLUE,
                FgColor::BrightBlack
                | FgColor::BrightRed
                | FgColor::BrightGreen
                | FgColor::BrightYellow
                | FgColor::BrightBlue
                | FgColor::BrightMagenta
                | FgColor::BrightCyan
                | FgColor::BrightWhite => {
                    self.to_less_bright().windows_text_attribute() | FOREGROUND_INTENSITY
                }
            }
        }
        #[expect(
            dead_code,
            reason = "we use console text attributes to be more compatible with older Windows terminals"
        )]
        const fn ansi(self) -> &'static [u8] {
            match self {
                FgColor::Reset => b"\x1B[0m",
                FgColor::Black => b"\x1B[0;30m",
                FgColor::Red => b"\x1B[0;31m",
                FgColor::Green => b"\x1B[0;32m",
                FgColor::Yellow => b"\x1B[0;33m",
                FgColor::Blue => b"\x1B[0;34m",
                FgColor::Magenta => b"\x1B[0;35m",
                FgColor::Cyan => b"\x1B[0;36m",
                FgColor::White => b"\x1B[0;37m",
                FgColor::BrightBlack => b"\x1B[0m90m",
                FgColor::BrightRed => b"\x1B[0m91m",
                FgColor::BrightGreen => b"\x1B[0m92m",
                FgColor::BrightYellow => b"\x1B[0m93m",
                FgColor::BrightBlue => b"\x1B[0m94m",
                FgColor::BrightMagenta => b"\x1B[0m95m",
                FgColor::BrightCyan => b"\x1B[0m96m",
                FgColor::BrightWhite => b"\x1B[0m97m",
            }
        }
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

/// Logs values to console if the `logging` Cargo feature is enabled and a
/// console has been created (for example using the tray icon).
macro_rules! log {
    ($($value:expr),* $(,)?) => {
        #[cfg(feature = "logging")]
        {
            if $crate::logging::is_logging() {
                $(
                    $crate::logging::LogValue::from($value).write();
                )*
            }
        }
        #[cfg(not(feature = "logging"))]
        {
            $(
                _ = $value;
            )*
        }
    };
}

#[inline(always)]
fn log_error(_error: impl core::fmt::Display) {
    #[cfg(all(feature = "std", debug_assertions, not(feature = "logging")))]
    {
        eprintln!("Error: {_error}");
    }
    #[cfg(all(feature = "std", feature = "logging"))]
    {
        use std::io::Write;

        _ = write!(std::io::stdout(), "{}", _error);
    }
}

/// Terminal colors using ANSI escape codes or Windows console text attributes.
///
/// # References
///
/// - <https://en.wikipedia.org/wiki/ANSI_escape_code>
/// - <https://stackoverflow.com/questions/2348000/colors-in-c-win32-console>
/// - <https://stackoverflow.com/questions/43539956/how-to-stop-the-effect-of-ansi-text-color-code-or-set-text-color-back-to-default>
#[derive(Clone, Copy)]
#[expect(dead_code, reason = "we don't use all these colors yet")]
enum FgColor {
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}
impl FgColor {
    /// Color for log messages where a mouse click was blocked/ignored.
    const BLOCKED: Self = Self::BrightRed;
    /// Color for parts of log messages that includes time values.
    const TIME: Self = Self::BrightCyan;
}

static THRESHOLD_LM: AtomicU32 = AtomicU32::new(30);
static THRESHOLD_RM: AtomicU32 = AtomicU32::new(0);

const WM_LBUTTONDOWNU: usize = WM_LBUTTONDOWN as _;
const WM_LBUTTONUPU: usize = WM_LBUTTONUP as _;
const WM_RBUTTONDOWNU: usize = WM_RBUTTONDOWN as _;
const WM_RBUTTONUPU: usize = WM_RBUTTONUP as _;

#[cfg(feature = "logging")]
static BLOCKED_CLICKS: AtomicU32 = AtomicU32::new(0);

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
                let time_since_last_event =
                    tick.saturating_sub(LAST_DOWN_L.load(Relaxed).max(LAST_UP_L.load(Relaxed)));

                if time_since_last_event < THRESHOLD_LM.load(Relaxed) {
                    log![
                        FgColor::BLOCKED,
                        b"Left click ignored (too frequent, within ",
                        FgColor::TIME,
                        time_since_last_event,
                        b" ms",
                        FgColor::BLOCKED,
                        b")\r\n",
                        FgColor::Reset,
                    ];

                    #[cfg(feature = "logging")]
                    BLOCKED_CLICKS.fetch_add(1, Relaxed);

                    return 1;
                } else {
                    LAST_DOWN_L.store(tick, Relaxed);
                    log![
                        b"Left click accepted (after ",
                        FgColor::TIME,
                        time_since_last_event,
                        b" ms",
                        FgColor::Reset,
                        b")\r\n",
                    ];
                }
            }
            WM_LBUTTONUPU => {
                let tick = GetTickCount();
                let time_since_last_event = tick.saturating_sub(LAST_UP_L.load(Relaxed));

                if time_since_last_event < THRESHOLD_LM.load(Relaxed) {
                    log![
                        FgColor::BLOCKED,
                        b"\tLeft button up event ignored (too frequent, within ",
                        FgColor::TIME,
                        time_since_last_event,
                        b" ms",
                        FgColor::BLOCKED,
                        b")\r\n",
                        FgColor::Reset,
                    ];

                    #[cfg(feature = "logging")]
                    BLOCKED_CLICKS.fetch_add(1, Relaxed);

                    return 1;
                } else {
                    LAST_UP_L.store(tick, Relaxed);
                    log![
                        b"\tLeft button up event accepted (after ",
                        FgColor::TIME,
                        time_since_last_event,
                        b" ms",
                        FgColor::Reset,
                        b")\r\n",
                    ];
                }
            }
            WM_RBUTTONDOWNU => {
                let tick = GetTickCount();
                let time_since_last_event =
                    tick.saturating_sub(LAST_DOWN_R.load(Relaxed).max(LAST_UP_R.load(Relaxed)));

                if time_since_last_event < THRESHOLD_RM.load(Relaxed) {
                    log![
                        FgColor::BLOCKED,
                        b"Right click ignored (too frequent, within ",
                        FgColor::TIME,
                        time_since_last_event,
                        b" ms",
                        FgColor::BLOCKED,
                        b")\r\n",
                        FgColor::Reset,
                    ];

                    #[cfg(feature = "logging")]
                    BLOCKED_CLICKS.fetch_add(1, Relaxed);

                    return 1;
                } else {
                    LAST_DOWN_R.store(tick, Relaxed);
                    log![
                        b"Right click accepted (after ",
                        FgColor::TIME,
                        time_since_last_event,
                        b" ms",
                        FgColor::Reset,
                        b")\r\n",
                    ];
                }
            }
            WM_RBUTTONUPU => {
                let tick = GetTickCount();
                let time_since_last_event = tick.saturating_sub(LAST_UP_R.load(Relaxed));

                if time_since_last_event < THRESHOLD_RM.load(Relaxed) {
                    log![
                        FgColor::BLOCKED,
                        b"\tRight button up event ignored (too frequent, within ",
                        FgColor::TIME,
                        time_since_last_event,
                        b" ms",
                        FgColor::BLOCKED,
                        b")\r\n",
                        FgColor::Reset,
                    ];

                    #[cfg(feature = "logging")]
                    BLOCKED_CLICKS.fetch_add(1, Relaxed);

                    return 1;
                } else {
                    LAST_UP_R.store(tick, Relaxed);
                    log![
                        b"\tRight button up event accepted (after ",
                        FgColor::TIME,
                        time_since_last_event,
                        b" ms",
                        FgColor::Reset,
                        b")\r\n",
                    ];
                }
            }
            _ => (),
        }
    }

    CallNextHookEx(ptr::null_mut(), code, wparam, lparam)
}

fn parse_and_save_args() {
    let args = std_polyfill::args();

    let mut args = args.filter_map(|arg| {
        #[cfg(feature = "logging")]
        if arg.trim().eq_ignore_ascii_case("logging") {
            logging::set_should_log(true);
            return None;
        }
        Some(
            arg.parse::<u32>()
                .inspect_err(|e| log_error(e))
                .unwrap_or_else(|_| std_polyfill::exit(2)),
        )
    });

    if let Some(arg_lm) = args.next() {
        THRESHOLD_LM.store(arg_lm, Relaxed);
    }
    if let Some(arg_rm) = args.next() {
        THRESHOLD_RM.store(arg_rm, Relaxed);
    }
}

fn program_start() {
    parse_and_save_args();

    #[cfg(feature = "std")]
    {
        // Allow enabling logging using an environment variable:
        if std::env::var_os("CLICK_ONCE_LOGGING").is_some_and(|value| !value.is_empty()) {
            logging::set_should_log(true);
        }
    }

    log![
        b"\r\nProgram Config:\r\nLeft Click:  ",
        FgColor::TIME,
        THRESHOLD_LM.load(Relaxed),
        b" ms",
        FgColor::Reset,
        if THRESHOLD_LM.load(Relaxed) == 0 {
            b" (Disabled)".as_slice()
        } else {
            b""
        },
        b"\r\nRight Click: ",
        FgColor::TIME,
        THRESHOLD_RM.load(Relaxed),
        b" ms",
        FgColor::Reset,
        if THRESHOLD_RM.load(Relaxed) == 0 {
            b" (Disabled)".as_slice()
        } else {
            b""
        },
        b"\r\n\r\n",
    ];

    unsafe {
        SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), ptr::null_mut(), 0);
    }

    #[cfg(feature = "tray")]
    {
        #[cfg(feature = "logging")]
        use tray_icon::menu::CheckMenuItem;
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
            #[cfg(feature = "logging")]
            ToggleLogging,
        }

        let tray_menu = Menu::new();
        let quit_item = MenuItem::new("Quit", true, Some(Accelerator::new(None, Code::KeyQ)));
        #[cfg(feature = "logging")]
        let logging_item = CheckMenuItem::new(
            "Toggle Logging",
            true,
            logging::is_logging(),
            Some(Accelerator::new(None, Code::KeyL)),
        );

        tray_menu
            .append_items(&[
                #[cfg(feature = "logging")]
                &logging_item,
                &quit_item,
            ])
            .expect("Failed to add context menu items");

        let mut tray = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_tooltip({
                use std::fmt::Write;

                let mut tooltip = "click-once".to_owned();
                {
                    tooltip.push_str("\r\nLeft Click: ");
                    let threshold_left = THRESHOLD_LM.load(Relaxed);
                    if threshold_left == 0 {
                        tooltip.push_str("Disabled");
                    } else {
                        write!(tooltip, "{} ms", threshold_left).unwrap();
                    }
                }
                {
                    tooltip.push_str("\r\nRight Click: ");
                    let threshold_right = THRESHOLD_RM.load(Relaxed);
                    if threshold_right == 0 {
                        tooltip.push_str("Disabled");
                    } else {
                        write!(tooltip, "{} ms", threshold_right).unwrap();
                    }
                }
                tooltip
            });

        // https://learn.microsoft.com/en-us/windows/deployment/usmt/usmt-recognized-environment-variables
        match std::env::var("WINDIR") {
            Ok(win_dir) => {
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
        let event_loop = EventLoop::with_user_event().build().unwrap();

        MenuEvent::set_event_handler(Some({
            let proxy = event_loop.create_proxy();
            let quit_id = quit_item.id().clone();
            #[cfg(feature = "logging")]
            let logging_id = logging_item.id().clone();
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
            }
        }));

        struct App {
            tray: TrayIcon,
            #[cfg(feature = "logging")]
            logging_item: CheckMenuItem,
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
                            Clicks blocked since program was started: ",
                            BLOCKED_CLICKS.load(Relaxed),
                            b"\r\n\r\n\r\n",
                        ];
                    }
                }
            }
        }

        event_loop
            .run_app(&mut App {
                tray,
                #[cfg(feature = "logging")]
                logging_item,
            })
            .unwrap();
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
