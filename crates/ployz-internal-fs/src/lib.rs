//! Unix filesystem and account operations used by Ployz.
//!
//! The `files` feature reproduces Go's cgo-free Linux account lookup. The
//! `native` feature uses the platform NSS/Open Directory implementation and
//! takes precedence when both features are enabled for workspace-wide checks.

#[cfg(not(unix))]
compile_error!("ployz-internal-fs supports Unix targets only");
#[cfg(not(any(feature = "files", feature = "native")))]
compile_error!("enable either the `files` or `native` account backend");
#[cfg(all(feature = "files", not(feature = "native"), not(target_os = "linux")))]
compile_error!("the `files` account backend is supported only on Linux");

use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

#[cfg(all(feature = "files", not(feature = "native")))]
mod files;
#[cfg(feature = "native")]
mod native;

#[cfg(all(feature = "files", not(feature = "native")))]
use files as backend;
#[cfg(feature = "native")]
use native as backend;

/// A Unix user record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct User {
    pub uid: String,
    pub gid: String,
    pub username: OsString,
    pub name: OsString,
    pub home_dir: PathBuf,
}

/// A Unix group record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Group {
    pub gid: String,
    pub name: OsString,
}

/// Account lookup failure, including typed not-found outcomes.
#[derive(Clone, Debug)]
pub enum LookupError {
    UnknownUser(OsString),
    UnknownUserId(String),
    UnknownGroup(OsString),
    UnknownGroupId(String),
    InvalidUserId(String),
    CurrentEnvironment(Vec<&'static str>),
    BufferLimit,
    Io {
        context: String,
        source: Arc<io::Error>,
    },
}

impl LookupError {
    fn io(context: impl Into<String>, error: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source: Arc::new(error),
        }
    }

    /// Returns true only for the typed unknown-group outcome.
    #[must_use]
    pub fn is_unknown_group(&self) -> bool {
        matches!(self, Self::UnknownGroup(_))
    }
}

impl fmt::Display for LookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownUser(name) => write!(f, "user: unknown user {}", name.to_string_lossy()),
            Self::UnknownUserId(id) => write!(f, "user: unknown userid {id}"),
            Self::UnknownGroup(name) => {
                write!(f, "group: unknown group {}", name.to_string_lossy())
            }
            Self::UnknownGroupId(id) => write!(f, "group: unknown groupid {id}"),
            Self::InvalidUserId(id) => write!(f, "user: invalid userid {id}"),
            Self::CurrentEnvironment(missing) => write!(
                f,
                "user: Current requires cgo or {} set in environment",
                missing.join(", ")
            ),
            Self::BufferLimit => write!(f, "internal buffer exceeds 1048576 bytes"),
            Self::Io { context, source } => write!(f, "{context}: {source}"),
        }
    }
}

impl std::error::Error for LookupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

/// Failure from [`lookup_uid_gid`].
#[derive(Debug)]
pub enum IdError {
    Lookup {
        username: OsString,
        source: LookupError,
    },
    ParseUid {
        username: OsString,
        value: String,
        source: std::num::ParseIntError,
    },
    ParseGid {
        username: OsString,
        value: String,
        source: std::num::ParseIntError,
    },
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lookup { username, source } => {
                write!(f, "lookup user {username:?}: {source}")
            }
            Self::ParseUid {
                username,
                value,
                source,
            } => write!(f, "parse {username:?} user ID (UID) {value:?}: {source}"),
            Self::ParseGid {
                username,
                value,
                source,
            } => write!(
                f,
                "parse {username:?} user group ID (GID) {value:?}: {source}"
            ),
        }
    }
}

impl std::error::Error for IdError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lookup { source, .. } => Some(source),
            Self::ParseUid { source, .. } | Self::ParseGid { source, .. } => Some(source),
        }
    }
}

