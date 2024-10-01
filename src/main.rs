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
        crate::free_mouse_hook();
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

    #[inline]
    pub fn exit(code: i32) -> ! {
        crate::free_mouse_hook();
        std::process::exit(code);
    }

    /// Wrapper around [`std::env::args`] that skips the first argument (which
    /// would otherwise be the executable's path).
    pub fn args() -> impl Iterator<Item = String> {
        std::env::args().skip(1)
    }
}

#[cfg(feature = "logging")]
mod logging;
#[cfg(feature = "tray")]
mod tray;

use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering::Relaxed};
use core::*;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP,
};

macro_rules! log_mouse_event {
    ($button:ident, $direction:ident, $blocked:expr, $time_since_last_event:expr) => {
        #[cfg(feature = "logging")]
        $crate::logging::MouseEvent {
            button: $crate::logging::MouseButton::$button,
            direction: $crate::logging::MouseDirection::$direction,
            blocked: $blocked,
            time_since_last_event: $time_since_last_event,
        }
        .log();
    };
}

/// Logs values to console if the `logging` Cargo feature is enabled and a
/// console has been created (for example using the tray icon).
macro_rules! _log {
    ($($value:expr),* $(,)?) => {{
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
    }};
}
// Allow macro to be used on lines before it was declared:
#[allow(
    unused_imports,
    reason = "might not be used when all feature flags are disabled"
)]
use _log as log;

#[inline(always)] // <- so that the argument can be removed when this is a noop
fn log_error(_error: impl core::fmt::Display) {
    #[cfg(all(feature = "std", debug_assertions, not(feature = "logging")))]
    {
        eprintln!("Error: {_error}");
    }
    #[cfg(all(feature = "std", feature = "logging"))]
    {
        use std::io::Write;

        _ = writeln!(std::io::stdout(), "{}", _error);
    }
}

/// If a left mouse button event happens faster than this many milliseconds
/// then it is suppressed.
static THRESHOLD_LM: AtomicU32 = AtomicU32::new(30);

/// If a right mouse button event happens faster than this many milliseconds
/// then it is suppressed.
static THRESHOLD_RM: AtomicU32 = AtomicU32::new(0);

/// If a middle mouse button event happens faster than this many milliseconds
/// then it is suppressed.
static THRESHOLD_MM: AtomicU32 = AtomicU32::new(0);

const WM_LBUTTONDOWNU: usize = WM_LBUTTONDOWN as _;
const WM_LBUTTONUPU: usize = WM_LBUTTONUP as _;
const WM_RBUTTONDOWNU: usize = WM_RBUTTONDOWN as _;
const WM_RBUTTONUPU: usize = WM_RBUTTONUP as _;
const WM_MBUTTONDOWNU: usize = WM_MBUTTONDOWN as _;
const WM_MBUTTONUPU: usize = WM_MBUTTONUP as _;

unsafe extern "system" fn low_level_mouse_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    static LAST_DOWN_L: AtomicU32 = AtomicU32::new(0);
    static LAST_UP_L: AtomicU32 = AtomicU32::new(0);
    static LAST_DOWN_R: AtomicU32 = AtomicU32::new(0);
    static LAST_UP_R: AtomicU32 = AtomicU32::new(0);
    static LAST_DOWN_M: AtomicU32 = AtomicU32::new(0);
    static LAST_UP_M: AtomicU32 = AtomicU32::new(0);

    if code >= 0 {
        match wparam {
            WM_LBUTTONDOWNU => {
                let tick = GetTickCount();
                let time_since_last_event =
                    tick.saturating_sub(LAST_DOWN_L.load(Relaxed).max(LAST_UP_L.load(Relaxed)));

                if time_since_last_event < THRESHOLD_LM.load(Relaxed) {
                    log_mouse_event!(Left, Down, true, time_since_last_event);
                    return 1;
                } else {
                    LAST_DOWN_L.store(tick, Relaxed);
                    log_mouse_event!(Left, Down, false, time_since_last_event);
                }
            }
            WM_LBUTTONUPU => {
                let tick = GetTickCount();
                let time_since_last_event = tick.saturating_sub(LAST_UP_L.load(Relaxed));

                if time_since_last_event < THRESHOLD_LM.load(Relaxed) {
                    log_mouse_event!(Left, Up, true, time_since_last_event);
                    return 1;
                } else {
                    LAST_UP_L.store(tick, Relaxed);
                    log_mouse_event!(Left, Up, false, time_since_last_event);
                }
            }
            WM_RBUTTONDOWNU => {
                let tick = GetTickCount();
                let time_since_last_event =
                    tick.saturating_sub(LAST_DOWN_R.load(Relaxed).max(LAST_UP_R.load(Relaxed)));

                if time_since_last_event < THRESHOLD_RM.load(Relaxed) {
                    log_mouse_event!(Right, Down, true, time_since_last_event);
                    return 1;
                } else {
                    LAST_DOWN_R.store(tick, Relaxed);
                    log_mouse_event!(Right, Down, false, time_since_last_event);
                }
            }
            WM_RBUTTONUPU => {
                let tick = GetTickCount();
                let time_since_last_event = tick.saturating_sub(LAST_UP_R.load(Relaxed));

                if time_since_last_event < THRESHOLD_RM.load(Relaxed) {
                    log_mouse_event!(Right, Up, true, time_since_last_event);
                    return 1;
                } else {
                    LAST_UP_R.store(tick, Relaxed);
                    log_mouse_event!(Right, Up, false, time_since_last_event);
                }
            }
            WM_MBUTTONDOWNU => {
                let tick = GetTickCount();
                let time_since_last_event =
                    tick.saturating_sub(LAST_DOWN_M.load(Relaxed).max(LAST_UP_M.load(Relaxed)));

                if time_since_last_event < THRESHOLD_MM.load(Relaxed) {
                    log_mouse_event!(Middle, Down, true, time_since_last_event);
                    return 1;
                } else {
                    LAST_DOWN_M.store(tick, Relaxed);
                    log_mouse_event!(Middle, Down, false, time_since_last_event);
                }
            }
            WM_MBUTTONUPU => {
                let tick = GetTickCount();
                let time_since_last_event = tick.saturating_sub(LAST_UP_M.load(Relaxed));

                if time_since_last_event < THRESHOLD_MM.load(Relaxed) {
                    log_mouse_event!(Middle, Up, true, time_since_last_event);
                    return 1;
                } else {
                    LAST_UP_M.store(tick, Relaxed);
                    log_mouse_event!(Middle, Up, false, time_since_last_event);
                }
            }
            _ => (),
        }
    }

    CallNextHookEx(ptr::null_mut(), code, wparam, lparam)
}

