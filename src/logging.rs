//! Implements logging by writing to a console window, optionally creating
//! such a window if it doesn't exist.

/// Create an array of [`LogValue`] by calling `from` on the provided items.
/// Won't actually log anything.
macro_rules! log_array {
    ($($value:expr),* $(,)?) => {
        [
            $($crate::logging::LogValue::from($value),)*
        ]
    };
}

#[cfg(feature = "tray")] // Note: implies "std" feature
pub mod stats {
    //! Track statistics and allow printing them. This module is only useful
    //! when we have a system tray since otherwise there is no way to interact
    //! with the program and request the statistics.

    use super::{LogValue, MouseButton, MouseDirection};
    use core::sync::atomic::{AtomicU32, Ordering::*};

    type LogWriteCallback<'a> = &'a mut dyn FnMut(LogValue<'_>);

    pub struct MouseEventStats {
        pub unblocked: AtomicU32,
        pub blocked: AtomicU32,
    }
    impl MouseEventStats {
        pub const fn new() -> Self {
            Self {
                unblocked: AtomicU32::new(0),
                blocked: AtomicU32::new(0),
            }
        }
        #[inline(always)]
        pub fn increment(&self, blocked: bool) {
            if blocked {
                _ = self.blocked.fetch_add(1, Relaxed);
            } else {
                _ = self.unblocked.fetch_add(1, Relaxed);
            }
        }
        pub fn get(button: MouseButton, direction: MouseDirection) -> &'static Self {
            macro_rules! define_stats {
                () => {{
                    static STATS: MouseEventStats = MouseEventStats::new();
                    &STATS
                }};
            }
            match (button, direction) {
                (MouseButton::Left, MouseDirection::Up) => define_stats!(),
                (MouseButton::Left, MouseDirection::Down) => define_stats!(),
                (MouseButton::Right, MouseDirection::Up) => define_stats!(),
                (MouseButton::Right, MouseDirection::Down) => define_stats!(),
                (MouseButton::Middle, MouseDirection::Up) => define_stats!(),
                (MouseButton::Middle, MouseDirection::Down) => define_stats!(),
            }
        }
        fn sum_stats(
            parts: impl Iterator<Item = (MouseButton, MouseDirection)>,
        ) -> MouseEventStats {
            let mut unblocked_sum = 0;
            let mut blocked_sum = 0;
            parts
                .map(|(btn, dir)| MouseEventStats::get(btn, dir))
                .for_each(|stats| {
                    unblocked_sum += stats.unblocked.load(Relaxed);
                    blocked_sum += stats.blocked.load(Relaxed);
                });
            MouseEventStats {
                unblocked: AtomicU32::new(unblocked_sum),
                blocked: AtomicU32::new(blocked_sum),
            }
        }
        fn log(&self, log_write: LogWriteCallback) {
            let blocked = self.blocked.load(Relaxed);
            let total = self.unblocked.load(Relaxed) + blocked;
            log_array![blocked, b" / ", total, b"  (",]
                .into_iter()
                .for_each(&mut *log_write);

            const MAX_TRAILING_DIGITS: usize = (u32::MAX.ilog10() + 1) as usize;
            const DOT_AND_PADDING: &[u8; 1 + MAX_TRAILING_DIGITS] = b".0000000000";
            let decimals: u32 = 4;

            if total == 0 {
                log_write(LogValue::Number(0));
                if decimals > 0 {
                    log_write(DOT_AND_PADDING[..(1 + decimals) as usize].into());
                }
            } else {
                let tens = 10_u64.pow(decimals);
                let percent = (blocked as u64 * 100 * 100 * tens) / (total as u64 * 100);

                log_write(((percent / tens) as u32).into());
                if decimals > 0 {
                    // Note: number formatting might print less than `decimals`
                    // digits if leading ones are zero, therefore we add padding
                    // with `0` character to not misrepresent the number. The
                    // result should be that the printed number always has
                    // `decimals` number of digits after the decimal sign.
                    let after_dot = (percent % tens) as u32;
                    let digits = if after_dot == 0 {
                        // We still print 0 in this case, so one digit will be printed:
                        1
                    } else {
                        after_dot.ilog10() + 1
                    };
                    let missing_digits = decimals.saturating_sub(digits) as usize;
                    log_write(DOT_AND_PADDING[..1 + missing_digits].into());
                    log_write(after_dot.into());
                }
            }
            log_write(b"%)".into());
        }
    }

