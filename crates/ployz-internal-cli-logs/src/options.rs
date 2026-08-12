use std::error::Error;
use std::fmt;
use std::num::{IntErrorKind, ParseIntError};

use clap::Args;

/// Shared options for service and system-service log commands.
///
/// Compose files are carried for the service command but deliberately skipped
/// by this shared argument group: commands that support `--file` define it in
/// their own option group and copy the parsed values here.
#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct Options {
    #[arg(skip)]
    pub files: Vec<String>,

    /// Continually stream new logs.
    #[arg(short, long, help = "Continually stream new logs.")]
    pub follow: bool,

    /// Filter by machine name or ID; repeat or delimit values with commas.
    #[arg(
        short,
        long = "machine",
        value_name = "MACHINE",
        value_delimiter = ',',
        help = "Filter logs by machine name or ID. Can be specified multiple times or as a comma-separated list."
    )]
    pub machines: Vec<String>,

    /// Show logs generated on or after this timestamp.
    #[arg(
        long,
        default_value = "",
        help = "Show logs generated on or after the given timestamp. Accepts relative duration, RFC 3339 date, or Unix timestamp.\nExamples:\n  --since 2m30s                      Relative duration (2 minutes 30 seconds ago)\n  --since 1h                         Relative duration (1 hour ago)\n  --since 2025-11-24                 RFC 3339 date only (midnight using local timezone)\n  --since 2024-05-14T22:50:00        RFC 3339 date/time using local timezone\n  --since 2024-01-31T10:30:00Z       RFC 3339 date/time in UTC\n  --since 1763953966                 Unix timestamp (seconds since January 1, 1970)"
    )]
    pub since: String,

    /// Number of recent lines per replica, or `all`.
    #[arg(
        short = 'n',
        long,
        default_value = "100",
        allow_hyphen_values = true,
        help = "Show the most recent logs and limit the number of lines shown per replica. Use 'all' to show all logs."
    )]
    pub tail: String,

    /// Show logs generated before this timestamp.
    #[arg(
        long,
        default_value = "",
        help = "Show logs generated before the given timestamp. Accepts relative duration, RFC 3339 date, or Unix timestamp.\nSee --since for examples."
    )]
    pub until: String,

    /// Print timestamps in UTC instead of the local time zone.
    #[arg(long, help = "Print timestamps in UTC instead of local timezone.")]
    pub utc: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            follow: false,
            machines: Vec::new(),
            since: String::new(),
            tail: "100".to_owned(),
            until: String::new(),
            utc: false,
        }
    }
}

/// An invalid value supplied to `--tail`.
#[derive(Debug)]
pub struct TailError {
    value: String,
    source: ParseIntError,
}

impl TailError {
    /// Returns the rejected option value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for TailError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.source.kind() {
            IntErrorKind::Empty | IntErrorKind::InvalidDigit => "invalid syntax",
            IntErrorKind::PosOverflow | IntErrorKind::NegOverflow => "value out of range",
            _ => {
                return write!(
                    formatter,
                    "invalid --tail value '{}': {}",
                    self.value, self.source
                );
            }
        };
        write!(
            formatter,
            "invalid --tail value '{}': strconv.Atoi: parsing {:?}: {reason}",
            self.value, self.value
        )
    }
}

impl Error for TailError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// Parses the log-line limit, mapping `all` to the API's unbounded sentinel.
pub fn parse_tail(value: &str) -> Result<isize, TailError> {
    if value == "all" {
        return Ok(-1);
    }

    value.parse().map_err(|source| TailError {
        value: value.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::*;

    #[derive(Debug, Parser)]
    struct Command {
        #[command(flatten)]
        logs: Options,
    }

    #[test]
    fn defaults_match_shared_go_flags() {
        let parsed = Command::try_parse_from(["uc"]).unwrap().logs;

        assert_eq!(parsed, Options::default());
        assert_eq!(parsed.tail, "100");
    }

    #[test]
    fn parses_repeated_and_comma_delimited_machines_and_negative_tail() {
        let parsed = Command::try_parse_from([
            "uc",
            "-f",
            "-m",
            "alpha,beta",
            "--machine",
            "gamma",
            "--since",
            "2m30s",
            "-n",
            "-1",
            "--until",
            "2025-11-24",
            "--utc",
        ])
        .unwrap()
        .logs;

        assert!(parsed.follow);
        assert_eq!(parsed.machines, ["alpha", "beta", "gamma"]);
        assert_eq!(parsed.since, "2m30s");
        assert_eq!(parsed.tail, "-1");
        assert_eq!(parsed.until, "2025-11-24");
        assert!(parsed.utc);
    }

    #[test]
    fn shared_group_does_not_expose_service_only_file_option() {
        let error = Command::try_parse_from(["uc", "--file", "compose.yaml"]).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn help_retains_the_oracle_descriptions_and_examples() {
        let help = Command::command().render_long_help().to_string();

        assert!(help.contains("Continually stream new logs."));
        assert!(help.contains("Can be specified multiple times or as a comma-separated list."));
        assert!(help.contains("--since 2m30s"));
        assert!(help.contains("Unix timestamp (seconds since January 1, 1970)"));
        assert!(help.contains("Use 'all' to show all logs."));
        assert!(help.contains("See --since for examples."));
        assert!(help.contains("Print timestamps in UTC instead of local timezone."));
    }

    #[test]
    fn parses_tail_all_signed_values_and_rejects_other_text() {
        assert_eq!(parse_tail("all").unwrap(), -1);
        assert_eq!(parse_tail("0").unwrap(), 0);
        assert_eq!(parse_tail("+20").unwrap(), 20);
        assert_eq!(parse_tail("-20").unwrap(), -20);

        let error = parse_tail("everything").unwrap_err();
        assert_eq!(error.value(), "everything");
        assert_eq!(
            error.to_string(),
            "invalid --tail value 'everything': strconv.Atoi: parsing \"everything\": invalid syntax"
        );
        assert!(error.source().is_some());

        let overflow = parse_tail("999999999999999999999999999").unwrap_err();
        assert_eq!(
            overflow.to_string(),
            "invalid --tail value '999999999999999999999999999': strconv.Atoi: parsing \"999999999999999999999999999\": value out of range"
        );
    }
}
