//! Version and build metadata for Ployz binaries.

#[cfg(test)]
mod build_support;

use std::fmt::{self, Write};

pub const WEBSITE_URL: &str = "https://ployz.run";
pub const DOCS_URL: &str = "https://ployz.run/docs";
pub const DISCORD_URL: &str = "https://ployz.run/discord";
pub const DEVELOPMENT_VERSION: &str = "999.0.0-dev";

const UNKNOWN: &str = "unknown";

/// Version and build metadata exposed by Ployz version commands.
///
/// `go_version` deliberately retains the oracle's caller-facing field name.
/// Its value identifies the Rust compiler that built this binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub version: &'static str,
    pub git_commit: &'static str,
    pub git_state: &'static str,
    pub build_date: String,
    pub built_by: &'static str,
    pub go_version: &'static str,
    pub platform: String,
}

impl Info {
    /// Returns the indented JSON representation used by the version commands.
    #[must_use]
    pub fn json_string(&self) -> String {
        let fields = [
            ("Version", self.version),
            ("GitCommit", self.git_commit),
            ("GitState", self.git_state),
            ("BuildDate", self.build_date.as_str()),
            ("BuiltBy", self.built_by),
            ("GoVersion", self.go_version),
            ("Platform", self.platform.as_str()),
        ];

        let mut json = String::from("{\n");
        for (index, (name, value)) in fields.into_iter().enumerate() {
            json.push_str("  \"");
            json.push_str(name);
            json.push_str("\": ");
            push_json_string(&mut json, value);
            if index + 1 != fields.len() {
                json.push(',');
            }
            json.push('\n');
        }
        json.push('}');
        json
    }
}

impl fmt::Display for Info {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut tabbed = String::new();
        writeln!(tabbed, "Version:\t{}", self.version).expect("writing to a String");
        writeln!(tabbed, "Git commit:\t{}", self.git_commit).expect("writing to a String");
        writeln!(tabbed, "Git state:\t{}", self.git_state).expect("writing to a String");
        writeln!(tabbed, "Build date:\t{}", self.build_date).expect("writing to a String");
        writeln!(tabbed, "Built by:\t{}", self.built_by).expect("writing to a String");
        writeln!(tabbed, "Go version:\t{}", self.go_version).expect("writing to a String");
        writeln!(tabbed, "Platform:\t{}", self.platform).expect("writing to a String");
        formatter.write_str(&expand_tabs(&tabbed))
    }
}

/// Returns the semver-compatible version string used for compatibility checks,
/// machine reports, and metrics labels.
#[must_use]
pub fn version() -> &'static str {
    nonempty(option_env!("PLOYZ_VERSION")).unwrap_or(DEVELOPMENT_VERSION)
}

/// Returns immutable version and build metadata for the current binary.
#[must_use]
pub fn get_info() -> Info {
    Info {
        version: version(),
        git_commit: nonempty(option_env!("PLOYZ_GIT_COMMIT"))
            .or_else(|| nonempty(option_env!("VERGEN_GIT_SHA")))
            .unwrap_or(UNKNOWN),
        git_state: git_state(
            option_env!("PLOYZ_GIT_DIRTY"),
            option_env!("VERGEN_GIT_DIRTY"),
        ),
        build_date: nonempty(option_env!("PLOYZ_BUILD_DATE"))
            .map(str::to_owned)
            .or_else(|| {
                option_env!("VERGEN_GIT_COMMIT_TIMESTAMP").and_then(normalize_git_timestamp)
            })
            .unwrap_or_else(|| UNKNOWN.to_owned()),
        built_by: nonempty(option_env!("PLOYZ_BUILT_BY")).unwrap_or(UNKNOWN),
        go_version: option_env!("PLOYZ_RUSTC_VERSION").unwrap_or(UNKNOWN),
        platform: platform_for(std::env::consts::OS, std::env::consts::ARCH),
    }
}

fn nonempty(value: Option<&'static str>) -> Option<&'static str> {
    value.filter(|value| !value.is_empty())
}

fn git_state(injected: Option<&str>, generated: Option<&str>) -> &'static str {
    let selected = match injected {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => match generated {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => None,
        },
    };

    match selected {
        Some(true) => "dirty",
        Some(false) => "clean",
        None => UNKNOWN,
    }
}

