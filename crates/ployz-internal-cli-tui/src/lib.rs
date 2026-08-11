//! Terminal presentation primitives shared by Ployz command-line programs.
//!
//! Static rendering is intentionally independent from terminal detection: callers
//! can compose styled fragments before choosing an output stream. Interactive
//! operations render to stderr so stdout remains suitable for machine-readable
//! command output.

mod format;
mod image;
mod print;
mod prompt;
mod runtime;
mod spinner;
mod style;
mod table;
mod terminal;

pub use format::format_rtt;
pub use image::format_image;
pub use iocraft::Color;
pub use print::print_warning;
pub use prompt::{ConfirmTheme, confirm, confirm_danger};
pub use spinner::{CancellationToken, Spinner, SpinnerError, run_spinner};
pub use style::{
    BOLD, BOLD_GREEN, BOLD_RED, BOLD_YELLOW, FAINT, GREEN, NAME_STYLE, NO_STYLE, RED, Style,
    URL_STYLE, YELLOW,
};
pub use table::Table;
pub use terminal::{
    is_stderr_terminal, is_stdin_terminal, is_stdout_terminal, is_terminal_available,
    terminal_width,
};