    /// This function prints statistics about blocked clicks when a logging session
    /// is started via the tray icon.
    pub fn log_current_stats(log_write: LogWriteCallback) {
        fn log_stats_total_clicks(log_write: LogWriteCallback) {
            let sum =
                MouseEventStats::sum_stats(MouseButton::all().iter().copied().flat_map(|button| {
                    [button]
                        .into_iter()
                        .cycle()
                        .zip(MouseDirection::all().iter().copied())
                }));

            log_write(b"Total blocked events: ".into());
            sum.log(log_write);
            log_write(b"\r\n".into());
        }
        fn log_stats_for_button(button: MouseButton, log_write: LogWriteCallback) {
            let button_text = match button {
                MouseButton::Left => b"\tLeft button:   ",
                MouseButton::Right => b"\tRight button:  ",
                MouseButton::Middle => b"\tMiddle button: ",
            };
            log_write(button_text.into());

            let all_dirs = MouseEventStats::sum_stats(
                [button]
                    .into_iter()
                    .cycle()
                    .zip(MouseDirection::all().iter().copied()),
            );
            all_dirs.log(log_write);
            log_write(b"\r\n".into());
        }
        fn log_stats_for_button_with_direction(
            button: MouseButton,
            direction: MouseDirection,
            log_write: LogWriteCallback,
        ) {
            let dir_text = match direction {
                MouseDirection::Down => b"\t\tDown event: ",
                MouseDirection::Up => b"\t\tUp event:   ",
            };
            log_write(dir_text.into());
            let stats = MouseEventStats::get(button, direction);
            stats.log(log_write);
            log_write(b"\r\n".into());
        }

        log_write(b"\r\nStatistics:\r\n".into());

        log_stats_total_clicks(log_write);
        for &button in MouseButton::all() {
            log_stats_for_button(button, log_write);
            for &dir in MouseDirection::all() {
                log_stats_for_button_with_direction(button, dir, log_write);
            }
        }

        log_write(b"\r\n\r\n\r\n".into());
    }
}

use crate::{log, log_error};
use core::sync::atomic::{AtomicBool, Ordering::*};
use windows_sys::Win32::System::Console::{
    AllocConsole, AttachConsole, FreeConsole, GetStdHandle, SetConsoleTextAttribute, WriteConsoleA,
    ATTACH_PARENT_PROCESS, FOREGROUND_BLUE, FOREGROUND_GREEN, FOREGROUND_INTENSITY, FOREGROUND_RED,
    STD_OUTPUT_HANDLE,
};

/// The console window only exists in debug builds with `std` feature since that
/// is when we disable the: windows_subsystem = `windows` (also see the build
/// script were we also specify this subsystem).
static SHOULD_LOG: AtomicBool = AtomicBool::new(cfg!(all(debug_assertions, feature = "std")));

pub fn is_logging() -> bool {
    SHOULD_LOG.load(Acquire)
}

