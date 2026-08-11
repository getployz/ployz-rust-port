//! Best-effort inspection of the Git repository containing a path.

use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::path::Path;
use std::process::Command;

/// A Git commit timestamp, represented as whole seconds from the Unix epoch.
///
/// Git's `%ct` format is a signed integer. Keeping that representation avoids
/// imposing a date-time library on callers and preserves pre-epoch values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitTime {
    unix_seconds: i64,
}

impl CommitTime {
    /// Creates a commit time from Git's Unix timestamp representation.
    #[must_use]
    pub const fn from_unix_seconds(unix_seconds: i64) -> Self {
        Self { unix_seconds }
    }

    /// Returns the signed number of whole seconds from the Unix epoch.
    #[must_use]
    pub const fn unix_seconds(self) -> i64 {
        self.unix_seconds
    }
}

/// Information about the current state of a Git repository.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GitState {
    /// The current commit time, or `None` when the path is not a usable repo.
    pub date: Option<CommitTime>,
    /// Whether tracked or untracked files have uncommitted changes.
    pub is_dirty: bool,
    /// Whether the path belongs to a repository with a resolvable `HEAD`.
    pub is_repo: bool,
    /// The full object ID reported for `HEAD`.
    pub sha: String,
}

impl GitState {
    /// Returns a SHA truncated to `length` bytes.
    ///
    /// A non-positive length, a length greater than the SHA, or a length that
    /// is not a UTF-8 boundary returns the full SHA. Git object IDs are ASCII,
    /// so the boundary case matters only for manually constructed states.
    #[must_use]
    pub fn short_sha(&self, length: i64) -> &str {
        let Ok(length) = usize::try_from(length) else {
            return &self.sha;
        };
        if length == 0 || length > self.sha.len() {
            return &self.sha;
        }

        self.sha.get(..length).unwrap_or(&self.sha)
    }
}

/// An error encountered after a path has been identified as a usable repo.
#[derive(Debug)]
pub struct InspectError {
    state: GitState,
    context: &'static str,
    source: InspectErrorSource,
}

impl InspectError {
    /// Returns the repository state collected before inspection failed.
    #[must_use]
    pub const fn state(&self) -> &GitState {
        &self.state
    }

    /// Consumes the error and returns the repository state collected before it failed.
    #[must_use]
    pub fn into_state(self) -> GitState {
        self.state
    }
}

impl fmt::Display for InspectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.context, self.source)
    }
}

impl Error for InspectError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug)]
enum InspectErrorSource {
    Command(CommandError),
    Timestamp(TimestampParseError),
}

#[derive(Debug)]
struct TimestampParseError {
    input: String,
    source: std::num::ParseIntError,
}

impl fmt::Display for TimestampParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.source.kind() {
            std::num::IntErrorKind::PosOverflow | std::num::IntErrorKind::NegOverflow => {
                "value out of range"
            }
            _ => "invalid syntax",
        };
        write!(
            formatter,
            "strconv.ParseInt: parsing {:?}: {reason}",
            self.input
        )
    }
}

impl Error for TimestampParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

impl fmt::Display for InspectErrorSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command(error) => error.fmt(formatter),
            Self::Timestamp(error) => error.fmt(formatter),
        }
    }
}

impl Error for InspectErrorSource {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Command(error) => Some(error),
            Self::Timestamp(error) => Some(error),
        }
    }
}

impl Error for CommandError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Start(error) => Some(error),
            Self::Failed(_) => None,
        }
    }
}

#[derive(Debug)]
enum CommandError {
    Start(io::Error),
    Failed(Vec<u8>),
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start(error) => error.fmt(formatter),
            Self::Failed(stderr) => {
                write!(
                    formatter,
                    "git command failed: {}",
                    String::from_utf8_lossy(stderr)
                )
            }
        }
    }
}