#[cfg_attr(
    not(feature = "logging"),
    expect(
        clippy::unnecessary_filter_map,
        reason = "Only use None case when parsing \"logging\" argument"
    )
)]
fn parse_and_save_args() {
    let args = std_polyfill::args();

    let mut args = args.enumerate().filter_map(|(ix, arg)| {
        #[cfg(feature = "logging")]
        if arg.trim().eq_ignore_ascii_case("logging") {
            logging::set_should_log(true);
            return None;
        }
        Some(
            arg.parse::<u32>()
                .inspect_err(|e| {
                    log_error(format_args!(
                        "CLI argument \"{arg}\" at position {} is invalid, \
                        could not parse it as positive integer: {e}",
                        ix + 1
                    ))
                })
                .unwrap_or_else(|_| std_polyfill::exit(2)),
        )
    });

    if let Some(arg_lm) = args.next() {
        THRESHOLD_LM.store(arg_lm, Relaxed);
    }
    if let Some(arg_rm) = args.next() {
        THRESHOLD_RM.store(arg_rm, Relaxed);
    }
    if let Some(arg_mm) = args.next() {
        THRESHOLD_MM.store(arg_mm, Relaxed);
    }
    if let Some(extra_arg) = args.next() {
        log_error(format_args!(
            "Too many integers provided as arguments, could not use: {extra_arg}"
        ));
        std_polyfill::exit(2);
    }
}

static MOUSE_HOOK: AtomicPtr<ffi::c_void> = AtomicPtr::new(ptr::null_mut());
fn free_mouse_hook() {
    let mouse_hook = MOUSE_HOOK.swap(ptr::null_mut(), Relaxed);
    if !mouse_hook.is_null() {
        unsafe { UnhookWindowsHookEx(mouse_hook) };
    }
}

fn program_start() {
    #[cfg(all(feature = "std", feature = "logging"))]
    {
        // Allow enabling logging using an environment variable:
        if std::env::var_os("CLICK_ONCE_LOGGING").is_some_and(|value| !value.is_empty()) {
            logging::set_should_log(true);
        }
    }

    parse_and_save_args();

    #[cfg(feature = "logging")]
    logging::log_program_config()
        .iter()
        .for_each(|value| value.write());

    let guard = {
        let mouse_hook = unsafe {
            SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), ptr::null_mut(), 0)
        };
        if mouse_hook.is_null() {
            log_error("Failed to install mouse hook!");
            std_polyfill::exit(1);
        }
        if MOUSE_HOOK
            .compare_exchange(ptr::null_mut(), mouse_hook, Relaxed, Relaxed)
            .is_err()
        {
            log_error("Mouse hook was set more than once");

            unsafe { UnhookWindowsHookEx(mouse_hook) };
            std_polyfill::exit(1);
        }

        struct FinallyFreeHook;
        impl Drop for FinallyFreeHook {
            fn drop(&mut self) {
                free_mouse_hook();
            }
        }
        FinallyFreeHook
    };

    #[cfg(feature = "tray")]
    tray::run_event_loop_with_tray();

    // Simples event loop replacement:
    #[cfg(not(feature = "tray"))]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetMessageW;

        GetMessageW(&mut mem::zeroed(), ptr::null_mut(), 0, 0);
    }

    drop(guard);
}

#[cfg(feature = "std")]
fn main() {
    program_start();
}
