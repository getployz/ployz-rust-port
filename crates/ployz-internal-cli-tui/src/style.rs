use iocraft::prelude::*;

/// A composable terminal text style.
///
/// Rendering is backed by iocraft's canvas so color encoding and display-width
/// behavior follow the selected terminal stack. The value is small and copyable;
/// builder methods therefore mirror ordinary Rust value semantics.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Style {
    color: Option<Color>,
    weight: Weight,
    underline: bool,
    padding_right: u32,
    width: Option<u32>,
}

impl Style {
    /// Returns this style with bold text.
    #[must_use]
    pub const fn bold(mut self) -> Self {
        self.weight = Weight::Bold;
        self
    }

    /// Returns this style with faint text.
    #[must_use]
    pub const fn faint(mut self) -> Self {
        self.weight = Weight::Light;
        self
    }

    /// Returns this style with underline enabled or disabled.
    #[must_use]
    pub const fn underline(mut self, enabled: bool) -> Self {
        self.underline = enabled;
        self
    }

    /// Returns this style with the given iocraft color.
    #[must_use]
    pub const fn foreground(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Adds display-cell padding after rendered content.
    #[must_use]
    pub const fn padding_right(mut self, cells: u32) -> Self {
        self.padding_right = cells;
        self
    }

    /// Constrains the rendered fragment to the given display width.
    #[must_use]
    pub const fn width(mut self, cells: u32) -> Self {
        self.width = Some(cells);
        self
    }

    /// Renders text with iocraft's ANSI style output.
    #[must_use]
    pub fn render(self, text: impl AsRef<str>) -> String {
        let text = text.as_ref();
        if self == NO_STYLE {
            return text.to_owned();
        }

        let current_width = display_width(text);
        let target_width = self
            .width
            .map_or(current_width, |width| current_width.max(width as usize));
        let padding = target_width - current_width + self.padding_right as usize;
        let mut content = String::with_capacity(text.len() + padding);
        content.push_str(text);
        content.extend(std::iter::repeat_n(' ', padding));
        if content.is_empty() {
            return content;
        }

        let mut codes = Vec::with_capacity(3);
        match self.weight {
            Weight::Normal => {}
            Weight::Bold => codes.push("1".to_owned()),
            Weight::Light => codes.push("2".to_owned()),
        }
        if self.underline {
            codes.push("4".to_owned());
        }
        if let Some(color) = self.color {
            codes.push(foreground_sgr(color));
        }
        if codes.is_empty() {
            content
        } else {
            format!("\x1b[{}m{content}\x1b[0m", codes.join(";"))
        }
    }

    pub(crate) fn content(self, text: impl ToString) -> MixedTextContent {
        MixedTextContent::new(text)
            .weight(self.weight)
            .decoration(if self.underline {
                TextDecoration::Underline
            } else {
                TextDecoration::None
            })
            .with_optional_color(self.color)
    }
}

fn display_width(value: &str) -> usize {
    let mut element = element!(Text(content: value, wrap: TextWrap::NoWrap));
    element.render(None).width()
}

fn foreground_sgr(color: Color) -> String {
    match color {
        Color::Reset => "39".to_owned(),
        Color::Black => "38;5;0".to_owned(),
        Color::DarkGrey => "38;5;8".to_owned(),
        Color::Red => "38;5;9".to_owned(),
        Color::DarkRed => "38;5;1".to_owned(),
        Color::Green => "38;5;10".to_owned(),
        Color::DarkGreen => "38;5;2".to_owned(),
        Color::Yellow => "38;5;11".to_owned(),
        Color::DarkYellow => "38;5;3".to_owned(),
        Color::Blue => "38;5;12".to_owned(),
        Color::DarkBlue => "38;5;4".to_owned(),
        Color::Magenta => "38;5;13".to_owned(),
        Color::DarkMagenta => "38;5;5".to_owned(),
        Color::Cyan => "38;5;14".to_owned(),
        Color::DarkCyan => "38;5;6".to_owned(),
        Color::White => "38;5;15".to_owned(),
        Color::Grey => "38;5;7".to_owned(),
        Color::Rgb { r, g, b } => format!("38;2;{r};{g};{b}"),
        Color::AnsiValue(value) => format!("38;5;{value}"),
    }
}

trait OptionalColor {
    fn with_optional_color(self, color: Option<Color>) -> Self;
}

impl OptionalColor for MixedTextContent {
    fn with_optional_color(self, color: Option<Color>) -> Self {
        match color {
            Some(color) => self.color(color),
            None => self,
        }
    }
}

/// No terminal decoration.
pub const NO_STYLE: Style = Style {
    color: None,
    weight: Weight::Normal,
    underline: false,
    padding_right: 0,
    width: None,
};

/// Faint text.
pub const FAINT: Style = NO_STYLE.faint();
/// Red text.
pub const RED: Style = NO_STYLE.foreground(Color::Red);
/// Green text.
pub const GREEN: Style = NO_STYLE.foreground(Color::Green);
/// Yellow text.
pub const YELLOW: Style = NO_STYLE.foreground(Color::Yellow);
/// Bold text.
pub const BOLD: Style = NO_STYLE.bold();
/// Bold red text.
pub const BOLD_RED: Style = RED.bold();
/// Bold green text.
pub const BOLD_GREEN: Style = GREEN.bold();
/// Bold yellow text.
pub const BOLD_YELLOW: Style = YELLOW.bold();
/// Bold service, machine, and context names in palette color 152.
pub const NAME_STYLE: Style = NO_STYLE.bold().foreground(Color::AnsiValue(152));
/// Underlined bright-blue URLs.
pub const URL_STYLE: Style = NO_STYLE.underline(true).foreground(Color::Blue);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_style_is_byte_preserving() {
        let input = "plain\n界\x1b[31mkept\x1b[0m";
        assert_eq!(NO_STYLE.render(input), input);
    }

    #[test]
    fn styles_emit_expected_attributes() {
        let bold_yellow = BOLD_YELLOW.render("WARNING");
        assert!(bold_yellow.contains("WARNING"));
        assert!(bold_yellow.starts_with("\x1b["));
        assert!(bold_yellow.ends_with("\x1b[0m"));

        let faint = FAINT.render(":");
        assert!(faint.contains("\x1b[2m"));
        assert!(faint.contains(':'));

        let url = URL_STYLE.render("https://getployz.com");
        assert!(url.starts_with("\x1b[4;"));
        assert!(url.contains("https://getployz.com"));
    }

    #[test]
    fn padding_and_width_use_display_cells() {
        assert_eq!(strip_sgr(&NO_STYLE.padding_right(3).render("界")), "界   ");
        assert_eq!(strip_sgr(&NO_STYLE.width(4).render("x")), "x   ");
    }

    fn strip_sgr(value: &str) -> String {
        let mut out = String::new();
        let mut chars = value.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }
}