/// Inspects the Git repository state from `dir`.
///
/// A missing `git` executable, a path outside a repository, and an initialized
/// repository without a commit all return the default state without an error.
/// Failures while reading metadata from a repository with a valid `HEAD` are
/// returned with the failed operation as context.
pub fn inspect_git_state(dir: impl AsRef<Path>) -> Result<GitState, InspectError> {
    inspect_git_state_with(dir.as_ref(), GitProgram::new(OsStr::new("git")))
}

#[derive(Clone, Copy)]
struct GitProgram<'a> {
    executable: &'a OsStr,
    prefix_args: &'a [&'a OsStr],
}

impl<'a> GitProgram<'a> {
    const fn new(executable: &'a OsStr) -> Self {
        Self {
            executable,
            prefix_args: &[],
        }
    }
}

fn inspect_git_state_with(dir: &Path, git: GitProgram<'_>) -> Result<GitState, InspectError> {
    let mut state = GitState {
        is_repo: git_command(dir, git, ["rev-parse", "--git-dir"]).is_ok(),
        ..GitState::default()
    };
    if !state.is_repo {
        return Ok(state);
    }

    let Ok(sha) = git_command(dir, git, ["rev-parse", "--verify", "HEAD"]) else {
        state.is_repo = false;
        return Ok(state);
    };
    state.sha = String::from_utf8_lossy(&sha).trim().to_owned();

    let timestamp = match git_command(dir, git, ["log", "-1", "--format=%ct"]) {
        Ok(timestamp) => timestamp,
        Err(error) => {
            return Err(inspect_error("get current commit timestamp", error, state));
        }
    };
    let timestamp = String::from_utf8_lossy(&timestamp);
    let timestamp = timestamp.trim();
    let unix_seconds = match timestamp.parse() {
        Ok(unix_seconds) => unix_seconds,
        Err(error) => {
            return Err(InspectError {
                state,
                context: "parse current commit timestamp",
                source: InspectErrorSource::Timestamp(TimestampParseError {
                    input: timestamp.to_owned(),
                    source: error,
                }),
            });
        }
    };
    state.date = Some(CommitTime::from_unix_seconds(unix_seconds));

    let status = match git_command(dir, git, ["status", "--porcelain"]) {
        Ok(status) => status,
        Err(error) => return Err(inspect_error("check git status", error, state)),
    };
    state.is_dirty = !String::from_utf8_lossy(&status).trim().is_empty();

    Ok(state)
}

fn inspect_error(context: &'static str, error: CommandError, state: GitState) -> InspectError {
    InspectError {
        state,
        context,
        source: InspectErrorSource::Command(error),
    }
}