fn normalize_git_timestamp(timestamp: &str) -> Option<String> {
    if timestamp.len() < 20 {
        return None;
    }

    let date_time = timestamp.get(..19)?;
    let bytes = date_time.as_bytes();
    if !matches!(bytes, [
        y0, y1, y2, y3, b'-', m0, m1, b'-', d0, d1, b'T', h0, h1, b':', n0, n1, b':', s0, s1
        ] if [y0, y1, y2, y3, m0, m1, d0, d1, h0, h1, n0, n1, s0, s1]
            .into_iter()
            .all(u8::is_ascii_digit))
    {
        return None;
    }

    let year = parse_decimal(&bytes[0..4]);
    let month = parse_decimal(&bytes[5..7]);
    let day = parse_decimal(&bytes[8..10]);
    let hour = parse_decimal(&bytes[11..13]);
    let minute = parse_decimal(&bytes[14..16]);
    let second = parse_decimal(&bytes[17..19]);
    if day == 0 || day > days_in_month(year, month)? || hour >= 24 || minute >= 60 || second >= 60 {
        return None;
    }

    let (fraction, zone) = split_fraction_and_zone(timestamp.get(19..)?)?;
    let valid_fraction = fraction.is_empty()
        || (matches!(fraction.as_bytes().first(), Some(b'.' | b','))
            && fraction.len() > 1
            && fraction.as_bytes()[1..].iter().all(u8::is_ascii_digit));
    if !valid_fraction {
        return None;
    }

    let offset_minutes = parse_zone_offset(zone)?;
    let (year, month, day, hour, minute) = to_utc(year, month, day, hour, minute, offset_minutes)?;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}"
    ))
}

fn split_fraction_and_zone(suffix: &str) -> Option<(&str, &str)> {
    if suffix.ends_with('Z') {
        return Some((suffix.get(..suffix.len() - 1)?, "Z"));
    }

    let zone_start = suffix.rfind(['+', '-'])?;
    Some((suffix.get(..zone_start)?, suffix.get(zone_start..)?))
}

fn parse_zone_offset(zone: &str) -> Option<i32> {
    if zone == "Z" {
        return Some(0);
    }
    let bytes = zone.as_bytes();
    if !matches!(bytes, [sign @ (b'+' | b'-'), h0, h1, b':', m0, m1]
        if [h0, h1, m0, m1].into_iter().all(u8::is_ascii_digit)
            && (*sign == b'+' || *sign == b'-'))
    {
        return None;
    }
    let hours = i32::try_from(parse_decimal(&bytes[1..3])).ok()?;
    let minutes = i32::try_from(parse_decimal(&bytes[4..6])).ok()?;
    if hours >= 24 || minutes >= 60 {
        return None;
    }
    let offset = hours * 60 + minutes;
    Some(if bytes[0] == b'+' { offset } else { -offset })
}

fn to_utc(
    mut year: u32,
    mut month: u32,
    mut day: u32,
    hour: u32,
    minute: u32,
    offset_minutes: i32,
) -> Option<(u32, u32, u32, u32, u32)> {
    let local_minutes = i32::try_from(hour * 60 + minute).ok()?;
    let mut utc_minutes = local_minutes - offset_minutes;
    if utc_minutes < 0 {
        (year, month, day) = previous_day(year, month, day)?;
        utc_minutes += 24 * 60;
    } else if utc_minutes >= 24 * 60 {
        (year, month, day) = next_day(year, month, day)?;
        utc_minutes -= 24 * 60;
    }

    Some((
        year,
        month,
        day,
        u32::try_from(utc_minutes / 60).ok()?,
        u32::try_from(utc_minutes % 60).ok()?,
    ))
}

fn previous_day(mut year: u32, mut month: u32, day: u32) -> Option<(u32, u32, u32)> {
    if day > 1 {
        return Some((year, month, day - 1));
    }
    if month > 1 {
        month -= 1;
    } else {
        year = year.checked_sub(1)?;
        month = 12;
    }
    Some((year, month, days_in_month(year, month)?))
}

fn next_day(mut year: u32, mut month: u32, day: u32) -> Option<(u32, u32, u32)> {
    if day < days_in_month(year, month)? {
        return Some((year, month, day + 1));
    }
    if month < 12 {
        month += 1;
    } else {
        year = year.checked_add(1)?;
        month = 1;
    }
    Some((year, month, 1))
}

fn parse_decimal(digits: &[u8]) -> u32 {
    digits
        .iter()
        .fold(0, |value, digit| value * 10 + u32::from(digit - b'0'))
}

