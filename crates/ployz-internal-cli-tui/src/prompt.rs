use std::io::{self, BufRead, Write};

use crate::{BOLD_RED, BOLD_YELLOW};

const DEFAULT_TITLE: &str = "Do you want to continue?";

/// Visual theme for a confirmation title.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConfirmTheme {
    /// Yellow title for ordinary confirmations.
    #[default]
    Normal,
    /// Red title for destructive confirmations.
    Danger,
}

impl ConfirmTheme {
    /// Renders a confirmation title using this theme.
    #[must_use]
    pub fn render_title(self, title: &str) -> String {
        match self {
            Self::Normal => BOLD_YELLOW.render(title),
            Self::Danger => BOLD_RED.render(title),
        }
    }
}

/// Shows a confirmation prompt and returns the user's choice.
///
/// The accessible cooked prompt is rendered to stderr, even when a control
/// terminal is available, so screen readers and redirected input behave alike.
pub fn confirm(title: &str) -> io::Result<bool> {
    confirm_with_theme(title, ConfirmTheme::Normal)
}

/// Shows a destructive confirmation using the red danger title theme.
///
/// Input, output routing, retries, and error suppression are identical to
/// [`confirm`].
pub fn confirm_danger(title: &str) -> io::Result<bool> {
    confirm_with_theme(title, ConfirmTheme::Danger)
}

/// Shows a confirmation prompt with an explicit normal or danger theme.
fn confirm_with_theme(title: &str, theme: ConfirmTheme) -> io::Result<bool> {
    let title = normalized_title(title);
    confirm_with_io_and_theme(io::stdin().lock(), io::stderr().lock(), title, theme)
}

/// Runs the accessible, non-terminal confirmation interaction.
///
/// This entry point also makes stream routing and input behavior executable in
/// unit tests.
#[cfg(test)]
fn confirm_with_io(input: impl BufRead, output: impl Write, title: &str) -> io::Result<bool> {
    confirm_with_io_and_theme(input, output, title, ConfirmTheme::Normal)
}

fn confirm_with_io_and_theme(
    mut input: impl BufRead,
    mut output: impl Write,
    title: &str,
    theme: ConfirmTheme,
) -> io::Result<bool> {
    let title = normalized_title(title);
    let prompt = format!("{} ", theme.render_title(&format!("{title} [y/N]")));

    let confirmed = loop {
        let _ = write!(output, "{prompt}");
        match scan_line(&mut input) {
            Some(response) if is_blank(&response) => break false,
            Some(response) if is_yes(&response) => break true,
            Some(response) if is_no(&response) => break false,
            Some(_) => {
                let _ = writeln!(output, "invalid input. please try again");
            }
            None => {
                let _ = writeln!(output);
                break false;
            }
        }
    };
    let _ = writeln!(output);
    Ok(confirmed)
}

const MAX_SCAN_TOKEN_SIZE: usize = 64 * 1024;

fn scan_line(input: &mut impl BufRead) -> Option<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let buffer = input.fill_buf().ok()?;
        if buffer.is_empty() {
            return if line.is_empty() { None } else { Some(line) };
        }

        if let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
            if line.len() + newline >= MAX_SCAN_TOKEN_SIZE {
                return None;
            }
            line.extend_from_slice(&buffer[..newline]);
            input.consume(newline + 1);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Some(line);
        }

        if line.len() + buffer.len() >= MAX_SCAN_TOKEN_SIZE {
            return None;
        }
        let consumed = buffer.len();
        line.extend_from_slice(buffer);
        input.consume(consumed);
    }
}

fn is_blank(response: &[u8]) -> bool {
    std::str::from_utf8(response).is_ok_and(|response| response.trim().is_empty())
}

fn is_yes(response: &[u8]) -> bool {
    response.eq_ignore_ascii_case(b"y") || response.eq_ignore_ascii_case(b"yes")
}

fn is_no(response: &[u8]) -> bool {
    response.eq_ignore_ascii_case(b"n") || response.eq_ignore_ascii_case(b"no")
}

fn normalized_title(title: &str) -> &str {
    if title.is_empty() {
        DEFAULT_TITLE
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_title_uses_default_and_routes_prompt_to_given_output() {
        let mut output = Vec::new();
        let confirmed = confirm_with_io("yes\n".as_bytes(), &mut output, "").unwrap();
        assert!(confirmed);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(DEFAULT_TITLE));
        assert!(output.contains(" [y/N]"));
        assert!(output.ends_with("\x1b[0m \n"));
    }

    #[test]
    fn accessible_prompt_accepts_only_explicit_yes() {
        for input in ["y\n", "Y\n", "yes\n", "YES\n"] {
            assert!(confirm_with_io(input.as_bytes(), Vec::new(), "continue?").unwrap());
        }
        for input in ["\n", "n\n", "no\n"] {
            assert!(!confirm_with_io(input.as_bytes(), Vec::new(), "continue?").unwrap());
        }
    }

    #[test]
    fn accessible_prompt_retries_invalid_untrimmed_input() {
        let mut output = Vec::new();
        let confirmed = confirm_with_io(" yes \ny\n".as_bytes(), &mut output, "continue?").unwrap();

        assert!(confirmed);
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.matches("continue? [y/N]").count(), 2);
        assert!(output.contains("invalid input. please try again\n"));
        assert!(output.ends_with('\n'));
    }

    #[test]
    fn accessible_prompt_defaults_false_after_eof_or_scanner_overflow() {
        let mut eof_output = Vec::new();
        assert!(!confirm_with_io([].as_slice(), &mut eof_output, "continue?").unwrap());
        assert!(eof_output.ends_with(b"\n\n"));

        let oversized = format!("{}\ny\n", "x".repeat(65_537));
        let mut oversized_output = Vec::new();
        assert!(
            !confirm_with_io(oversized.as_bytes(), &mut oversized_output, "continue?").unwrap()
        );
        assert_eq!(
            String::from_utf8(oversized_output)
                .unwrap()
                .matches("continue? [y/N]")
                .count(),
            1
        );
    }

    #[test]
    fn accessible_prompt_suppresses_output_errors_and_still_reads_input() {
        struct BrokenOutput;

        impl Write for BrokenOutput {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }
        }

        let mut input = io::Cursor::new(b"yes\n".as_slice());
        assert!(confirm_with_io(&mut input, BrokenOutput, "continue?").unwrap());
        assert_eq!(input.position(), 4);
    }

    #[test]
    fn danger_theme_uses_red_title() {
        let rendered = ConfirmTheme::Danger.render_title("delete?");
        assert!(rendered.contains("delete?"));
        assert!(rendered.starts_with("\x1b["));
    }
}