fn git_command<const N: usize>(
    dir: &Path,
    git: GitProgram<'_>,
    args: [&str; N],
) -> Result<Vec<u8>, CommandError> {
    let output = Command::new(git.executable)
        .args(git.prefix_args)
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(CommandError::Start)?;

    if !output.status.success() {
        return Err(CommandError::Failed(output.stderr));
    }

    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_ID: AtomicU64 = AtomicU64::new(0);
    const COMMIT_SECONDS: i64 = 1_740_000_000;
    const FAKE_SHA: &str = "1234567890abcdef1234567890abcdef12345678";

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(test_name: &str) -> Self {
            let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ployz-gitutil-{}-{test_name}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create temporary test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove temporary test directory");
        }
    }

    fn git(dir: &Path, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .expect("start git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    fn init_repo(test_name: &str) -> TempDir {
        let dir = TempDir::new(test_name);
        git(dir.path(), &["init", "--quiet", "--object-format=sha1"]);
        git(dir.path(), &["config", "user.email", "test@example.com"]);
        git(dir.path(), &["config", "user.name", "Test User"]);
        dir
    }

    fn commit_file(dir: &Path) {
        fs::write(dir.join("tracked.txt"), "initial content").expect("write tracked file");
        git(dir, &["add", "tracked.txt"]);
        let timestamp = format!("@{COMMIT_SECONDS} +0000");
        let output = Command::new("git")
            .args(["commit", "--quiet", "-m", "initial commit"])
            .current_dir(dir)
            .env("GIT_AUTHOR_DATE", &timestamp)
            .env("GIT_COMMITTER_DATE", &timestamp)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .expect("start git commit");
        assert!(
            output.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn git_not_available_returns_default_state() {
        let dir = TempDir::new("git-not-available");
        let state = inspect_git_state_with(
            dir.path(),
            GitProgram::new(OsStr::new("git-that-does-not-exist")),
        )
        .expect("missing git is not an inspection error");

        assert_eq!(state, GitState::default());
        assert_eq!(state.short_sha(7), "");
    }

    #[test]
    fn non_repository_returns_default_state() {
        let dir = TempDir::new("non-repository");

        assert_eq!(inspect_git_state(dir.path()).unwrap(), GitState::default());
    }

    #[test]
    fn nonexistent_directory_returns_default_state() {
        let dir = TempDir::new("nonexistent-directory");
        let nonexistent = dir.path().join("missing");

        assert_eq!(inspect_git_state(nonexistent).unwrap(), GitState::default());
    }

    #[test]
    fn empty_repository_is_treated_as_a_non_repository() {
        let dir = init_repo("empty-repository");

        assert_eq!(inspect_git_state(dir.path()).unwrap(), GitState::default());
    }

    #[test]
    fn clean_repository_reports_git_command_values() {
        let dir = init_repo("clean-repository");
        commit_file(dir.path());

        let state = inspect_git_state(dir.path()).unwrap();
        let expected_sha =
            String::from_utf8(git(dir.path(), &["rev-parse", "--verify", "HEAD"])).unwrap();

        assert!(state.is_repo);
        assert!(!state.is_dirty);
        assert_eq!(state.sha, expected_sha.trim());
        assert_eq!(state.sha.len(), 40);
        assert_eq!(
            state.date,
            Some(CommitTime::from_unix_seconds(COMMIT_SECONDS))
        );
        assert_eq!(state.short_sha(7), &state.sha[..7]);
    }

    #[test]
    fn modified_tracked_file_makes_repository_dirty() {
        let dir = init_repo("modified-file");
        commit_file(dir.path());
        fs::write(dir.path().join("tracked.txt"), "modified").expect("modify tracked file");

        assert!(inspect_git_state(dir.path()).unwrap().is_dirty);
    }

    #[test]
    fn untracked_file_makes_repository_dirty() {
        let dir = init_repo("untracked-file");
        commit_file(dir.path());
        fs::write(dir.path().join("untracked.txt"), "untracked").expect("write untracked file");

        assert!(inspect_git_state(dir.path()).unwrap().is_dirty);
    }

    #[test]
    fn nested_directory_uses_containing_repository() {
        let dir = init_repo("nested-directory");
        commit_file(dir.path());
        let nested = dir.path().join("one/two");
        fs::create_dir_all(&nested).expect("create nested directories");

        let state = inspect_git_state(&nested).unwrap();

        assert!(state.is_repo);
        assert_eq!(state.date.unwrap().unix_seconds(), COMMIT_SECONDS);
        assert!(!state.is_dirty);
    }

    #[test]
    fn short_sha_preserves_oracle_length_edges() {
        let state = GitState {
            sha: "1234567890abcdef1234567890abcdef12345678".to_owned(),
            ..GitState::default()
        };

        assert_eq!(state.short_sha(7), "1234567");
        assert_eq!(state.short_sha(10), "1234567890");
        assert_eq!(state.short_sha(40), state.sha);
        assert_eq!(state.short_sha(50), state.sha);
        assert_eq!(state.short_sha(-42), state.sha);
        assert_eq!(state.short_sha(0), state.sha);
        assert_eq!(GitState::default().short_sha(7), "");
    }

    #[cfg(unix)]
    fn fake_git(test_name: &str, log_result: &str, status_result: &str) -> TempDir {
        let dir = TempDir::new(test_name);
        let script = format!(
            r#"#!/bin/sh
case "$*" in
  "rev-parse --git-dir") printf '.git\n' ;;
  "rev-parse --verify HEAD") printf '1234567890abcdef1234567890abcdef12345678\n' ;;
  "log -1 --format=%ct") {log_result} ;;
  "status --porcelain") {status_result} ;;
  *) printf 'unexpected arguments: %s\n' "$*" >&2; exit 99 ;;
