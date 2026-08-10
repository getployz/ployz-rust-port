use std::env;
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use anyhow::Result;
use nix::unistd::{Gid, Group, Uid, User, chown as os_chown};

use crate::error::context;

#[derive(Debug)]
struct PathOperationError {
    operation: &'static str,
    path: PathBuf,
    source: nix::errno::Errno,
}

impl std::fmt::Display for PathOperationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} {}: {}",
            self.operation,
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for PathOperationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

pub fn expand_home_dir(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() {
        return path.to_owned();
    }
    if bytes[0] == b'~' {
        let Some(home) = env::var_os("HOME") else {
            return path.to_owned();
        };
        if home.is_empty() {
            return path.to_owned();
        }
        let mut expanded = home.as_bytes().to_vec();
        expanded.extend_from_slice(&bytes[1..]);
        return PathBuf::from(OsString::from_vec(expanded));
    }
    path.to_owned()
}

/// lookup_uid_gid returns the user and group IDs for the given username.
pub fn lookup_uid_gid(username: &str) -> Result<(isize, isize)> {
    let user = User::from_name(username)
        .map_err(|err| context(format!("lookup user {username:?}"), err))?
        .ok_or_else(|| {
            anyhow::anyhow!("lookup user {username:?}: user: unknown user {username}")
        })?;
    let uid_string = user.uid.as_raw().to_string();
    let uid = uid_string.parse::<isize>().map_err(|err| {
        context(
            format!("parse {username:?} user ID (UID) {uid_string:?}"),
            err,
        )
    })?;
    let gid_string = user.gid.as_raw().to_string();
    let gid = gid_string.parse::<isize>().map_err(|err| {
        context(
            format!("parse {username:?} user group ID (GID) {gid_string:?}"),
            err,
        )
    })?;
    Ok((uid, gid))
}

pub fn chown(path: impl AsRef<Path>, username: &str, group: &str) -> Result<()> {
    let path = path.as_ref();
    let mut uid = None;
    let mut gid = None;
    if !username.is_empty() {
        let user = User::from_name(username)
            .map_err(|err| context(format!("lookup user {username:?}"), err))?
            .ok_or_else(|| {
                anyhow::anyhow!("lookup user {username:?}: user: unknown user {username}")
            })?;
        uid = Some(Uid::from_raw(user.uid.as_raw()));
    }

    if !group.is_empty() {
        let found_group = Group::from_name(group)
            .map_err(|err| context(format!("lookup group {group:?}"), err))?
            .ok_or_else(|| {
                anyhow::anyhow!("lookup group {group:?}: group: unknown group {group}")
            })?;
        gid = Some(Gid::from_raw(found_group.gid.as_raw()));
    }

    os_chown(path, uid, gid).map_err(|source| {
        context(
            format!("chown {path:?}"),
            PathOperationError {
                operation: "chown",
                path: path.to_owned(),
                source,
            },
        )
    })?;
    Ok(())
}

pub fn exists(path: impl AsRef<Path>) -> bool {
    std::fs::metadata(path).is_ok()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::sync::{Mutex, MutexGuard};

    use super::*;

    static HOME_LOCK: Mutex<()> = Mutex::new(());

    struct HomeGuard {
        previous: Option<OsString>,
        _lock: MutexGuard<'static, ()>,
    }

    impl HomeGuard {
        fn set(value: impl AsRef<OsStr>) -> Self {
            let lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
            let previous = env::var_os("HOME");
            // SAFETY: HOME mutations in this module are serialized and restored by Drop.
            unsafe { env::set_var("HOME", value) };
            Self {
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => {
                    // SAFETY: the HOME lock is held until this guard finishes dropping.
                    unsafe { env::set_var("HOME", value) };
                }
                None => {
                    // SAFETY: the HOME lock is held until this guard finishes dropping.
                    unsafe { env::remove_var("HOME") };
                }
            }
        }
    }

    #[test]
    fn expand_home_dir_cases() {
        assert_eq!(expand_home_dir(""), Path::new(""), "empty");
        assert_eq!(expand_home_dir("/path"), Path::new("/path"), "no home");

        let _home = HomeGuard::set("/home/user");
        assert_eq!(
            expand_home_dir("~/path"),
            Path::new("/home/user/path"),
            "home"
        );
    }

    #[test]
    fn paths_preserve_non_utf8_bytes() {
        let home = OsString::from_vec(b"/home/\xff".to_vec());
        let _home = HomeGuard::set(&home);
        let path = PathBuf::from(OsString::from_vec(b"~/file-\xfe".to_vec()));

        assert_eq!(
            expand_home_dir(&path).as_os_str().as_bytes(),
            b"/home/\xff/file-\xfe"
        );
        assert!(!exists(&path));
    }

    #[test]
    fn chown_error_preserves_display_and_source() {
        let path = Path::new("/definitely/not/a/ployz/path");
        let error = chown(path, "", "").unwrap_err();

        assert!(error.to_string().starts_with(
            "chown \"/definitely/not/a/ployz/path\": chown /definitely/not/a/ployz/path: "
        ));
        assert!(error.to_string().contains(": ENOENT:"));
        assert!(
            error
                .chain()
                .any(|source| source.is::<PathOperationError>())
        );
        assert!(error.chain().any(|source| source.is::<nix::errno::Errno>()));
    }
}
