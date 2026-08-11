use std::io::{self, Write};

use crate::BOLD_YELLOW;

fn write_warning(mut writer: impl Write, message: &str) -> io::Result<()> {
    writeln!(
        writer,
        "{}",
        BOLD_YELLOW.render(format!("WARNING: {message}"))
    )
}

/// Prints a styled warning to stderr on a best-effort basis.
pub fn print_warning(message: &str) {
    let _ = write_warning(io::stderr().lock(), message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_has_prefix_and_exact_newline_behavior() {
        let mut output = Vec::new();
        write_warning(&mut output, "disk almost full").unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("WARNING: disk almost full"));
        assert!(output.ends_with("\x1b[0m\n"));
    }

    #[test]
    fn embedded_newline_is_preserved_and_print_adds_one_more() {
        let mut output = Vec::new();
        write_warning(&mut output, "line one\nline two\n").unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.ends_with("\n\x1b[0m\n"));
    }

    #[test]
    fn injected_writer_reports_failure_below_best_effort_public_api() {
        struct BrokenOutput;

        impl Write for BrokenOutput {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }
        }

        assert_eq!(
            write_warning(BrokenOutput, "still best effort")
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