esac
"#
        );
        let program = dir.path().join("git");
        fs::write(&program, script).expect("write fake git");
        dir
    }

    #[cfg(unix)]
    fn inspect_with_fake_git(fake: &TempDir) -> Result<GitState, InspectError> {
        let script = fake.path().join("git");
        inspect_git_state_with(
            fake.path(),
            GitProgram {
                executable: OsStr::new("/bin/sh"),
                prefix_args: &[script.as_os_str()],
            },
        )
    }

    #[cfg(unix)]
    #[test]
    fn timestamp_command_failure_includes_operation_and_stderr() {
        let fake = fake_git(
            "timestamp-command-failure",
            "printf 'log exploded\\n' >&2; exit 12",
            "exit 0",
        );

        let error = inspect_with_fake_git(&fake).expect_err("failed git log must be reported");

        assert_eq!(
            error.to_string(),
            "get current commit timestamp: git command failed: log exploded\n"
        );
        assert_eq!(
            error.state(),
            &GitState {
                is_repo: true,
                sha: FAKE_SHA.to_owned(),
                ..GitState::default()
            }
        );
        let source = error.source().expect("operation must wrap command failure");
        assert_eq!(source.to_string(), "git command failed: log exploded\n");
        assert!(source.source().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn malformed_timestamp_includes_parse_context() {
        let fake = fake_git(
            "malformed-timestamp",
            "printf 'not-a-timestamp\\n'",
            "exit 0",
        );

        let error =
            inspect_with_fake_git(&fake).expect_err("malformed commit time must be reported");

        assert_eq!(
            error.to_string(),
            "parse current commit timestamp: strconv.ParseInt: parsing \"not-a-timestamp\": invalid syntax"
        );
        assert_eq!(error.state().sha, FAKE_SHA);
        assert!(error.state().is_repo);
        assert_eq!(error.state().date, None);
        assert!(error.source().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn signed_pre_epoch_timestamp_is_preserved() {
        let fake = fake_git("pre-epoch-timestamp", "printf '%s\\n' -42", "exit 0");

        let state = inspect_with_fake_git(&fake).expect("signed timestamp must parse");

        assert_eq!(state.date.unwrap().unix_seconds(), -42);
    }

    #[cfg(unix)]
    #[test]
    fn out_of_range_timestamp_matches_oracle_error_context() {
        let fake = fake_git(
            "out-of-range-timestamp",
            "printf '9223372036854775808\\n'",
            "exit 0",
        );

        let error =
            inspect_with_fake_git(&fake).expect_err("out-of-range commit time must be reported");

        assert_eq!(
            error.to_string(),
            "parse current commit timestamp: strconv.ParseInt: parsing \"9223372036854775808\": value out of range"
        );
        assert_eq!(error.state().sha, FAKE_SHA);
        assert!(error.state().is_repo);
        assert_eq!(error.state().date, None);
    }

    #[cfg(unix)]
    #[test]
    fn status_failure_includes_operation_and_stderr() {
        let fake = fake_git(
            "status-command-failure",
            "printf '1740000000\\n'",
            "printf 'status exploded\\n' >&2; exit 13",
        );

        let error = inspect_with_fake_git(&fake).expect_err("failed git status must be reported");

        assert_eq!(
            error.to_string(),
            "check git status: git command failed: status exploded\n"
        );
        assert_eq!(error.state().sha, FAKE_SHA);
        assert!(error.state().is_repo);
        assert_eq!(
            error.state().date,
            Some(CommitTime::from_unix_seconds(COMMIT_SECONDS))
        );
        assert!(!error.state().is_dirty);
        let source = error.source().expect("operation must wrap command failure");
        assert_eq!(source.to_string(), "git command failed: status exploded\n");
        assert!(source.source().is_some());
    }
}
