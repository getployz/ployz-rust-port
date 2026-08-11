use std::{
    fmt,
    io::{self, IsTerminal, Write},
};

use iocraft::prelude::*;

use crate::BOLD;

/// A borderless table with bold headers and three cells of right padding.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    /// Creates an empty table.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            headers: Vec::new(),
            rows: Vec::new(),
        }
    }

    /// Replaces the table header.
    pub fn headers<I, S>(&mut self, headers: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.headers = headers.into_iter().map(Into::into).collect();
        self
    }

    /// Appends a row.
    pub fn row<I, S>(&mut self, row: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.rows.push(row.into_iter().map(Into::into).collect());
        self
    }

    /// Renders the table without a trailing newline.
    #[must_use]
    pub fn render(&self) -> String {
        let column_count = self
            .rows
            .iter()
            .map(Vec::len)
            .chain([self.headers.len()])
            .max()
            .unwrap_or(0);
        if column_count == 0 {
            return String::new();
        }

        let mut widths = vec![0; column_count];
        for row in std::iter::once(&self.headers).chain(&self.rows) {
            for (column, value) in row.iter().enumerate() {
                widths[column] = widths[column].max(display_width(value));
            }
        }

        let mut output = String::new();
        if !self.headers.is_empty() {
            append_row(&mut output, &self.headers, &widths, true);
        }
        for row in &self.rows {
            append_row(&mut output, row, &widths, false);
        }
        output.pop();
        output
    }

    /// Prints the table to stdout, preserving redirected-output cleanliness.
    pub fn print(&self) -> io::Result<()> {
        let stdout = io::stdout();
        let mut writer = stdout.lock();
        let terminal = writer.is_terminal();
        write_profiled(&mut writer, &self.render(), terminal)?;
        writeln!(writer)
    }
}

impl fmt::Display for Table {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

fn append_row(output: &mut String, row: &[String], widths: &[usize], header: bool) {
    for (column, width) in widths.iter().copied().enumerate() {
        let value = row.get(column).map_or("", String::as_str);
        let styled = if header {
            BOLD.render(value)
        } else {
            value.to_owned()
        };
        output.push_str(&styled);
        let padding = width.saturating_sub(display_width(value)) + 3;
        output.extend(std::iter::repeat_n(' ', padding));
    }
    output.push('\n');
}

fn write_profiled(mut writer: impl Write, rendered: &str, terminal: bool) -> io::Result<()> {
    if terminal {
        writer.write_all(rendered.as_bytes())
    } else {
        writer.write_all(strip_ansi(rendered).as_bytes())
    }
}

fn display_width(value: &str) -> usize {
    let plain = strip_ansi(value);
    let mut element = element!(Text(content: plain, wrap: TextWrap::NoWrap));
    element.render(None).width()
}

fn strip_ansi(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if (b'@'..=b'~').contains(&byte) {
                    break;
                }
            }
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GREEN, NO_STYLE};

    #[test]
    fn renders_borderless_padded_table() {
        let mut table = Table::new();
        table.headers(["NAME", "STATE"]);
        table.row(["alpha", "running"]);
        table.row(["b", "stopped"]);

        assert_eq!(
            strip_ansi(&table.render()),
            "NAME    STATE     \nalpha   running   \nb       stopped   "
        );
    }

    #[test]
    fn preserves_styled_cells_and_uses_visible_width() {
        let mut table = Table::new();
        table.headers(["IMAGE", "STATE"]);
        table.row([GREEN.render("ubuntu:latest"), "up".to_owned()]);
        table.row([NO_STYLE.render("x"), "down".to_owned()]);
        let rendered = table.render();
        assert!(rendered.contains("\x1b["));
        assert_eq!(
            strip_ansi(&rendered),
            "IMAGE           STATE   \nubuntu:latest   up      \nx               down    "
        );
    }

    #[test]
    fn unicode_width_comes_from_iocraft() {
        assert_eq!(display_width("界"), 2);
        assert_eq!(display_width("e\u{301}"), 1);
    }

    #[test]
    fn redirected_profile_strips_style_but_preserves_layout() {
        let mut table = Table::new();
        table.headers(["NAME"]);
        table.row([GREEN.render("alpha")]);
        let mut output = Vec::new();
        write_profiled(&mut output, &table.render(), false).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "NAME    \nalpha   ");
    }

    #[test]
    fn display_has_no_trailing_newline_for_caller_control() {
        let mut table = Table::new();
        table.row(["value"]);
        assert_eq!(table.to_string(), "value   ");
    }
}
