use std::io::{self, IsTerminal};

/// Returns whether stdin is a terminal.
#[must_use]
pub fn is_stdin_terminal() -> bool {
    io::stdin().is_terminal()
}

/// Returns whether stdout is a terminal.
#[must_use]
pub fn is_stdout_terminal() -> bool {
    io::stdout().is_terminal()
}

/// Returns whether stderr is a terminal.
#[must_use]
pub fn is_stderr_terminal() -> bool {
    io::stderr().is_terminal()
}

/// Returns whether an interactive control terminal is available.
///
/// Interactive components read stdin and render to stderr, so both streams must
/// be terminals.
#[must_use]
pub fn is_terminal_available() -> bool {
    is_stdin_terminal() && is_stderr_terminal()
}

/// Returns the width of stdout's exact terminal handle, or zero when stdout is
/// redirected, the query fails, or the reported width is not positive.
#[must_use]
pub fn terminal_width() -> u16 {
    if !is_stdout_terminal() {
        return 0;
    }
    positive_width(platform::stdout_width())
}

fn positive_width(width: Option<u16>) -> u16 {
    width.filter(|width| *width > 0).unwrap_or(0)
}

#[cfg(unix)]
mod platform {
    use std::{
        ffi::{c_int, c_ulong},
        io,
        mem::MaybeUninit,
        os::fd::AsRawFd,
    };

    #[repr(C)]
    struct WindowSize {
        rows: u16,
        columns: u16,
        x_pixels: u16,
        y_pixels: u16,
    }

    unsafe extern "C" {
        fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    const GET_WINDOW_SIZE: c_ulong = 0x5413;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const GET_WINDOW_SIZE: c_ulong = 0x4008_7468;

    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    pub(super) fn stdout_width() -> Option<u16> {
        let stdout = io::stdout();
        let mut size = MaybeUninit::<WindowSize>::zeroed();
        // SAFETY: stdout's live descriptor is queried with the platform's
        // TIOCGWINSZ request, and `size` points to writable WindowSize storage.
        let status = unsafe { ioctl(stdout.as_raw_fd(), GET_WINDOW_SIZE, size.as_mut_ptr()) };
        if status == 0 {
            // SAFETY: a successful ioctl initialized the WindowSize output.
            Some(unsafe { size.assume_init() }.columns)
        } else {
            None
        }
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    pub(super) fn stdout_width() -> Option<u16> {
        None
    }
}

#[cfg(windows)]
mod platform {
    use std::{ffi::c_void, io, mem::MaybeUninit, os::windows::io::AsRawHandle};

    #[repr(C)]
    struct Coordinate {
        x: i16,
        y: i16,
    }

    #[repr(C)]
    struct Rectangle {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }

    #[repr(C)]
    struct ConsoleScreenBufferInfo {
        size: Coordinate,
        cursor_position: Coordinate,
        attributes: u16,
        window: Rectangle,
        maximum_window_size: Coordinate,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleScreenBufferInfo(
            console_output: *mut c_void,
            info: *mut ConsoleScreenBufferInfo,
        ) -> i32;
    }

    pub(super) fn stdout_width() -> Option<u16> {
        let stdout = io::stdout();
        let mut info = MaybeUninit::<ConsoleScreenBufferInfo>::zeroed();
        // SAFETY: stdout's live console handle and writable output storage are
        // passed to the Windows console API for the duration of this call.
        let success =
            unsafe { GetConsoleScreenBufferInfo(stdout.as_raw_handle().cast(), info.as_mut_ptr()) };
        if success == 0 {
            return None;
        }
        // SAFETY: a successful call initialized the output structure.
        let window = unsafe { info.assume_init() }.window;
        let width = window.right.checked_sub(window.left)?.checked_add(1)?;
        u16::try_from(width).ok()
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    pub(super) const fn stdout_width() -> Option<u16> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirected_test_stdout_has_no_width() {
        assert_eq!(terminal_width(), 0);
    }

    #[test]
    fn validates_only_the_reported_width() {
        assert_eq!(positive_width(Some(91)), 91);
        assert_eq!(positive_width(Some(0)), 0);
        assert_eq!(positive_width(None), 0);
    }
}