/// Failure from [`chown`].
#[derive(Debug)]
pub enum ChownError {
    LookupUser {
        username: OsString,
        source: LookupError,
    },
    ParseUid {
        username: OsString,
        value: String,
        source: std::num::ParseIntError,
    },
    LookupGroup {
        group: OsString,
        source: LookupError,
    },
    ParseGid {
        group: OsString,
        value: String,
        source: std::num::ParseIntError,
    },
    Chown {
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for ChownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LookupUser { username, source } => {
                write!(f, "lookup user {username:?}: {source}")
            }
            Self::ParseUid {
                username,
                value,
                source,
            } => write!(f, "parse {username:?} user ID (UID) {value:?}: {source}"),
            Self::LookupGroup { group, source } => {
                write!(f, "lookup group {group:?}: {source}")
            }
            Self::ParseGid {
                group,
                value,
                source,
            } => write!(f, "parse {group:?} group ID (GID) {value:?}: {source}"),
            Self::Chown { path, source } => write!(f, "chown {path:?}: {source}"),
        }
    }
}

impl std::error::Error for ChownError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::LookupUser { source, .. } | Self::LookupGroup { source, .. } => Some(source),
            Self::ParseUid { source, .. } | Self::ParseGid { source, .. } => Some(source),
            Self::Chown { source, .. } => Some(source),
        }
    }
}

/// Replaces a leading `~` with `$HOME`. If `$HOME` is absent or empty, returns
/// the input unchanged, matching `os.UserHomeDir` on Unix.
#[must_use]
pub fn expand_home_dir(path: impl AsRef<Path>) -> PathBuf {
    expand_home_dir_with(path.as_ref(), env::var_os("HOME"))
}

fn expand_home_dir_with(path: &Path, home: Option<OsString>) -> PathBuf {
    let bytes = path.as_os_str().as_bytes();
    if bytes.first() != Some(&b'~') {
        return path.to_owned();
    }
    let Some(home) = home.filter(|value| !value.is_empty()) else {
        return path.to_owned();
    };
    let mut expanded = home;
    expanded.push(OsStr::from_bytes(&bytes[1..]));
    PathBuf::from(expanded)
}

/// Reports whether `stat(2)` succeeds for a path.
#[must_use]
pub fn exists(path: impl AsRef<Path>) -> bool {
    std::fs::metadata(path).is_ok()
}

struct CurrentUserCache(OnceLock<Result<User, LookupError>>);

impl CurrentUserCache {
    const fn new() -> Self {
        Self(OnceLock::new())
    }

    fn get_or_init(
        &self,
        initialize: impl FnOnce() -> Result<User, LookupError>,
    ) -> Result<User, LookupError> {
        self.0.get_or_init(initialize).clone()
    }
}

static CURRENT_USER: CurrentUserCache = CurrentUserCache::new();

/// Returns the real process user, with Go's process-global success/error cache.
pub fn current_user() -> Result<User, LookupError> {
    CURRENT_USER.get_or_init(backend::current_user)
}

/// Looks up a user by name. The cached current record wins on an exact match.
#[cfg(target_os = "linux")]
pub fn lookup_user(name: impl AsRef<OsStr>) -> Result<User, LookupError> {
    let name = name.as_ref();
    if let Ok(current) = current_user()
        && current.username == name
    {
        return Ok(current);
    }
    backend::lookup_user(name)
}

/// Looks up a user by its textual ID.
#[cfg(target_os = "linux")]
pub fn lookup_user_id(id: &str) -> Result<User, LookupError> {
    if let Ok(current) = current_user()
        && current.uid == id
    {
        return Ok(current);
    }
    backend::lookup_user_id(id)
}

/// Looks up a group by name without consulting the current-user cache.
#[cfg(target_os = "linux")]
pub fn lookup_group(name: impl AsRef<OsStr>) -> Result<Group, LookupError> {
    backend::lookup_group(name.as_ref())
}