/// Create or destroy a console window.
///
/// # References
///
/// - <https://learn.microsoft.com/en-us/windows/console/allocconsole>
/// - <https://learn.microsoft.com/en-us/windows/console/attachconsole>
/// - <https://stackoverflow.com/questions/432832/what-is-the-different-between-api-functions-allocconsole-and-attachconsole-1>
pub fn set_should_log(enabled: bool) {
    if SHOULD_LOG
        .compare_exchange(!enabled, enabled, AcqRel, Relaxed)
        .is_ok()
    {
        let result = if enabled {
            let result = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
            if result == 0 {
                // Failed to attach to existing console, so create a new one:
                unsafe { AllocConsole() }
            } else {
                result
            }
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

/// Get info about the current program configuration. Lazy so does nothing by itself.
pub fn log_program_config() -> [LogValue<'static>; 19] {
    log_array![
        b"\r\nProgram Config:\r\nLeft Click:  ",
        FgColor::TIME,
        crate::THRESHOLD_LM.load(Relaxed),
        b" ms",
        FgColor::Reset,
        if crate::THRESHOLD_LM.load(Relaxed) == 0 {
            b" (Disabled)".as_slice()
        } else {
            b""
        },
        b"\r\nRight Click: ",
        FgColor::TIME,
        crate::THRESHOLD_RM.load(Relaxed),
        b" ms",
        FgColor::Reset,
        if crate::THRESHOLD_RM.load(Relaxed) == 0 {
            b" (Disabled)".as_slice()
        } else {
            b""
        },
        b"\r\nMiddle Click: ",
        FgColor::TIME,
        crate::THRESHOLD_MM.load(Relaxed),
        b" ms",
        FgColor::Reset,
        if crate::THRESHOLD_MM.load(Relaxed) == 0 {
            b" (Disabled)".as_slice()
        } else {
            b""
        },
        b"\r\n\r\n",
    ]
}

macro_rules! all_variants {
    ($($variant:ident),* $(,)?) => {{
        _ = |__enum: Self| {
            match __enum {
                $(Self::$variant => {},)*
            }
        };
        &[
            $(Self::$variant,)*
        ]
    }};
}

#[derive(Clone, Copy)]
pub enum MouseDirection {
    Up,
    Down,
}
impl MouseDirection {
    #[allow(dead_code, reason = "only used by certain features")]
    pub fn all() -> &'static [Self] {
        all_variants![Up, Down]
    }
}

#[derive(Clone, Copy)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}
impl MouseButton {
    #[allow(dead_code, reason = "only used by certain features")]
    pub fn all() -> &'static [Self] {
        all_variants![Left, Right, Middle]
    }
}

#[derive(Clone, Copy)]
pub struct MouseEvent {
    pub button: MouseButton,
    pub direction: MouseDirection,
    pub blocked: bool,
    pub time_since_last_event: u32,
}
impl MouseEvent {
    pub fn log(self) {
        #[cfg(feature = "tray")]
        stats::MouseEventStats::get(self.button, self.direction).increment(self.blocked);

        if is_logging() {
            self.log_write();
        }
    }
    #[cold]
    fn log_write(self) {
        if self.blocked {
            log![FgColor::BLOCKED];
        }

        match (self.button, self.direction) {
            (MouseButton::Left, MouseDirection::Up) => log![b"\tLeft button up event "],
            (MouseButton::Left, MouseDirection::Down) => log![b"Left click "],
            (MouseButton::Right, MouseDirection::Up) => log![b"\tRight button up event "],
            (MouseButton::Right, MouseDirection::Down) => log![b"Right click "],
            (MouseButton::Middle, MouseDirection::Up) => log![b"\tMiddle button up event "],
            (MouseButton::Middle, MouseDirection::Down) => log![b"Middle click "],
        }

        if self.blocked {
            log![
                b"ignored (too frequent, within ",
                FgColor::TIME,
                self.time_since_last_event,
                b" ms",
                FgColor::BLOCKED,
                b")\r\n",
                FgColor::Reset,
            ];
        } else {
            log![
                b"accepted (after ",
                FgColor::TIME,
                self.time_since_last_event,
                b" ms",
                FgColor::Reset,
                b")\r\n",
            ];
        }
    }
}

/// A value that can be written to a console window.
#[derive(Clone, Copy)]
#[must_use = "Call write() to actually log something"]
pub enum LogValue<'a> {
    /// A number.
    Number(u32),
    /// ASCII text.
    Text(&'a [u8]),
    Color(FgColor),
}
impl<'a> LogValue<'a> {
    #[cfg(feature = "tray")]
    pub fn write_to_string(self, buffer: &mut String) {
        match self {
            LogValue::Number(number) => {
                let mut num_buf = itoa::Buffer::new();
                buffer.push_str(num_buf.format(number));
            }
            LogValue::Text(text) => {
                buffer.push_str(core::str::from_utf8(text).unwrap_or_else(|e| {
                    log_error(format_args!(
                        "LogValue::Text should only contain ASCII: {e}"
                    ));
                    crate::std_polyfill::exit(1);
                }));
            }
            LogValue::Color(_) => {}
        }
    }
    /// Write this value to the console.
    ///
    /// # References
    ///
    /// - <https://stackoverflow.com/questions/28890402/win32-console-write-c-c>
    /// - <https://learn.microsoft.com/en-us/windows/console/writeconsole>
    /// - <https://docs.rs/windows-sys/0.52.0/windows_sys/Win32/System/Console/fn.WriteConsoleA.html>
    pub fn write(self) {
        if let LogValue::Text(b"") = self {
            return;
        }
        if !SHOULD_LOG.load(Acquire) {
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

/// Terminal colors using ANSI escape codes or Windows console text attributes.
///
/// # References
///
/// - <https://en.wikipedia.org/wiki/ANSI_escape_code>
/// - <https://stackoverflow.com/questions/2348000/colors-in-c-win32-console>
/// - <https://stackoverflow.com/questions/43539956/how-to-stop-the-effect-of-ansi-text-color-code-or-set-text-color-back-to-default>
#[derive(Clone, Copy)]
#[expect(dead_code, reason = "we don't use all these colors yet")]
pub enum FgColor {
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
    pub const BLOCKED: Self = Self::BrightRed;
    /// Color for parts of log messages that includes time values.
    pub const TIME: Self = Self::BrightCyan;
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