fn days_in_month(year: u32, month: u32) -> Option<u32> {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => Some(31),
        4 | 6 | 9 | 11 => Some(30),
        2 if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) => {
            Some(29)
        }
        2 => Some(28),
        _ => None,
    }
}

fn platform_for(os: &str, architecture: &str) -> String {
    format!("{}/{}", go_os(os), go_architecture(architecture))
}

fn go_os(os: &str) -> &str {
    match os {
        "macos" => "darwin",
        os => os,
    }
}

fn go_architecture(architecture: &str) -> &str {
    match architecture {
        "x86" => "386",
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "loongarch64" => "loong64",
        "powerpc64" => "ppc64",
        "powerpc64le" => "ppc64le",
        "wasm32" => "wasm",
        architecture => architecture,
    }
}

#[derive(Debug)]
struct TabCell {
    text: String,
    width: usize,
}

fn expand_tabs(input: &str) -> String {
    let mut output = String::new();
    let mut lines: Vec<Vec<TabCell>> = vec![Vec::new()];
    let mut text = String::new();

    for character in input.chars() {
        if matches!(character, '\t' | '\u{0b}' | '\n' | '\u{0c}') {
            let width = text.chars().count();
            lines.last_mut().expect("one current line").push(TabCell {
                text: std::mem::take(&mut text),
                width,
            });

            if matches!(character, '\n' | '\u{0c}') {
                let one_cell_line = lines.last().is_some_and(|line| line.len() == 1);
                lines.push(Vec::new());
                if character == '\u{0c}' || one_cell_line {
                    format_tabbed_lines(&lines, 0, lines.len(), &mut Vec::new(), &mut output);
                    lines.clear();
                    lines.push(Vec::new());
                }
            }
        } else {
            text.push(character);
        }
    }

    if !text.is_empty() {
        let width = text.chars().count();
        lines
            .last_mut()
            .expect("one current line")
            .push(TabCell { text, width });
    }
    format_tabbed_lines(&lines, 0, lines.len(), &mut Vec::new(), &mut output);
    output
}

fn format_tabbed_lines(
    lines: &[Vec<TabCell>],
    mut line_start: usize,
    line_end: usize,
    widths: &mut Vec<usize>,
    output: &mut String,
) {
    let column = widths.len();
    let mut current = line_start;

    while current < line_end {
        if column >= lines[current].len().saturating_sub(1) {
            current += 1;
            continue;
        }

        write_tabbed_lines(lines, line_start, current, widths, output);
        line_start = current;

        let mut width = 0;
        while current < line_end && column < lines[current].len().saturating_sub(1) {
            width = width.max(lines[current][column].width + 2);
            current += 1;
        }

        widths.push(width);
        format_tabbed_lines(lines, line_start, current, widths, output);
        widths.pop();
        line_start = current;
    }

    write_tabbed_lines(lines, line_start, line_end, widths, output);
}

fn write_tabbed_lines(
    lines: &[Vec<TabCell>],
    line_start: usize,
    line_end: usize,
    widths: &[usize],
    output: &mut String,
) {
    for (line_index, line) in lines.iter().enumerate().take(line_end).skip(line_start) {
        for (column, cell) in line.iter().enumerate() {
            output.push_str(&cell.text);
            if let Some(width) = widths.get(column) {
                for _ in cell.width..*width {
                    output.push(' ');
                }
            }
        }
        if line_index + 1 != lines.len() {
            output.push('\n');
        }
    }
}

fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '<' => output.push_str("\\u003c"),
            '>' => output.push_str("\\u003e"),
            '&' => output.push_str("\\u0026"),
            '\u{2028}' => output.push_str("\\u2028"),
            '\u{2029}' => output.push_str("\\u2029"),
            '\0'..='\u{1f}' => {
                write!(output, "\\u{:04x}", u32::from(character)).expect("writing to a String")
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_info() -> Info {
        Info {
            version: "v1.2.3",
            git_commit: "0123456789abcdef",
            git_state: "dirty",
            build_date: "2026-08-11T01:02:03".to_owned(),
            built_by: "goreleaser",
            go_version: "go1.26.1",
            platform: "linux/amd64".to_owned(),
        }
    }

    #[test]
    fn human_output_matches_tabwriter_layout() {
        assert_eq!(
            fixture_info().to_string(),
            concat!(
                "Version:     v1.2.3\n",
                "Git commit:  0123456789abcdef\n",
                "Git state:   dirty\n",
                "Build date:  2026-08-11T01:02:03\n",
                "Built by:    goreleaser\n",
                "Go version:  go1.26.1\n",
                "Platform:    linux/amd64\n",
            )
        );
    }

    #[test]
    fn json_output_preserves_schema_and_go_escaping() {
        let mut info = fixture_info();
        info.built_by = "quote=\" slash=\\ controls=\n\t\u{1f} html=<&> lines=\u{2028}\u{2029}";
        assert_eq!(
            info.json_string(),
            concat!(
                "{\n",
                "  \"Version\": \"v1.2.3\",\n",
                "  \"GitCommit\": \"0123456789abcdef\",\n",
                "  \"GitState\": \"dirty\",\n",
                "  \"BuildDate\": \"2026-08-11T01:02:03\",\n",
                "  \"BuiltBy\": \"quote=\\\" slash=\\\\ controls=\\n\\t\\u001f html=\\u003c\\u0026\\u003e lines=\\u2028\\u2029\",\n",
                "  \"GoVersion\": \"go1.26.1\",\n",
                "  \"Platform\": \"linux/amd64\"\n",
                "}",
            )
        );
    }

    #[test]
    fn injected_dirty_only_overrides_for_exact_booleans() {
        assert_eq!(git_state(Some("true"), Some("false")), "dirty");
        assert_eq!(git_state(Some("false"), Some("true")), "clean");
        assert_eq!(git_state(Some(""), Some("true")), "dirty");
        assert_eq!(git_state(Some("invalid"), Some("false")), "clean");
        assert_eq!(git_state(Some("TRUE"), None), UNKNOWN);
        assert_eq!(git_state(None, Some("invalid")), UNKNOWN);
        assert_eq!(git_state(None, None), UNKNOWN);
    }

    #[test]
    fn generated_timestamp_is_normalized_or_becomes_unknown() {
        for (input, expected) in [
            ("2026-08-11T01:02:03Z", Some("2026-08-11T01:02:03")),
            (
                "2024-02-29T23:59:59.123456789Z",
                Some("2024-02-29T23:59:59"),
            ),
            (
                "2024-02-29T23:59:59,123456789Z",
                Some("2024-02-29T23:59:59"),
            ),
            (
                "2026-08-11T04:05:06.000000000+02:00",
                Some("2026-08-11T02:05:06"),
            ),
            ("2026-01-01T00:30:00+02:00", Some("2025-12-31T22:30:00")),
            ("2026-12-31T23:30:00-02:00", Some("2027-01-01T01:30:00")),
            ("2023-02-29T01:02:03Z", None),
            ("2026-13-11T01:02:03Z", None),
            ("2026-08-11T24:02:03Z", None),
            ("2026-08-11T01:02:60Z", None),
            ("2026-08-11T01:02:03.Z", None),
            ("2026-08-11T01:02:03+00:00", Some("2026-08-11T01:02:03")),
            ("not a date", None),
            ("", None),
        ] {
            assert_eq!(
                normalize_git_timestamp(input),
                expected.map(str::to_owned),
                "input: {input}"
            );
        }
    }

    #[test]
    fn platforms_use_go_names_expected_by_callers() {
        assert_eq!(platform_for("linux", "x86_64"), "linux/amd64");
        assert_eq!(platform_for("linux", "aarch64"), "linux/arm64");
        assert_eq!(platform_for("macos", "x86_64"), "darwin/amd64");
        assert_eq!(platform_for("macos", "aarch64"), "darwin/arm64");
        assert_eq!(platform_for("windows", "x86"), "windows/386");
        assert_eq!(platform_for("linux", "loongarch64"), "linux/loong64");
        assert_eq!(platform_for("aix", "powerpc64"), "aix/ppc64");
    }

    #[test]
    fn current_info_is_complete() {
        let info = get_info();
        assert!(!info.version.is_empty());
        assert!(!info.git_commit.is_empty());
        assert!(matches!(info.git_state, "dirty" | "clean" | "unknown"));
        assert!(!info.build_date.is_empty());
        assert!(!info.built_by.is_empty());
        assert!(info.go_version.starts_with("rustc "));
        assert!(info.platform.contains('/'));
    }

    #[test]
    fn renamed_public_urls_are_stable() {
        assert_eq!(WEBSITE_URL, "https://ployz.run");
        assert_eq!(DOCS_URL, "https://ployz.run/docs");
        assert_eq!(DISCORD_URL, "https://ployz.run/discord");
    }
}