/// Looks up and parses a user's UID and primary GID as signed 64-bit values.
#[cfg(target_os = "linux")]
pub fn lookup_uid_gid(username: impl AsRef<OsStr>) -> Result<(i64, i64), IdError> {
    let username = username.as_ref();
    let user = lookup_user(username).map_err(|source| IdError::Lookup {
        username: username.to_owned(),
        source,
    })?;
    let uid = user
        .uid
        .parse::<i64>()
        .map_err(|source| IdError::ParseUid {
            username: username.to_owned(),
            value: user.uid.clone(),
            source,
        })?;
    let gid = user
        .gid
        .parse::<i64>()
        .map_err(|source| IdError::ParseGid {
            username: username.to_owned(),
            value: user.gid,
            source,
        })?;
    Ok((uid, gid))
}

/// Changes path ownership. Empty user/group names independently leave that ID
/// unchanged; the syscall still occurs when both are empty.
#[cfg(target_os = "linux")]
pub fn chown(
    path: impl AsRef<Path>,
    username: impl AsRef<OsStr>,
    group: impl AsRef<OsStr>,
) -> Result<(), ChownError> {
    let path = path.as_ref();
    let username = username.as_ref();
    let group = group.as_ref();

    chown_with(
        path,
        username,
        group,
        |name| lookup_user(name),
        |name| lookup_group(name),
        |uid, gid| std::os::unix::fs::chown(path, uid, gid),
    )
}

