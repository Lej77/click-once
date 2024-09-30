//! Implements logging by writing to a console window, optionally creating
//! such a window if it doesn't exist (which it only does in debug builds
//! when `std` is enabled since we then use the default `console` subsystem).

use crate::{log, log_error};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering::*};
use windows_sys::Win32::System::Console::{
    AllocConsole, FreeConsole, GetStdHandle, SetConsoleTextAttribute, WriteConsoleA,
    FOREGROUND_BLUE, FOREGROUND_GREEN, FOREGROUND_INTENSITY, FOREGROUND_RED, STD_OUTPUT_HANDLE,
};

static SHOULD_LOG: AtomicBool = AtomicBool::new(cfg!(all(debug_assertions, feature = "std")));

pub fn is_logging() -> bool {
    SHOULD_LOG.load(Acquire)
}

/// Create or destroy a console window.
///
/// # References
///
/// - <https://learn.microsoft.com/en-us/windows/console/allocconsole>
pub fn set_should_log(enabled: bool) {
    if SHOULD_LOG
        .compare_exchange(!enabled, enabled, AcqRel, Relaxed)
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

pub fn log_program_config() {
    log![
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
        b"\r\n\r\n",
    ];
}

/// This function prints statistics about blocked clicks when a logging session
/// is started via the tray icon.
#[cfg_attr(
    not(feature = "tray"),
    expect(
        dead_code,
        reason = "Currently the only way to trigger statistics printing is via the tray."
    )
)]
pub fn log_current_stats() {
    fn log_stats_total_clicks() {
        let sum =
            MouseEventStats::sum_stats(MouseButton::all().iter().copied().flat_map(|button| {
                [button]
                    .into_iter()
                    .cycle()
                    .zip(MouseDirection::all().iter().copied())
            }));

        log![b"Total blocked events: ",];
        sum.log();
        log![b"\r\n"];
    }
    fn log_stats_for_button(button: MouseButton) {
        match button {
            MouseButton::Left => log![b"\tLeft button:  "],
            MouseButton::Right => log![b"\tRight button: "],
        }
        let all_dirs = MouseEventStats::sum_stats(
            [button]
                .into_iter()
                .cycle()
                .zip(MouseDirection::all().iter().copied()),
        );
        all_dirs.log();
        log![b"\r\n"];
    }
    fn log_stats_for_button_with_direction(button: MouseButton, direction: MouseDirection) {
        match direction {
            MouseDirection::Down => log![b"\t\tDown event: "],
            MouseDirection::Up => log![b"\t\tUp event:   "],
        }
        let stats = MouseEventStats::get(button, direction);
        stats.log();
        log![b"\r\n"];
    }

    log![b"\r\nStatistics:\r\n",];

    log_stats_total_clicks();
    for &button in MouseButton::all() {
        log_stats_for_button(button);
        for &dir in MouseDirection::all() {
            log_stats_for_button_with_direction(button, dir);
        }
    }

    log![b"\r\n\r\n\r\n"];
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
    pub fn all() -> &'static [Self] {
        all_variants![Up, Down]
    }
}

#[derive(Clone, Copy)]
pub enum MouseButton {
    Left,
    Right,
}
impl MouseButton {
    pub fn all() -> &'static [Self] {
        all_variants![Left, Right]
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
        let stats = self.associated_stats();
        if self.blocked {
            _ = stats.blocked.fetch_add(1, Relaxed);
        } else {
            _ = stats.unblocked.fetch_add(1, Relaxed);
        }

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
    fn associated_stats(&self) -> &'static MouseEventStats {
        MouseEventStats::get(self.button, self.direction)
    }
}

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
        }
    }
    fn sum_stats(parts: impl Iterator<Item = (MouseButton, MouseDirection)>) -> MouseEventStats {
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
    fn log(&self) {
        let blocked = self.blocked.load(Relaxed);
        let total = self.unblocked.load(Relaxed) + blocked;
        log![blocked, b" / ", total, b"  (",];
        if total == 0 {
            log![0];
        } else {
            let decimals = 4;
            let tens = 10_u64.pow(decimals);
            let percent = (blocked as u64 * 100 * 100 * tens) / (total as u64 * 100);

            log![(percent / tens) as u32];
            if decimals > 0 {
                log![b".", (percent % tens) as u32,];
            }
        }
        log![b"%)"];
    }
}

/// A value that can be written to a console window.
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