#[cfg(target_os = "linux")]
fn chown_with(
    path: &Path,
    username: &OsStr,
    group: &OsStr,
    mut find_user: impl FnMut(&OsStr) -> Result<User, LookupError>,
    mut find_group: impl FnMut(&OsStr) -> Result<Group, LookupError>,
    mut perform: impl FnMut(Option<u32>, Option<u32>) -> io::Result<()>,
) -> Result<(), ChownError> {
    let uid = if username.is_empty() {
        None
    } else {
        let user = find_user(username).map_err(|source| ChownError::LookupUser {
            username: username.to_owned(),
            source,
        })?;
        let parsed = user
            .uid
            .parse::<i64>()
            .map_err(|source| ChownError::ParseUid {
                username: username.to_owned(),
                value: user.uid,
                source,
            })?;
        Some(parsed as u32)
    };

    let gid = if group.is_empty() {
        None
    } else {
        let found = find_group(group).map_err(|source| ChownError::LookupGroup {
            group: group.to_owned(),
            source,
        })?;
        let parsed = found
            .gid
            .parse::<i64>()
            .map_err(|source| ChownError::ParseGid {
                group: group.to_owned(),
                value: found.gid,
                source,
            })?;
        Some(parsed as u32)
    };

    loop {
        match perform(uid, gid) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(source) => {
                return Err(ChownError::Chown {
                    path: path.to_owned(),
                    source,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn expands_only_a_leading_tilde() {
        let home = || Some(OsString::from("/home/user"));
        assert_eq!(expand_home_dir_with(Path::new(""), home()), Path::new(""));
        assert_eq!(
            expand_home_dir_with(Path::new("/path"), home()),
            Path::new("/path")
        );
        assert_eq!(
            expand_home_dir_with(Path::new("~/path"), home()),
            Path::new("/home/user/path")
        );
        assert_eq!(
            expand_home_dir_with(Path::new("~other/~"), home()),
            Path::new("/home/userother/~")
        );
        assert_eq!(
            expand_home_dir_with(Path::new("~/path"), None),
            Path::new("~/path")
        );
        assert_eq!(
            expand_home_dir_with(Path::new("~/path"), Some(OsString::new())),
            Path::new("~/path")
        );
    }

    #[test]
    fn expansion_preserves_non_utf8_path_bytes() {
        assert_eq!(
            expand_home_dir_with(
                &PathBuf::from(OsString::from_vec(vec![b'~', b'/', 0xfe])),
                Some(OsString::from_vec(vec![b'/', 0xff])),
            )
            .as_os_str()
            .as_bytes(),
            &[b'/', 0xff, b'/', 0xfe]
        );
    }

    #[test]
    fn exists_uses_following_metadata() {
        let base = env::temp_dir().join(format!("ployz-fs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir(&base).unwrap();
        let file = base.join("file");
        std::fs::write(&file, b"x").unwrap();
        assert!(exists(&file));
        assert!(!exists(base.join("missing")));
        std::os::unix::fs::symlink("missing", base.join("broken")).unwrap();
        assert!(!exists(base.join("broken")));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn current_lookup_is_cloneable_and_concurrent() {
        let mut first = current_user().unwrap();
        let second = current_user().unwrap();
        assert_eq!(first, second);
        first.username.push("-mutated-copy");
        assert_ne!(first, current_user().unwrap());
        let expected = second;
        let workers: Vec<_> = (0..16).map(|_| std::thread::spawn(current_user)).collect();
        for worker in workers {
            assert_eq!(worker.join().unwrap().unwrap(), expected);
        }
    }

    #[test]
    fn cache_is_sticky_for_success_and_error() {
        let success = CurrentUserCache::new();
        let expected = fixture_user("7", "8");
        assert_eq!(
            success.get_or_init(|| Ok(expected.clone())).unwrap(),
            expected
        );
        assert_eq!(
            success
                .get_or_init(|| Err(LookupError::UnknownUserId("changed".into())))
                .unwrap(),
            expected
        );

        let failure = CurrentUserCache::new();
        assert!(matches!(
            failure.get_or_init(|| Err(LookupError::UnknownUserId("first".into()))),
            Err(LookupError::UnknownUserId(id)) if id == "first"
        ));
        assert!(matches!(
            failure.get_or_init(|| Ok(expected)),
            Err(LookupError::UnknownUserId(id)) if id == "first"
        ));
    }

    #[test]
    fn lookup_io_error_preserves_chainable_source() {
        let lookup = LookupError::io(
            "read fixture",
            io::Error::from(io::ErrorKind::PermissionDenied),
        );
        let source = std::error::Error::source(&lookup)
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap();
        assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn concurrent_first_cache_initialization_has_one_winner() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Barrier};

        let cache = Arc::new(CurrentUserCache::new());
        let barrier = Arc::new(Barrier::new(16));
        let initializations = Arc::new(AtomicUsize::new(0));
        let workers: Vec<_> = (0..16)
            .map(|index| {
                let cache = Arc::clone(&cache);
                let barrier = Arc::clone(&barrier);
                let initializations = Arc::clone(&initializations);
                std::thread::spawn(move || {
                    barrier.wait();
                    cache.get_or_init(|| {
                        initializations.fetch_add(1, Ordering::SeqCst);
                        Ok(fixture_user(&index.to_string(), "8"))
                    })
                })
            })
            .collect();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap().unwrap())
            .collect();
        assert_eq!(initializations.load(Ordering::SeqCst), 1);
        assert!(results.iter().all(|user| user == &results[0]));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn lookup_current_name_and_id_match_signed_ids() {
        let current = current_user().unwrap();
        assert_eq!(lookup_user(&current.username).unwrap(), current);
        assert_eq!(lookup_user_id(&current.uid).unwrap(), current);
        let (uid, gid) = lookup_uid_gid(&current.username).unwrap();
        assert_eq!(uid.to_string(), current.uid);
        assert_eq!(gid.to_string(), current.gid);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn chown_calls_the_kernel_when_both_ids_are_omitted() {
        let base = env::temp_dir().join(format!("ployz-fs-chown-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir(&base).unwrap();
        let target = base.join("target");
        std::fs::write(&target, b"x").unwrap();
        let link = base.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        chown(&link, "", "").unwrap();

        let broken = base.join("broken");
        std::os::unix::fs::symlink(base.join("missing"), &broken).unwrap();
        assert!(matches!(
            chown(&broken, "", "").unwrap_err(),
            ChownError::Chown { .. }
        ));
        let nul = PathBuf::from(OsString::from_vec(b"bad\0path".to_vec()));
        let error = chown(&nul, "", "").unwrap_err();
        assert!(matches!(
            error,
            ChownError::Chown {
                source,
                ..
            } if source.kind() == io::ErrorKind::InvalidInput
        ));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn chown_resolves_in_order_and_retries_only_interrupted() {
        use std::cell::{Cell, RefCell};

        let operations = RefCell::new(Vec::new());
        let attempts = Cell::new(0);
        chown_with(
            Path::new("fixture"),
            OsStr::new("user"),
            OsStr::new("group"),
            |name| {
                operations.borrow_mut().push(format!("user:{name:?}"));
                Ok(fixture_user("12", "99"))
            },
            |name| {
                operations.borrow_mut().push(format!("group:{name:?}"));
                Ok(fixture_group("34"))
            },
            |uid, gid| {
                operations
                    .borrow_mut()
                    .push(format!("chown:{uid:?}:{gid:?}"));
                let count = attempts.get();
                attempts.set(count + 1);
                if count == 0 {
                    Err(io::Error::from(io::ErrorKind::Interrupted))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap();
        assert_eq!(
            operations.into_inner(),
            [
                "user:\"user\"",
                "group:\"group\"",
                "chown:Some(12):Some(34)",
                "chown:Some(12):Some(34)",
            ]
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn chown_narrows_ids_and_preserves_all_ones_sentinel() {
        for signed in ["-1", "4294967295", "8589934591", "9223372036854775807"] {
            let observed = std::cell::Cell::new(None);
            chown_with(
                Path::new("fixture"),
                OsStr::new("user"),
                OsStr::new("group"),
                |_| Ok(fixture_user(signed, "0")),
                |_| Ok(fixture_group("7")),
                |uid, gid| {
                    observed.set(Some((uid, gid)));
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(observed.get(), Some((Some(u32::MAX), Some(7))));
        }

        for signed in ["-1", "4294967295", "8589934591", "9223372036854775807"] {
            let observed = std::cell::Cell::new(None);
            chown_with(
                Path::new("fixture"),
                OsStr::new("user"),
                OsStr::new("group"),
                |_| Ok(fixture_user("8", "0")),
                |_| Ok(fixture_group(signed)),
                |uid, gid| {
                    observed.set(Some((uid, gid)));
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(observed.get(), Some((Some(8), Some(u32::MAX))));
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn chown_supports_every_omitted_id_combination() {
        for (username, group, expected) in [
            ("", "", (None, None)),
            ("user", "", (Some(12), None)),
            ("", "group", (None, Some(34))),
            ("user", "group", (Some(12), Some(34))),
        ] {
            let observed = std::cell::Cell::new(None);
            chown_with(
                Path::new("fixture"),
                OsStr::new(username),
                OsStr::new(group),
                |_| Ok(fixture_user("12", "99")),
                |_| Ok(fixture_group("34")),
                |uid, gid| {
                    observed.set(Some((uid, gid)));
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(observed.get(), Some(expected));
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn chown_short_circuits_before_group_and_syscall_on_user_failure() {
        let group_called = std::cell::Cell::new(false);
        let syscall_called = std::cell::Cell::new(false);
        let error = chown_with(
            Path::new("fixture"),
            OsStr::new("missing"),
            OsStr::new("group"),
            |_| Err(LookupError::UnknownUser(OsString::from("missing"))),
            |_| {
                group_called.set(true);
                Ok(fixture_group("7"))
            },
            |_, _| {
                syscall_called.set(true);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(matches!(error, ChownError::LookupUser { .. }));
        assert!(!group_called.get());
        assert!(!syscall_called.get());
    }

    fn fixture_user(uid: &str, gid: &str) -> User {
        User {
            uid: uid.into(),
            gid: gid.into(),
            username: OsString::from("fixture"),
            name: OsString::new(),
            home_dir: PathBuf::from("/fixture"),
        }
    }

    #[cfg(target_os = "linux")]
    fn fixture_group(gid: &str) -> Group {
        Group {
            gid: gid.into(),
            name: OsString::from("fixture"),
        }
    }
}
