#[cfg(target_os = "linux")]
use super::Group;
use super::{LookupError, User};
#[cfg(target_os = "linux")]
use std::ffi::OsStr;
use std::ffi::{CStr, OsString};
use std::io;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr;

const MAX_BUFFER: usize = 1 << 20;

pub(super) fn current_user() -> Result<User, LookupError> {
    // SAFETY: getuid has no preconditions and cannot fail on supported Unix.
    let uid = unsafe { libc::getuid() };
    lookup_uid(uid, uid.to_string())
}

#[cfg(target_os = "linux")]
pub(super) fn lookup_user(name: &OsStr) -> Result<User, LookupError> {
    let requested = name.to_owned();
    let mut c_name = name.as_bytes().to_vec();
    c_name.push(0);
    lookup_passwd(
        libc::_SC_GETPW_R_SIZE_MAX,
        format!("user: lookup username {}", requested.to_string_lossy()),
        |record, buffer, result| {
            // SAFETY: all pointers refer to live, suitably aligned storage; the
            // name has a trailing NUL and the buffer length is exact.
            unsafe {
                libc::getpwnam_r(
                    c_name.as_ptr().cast(),
                    record,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    result,
                )
            }
        },
    )?
    .ok_or(LookupError::UnknownUser(requested))
}

#[cfg(target_os = "linux")]
pub(super) fn lookup_user_id(id: &str) -> Result<User, LookupError> {
    let parsed = id
        .parse::<i64>()
        .map_err(|_| LookupError::InvalidUserId(id.to_owned()))?;
    lookup_uid(parsed as libc::uid_t, parsed.to_string())
}

fn lookup_uid(uid: libc::uid_t, unknown_id: String) -> Result<User, LookupError> {
    lookup_passwd(
        libc::_SC_GETPW_R_SIZE_MAX,
        format!("user: lookup userid {uid}"),
        |record, buffer, result| {
            // SAFETY: all pointers refer to live, suitably aligned storage and
            // the buffer length describes its complete allocation.
            unsafe {
                libc::getpwuid_r(
                    uid,
                    record,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    result,
                )
            }
        },
    )?
    .ok_or(LookupError::UnknownUserId(unknown_id))
}

#[cfg(target_os = "linux")]
pub(super) fn lookup_group(name: &OsStr) -> Result<Group, LookupError> {
    let requested = name.to_owned();
    let mut c_name = name.as_bytes().to_vec();
    c_name.push(0);
    let mut size = initial_size(libc::_SC_GETGR_R_SIZE_MAX);
    loop {
        let mut record = MaybeUninit::<libc::group>::zeroed();
        let mut result = ptr::null_mut();
        let mut buffer = vec![0_u8; size];
        // SAFETY: all pointers refer to live, suitably aligned storage; the
        // name has a trailing NUL and the buffer length is exact.
        let status = unsafe {
            libc::getgrnam_r(
                c_name.as_ptr().cast(),
                record.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE {
            size = grow(size)?;
            continue;
        }
        if status == libc::ENOENT || (status == 0 && result.is_null()) {
            return Err(LookupError::UnknownGroup(requested));
        }
        if status != 0 {
            return Err(LookupError::io(
                format!("user: lookup groupname {}", requested.to_string_lossy()),
                io::Error::from_raw_os_error(status),
            ));
        }
        if result != record.as_mut_ptr() {
            return Err(LookupError::io(
                "user: native group lookup",
                io::Error::other("native group lookup returned an unexpected result pointer"),
            ));
        }
        // SAFETY: status is success and libc returned a non-null result pointer
        // whose string fields point into the still-live buffer.
        return Ok(unsafe { group_to_group(&*result) });
    }
}

fn lookup_passwd(
    sysconf_name: libc::c_int,
    context: String,
    mut call: impl FnMut(*mut libc::passwd, &mut [u8], *mut *mut libc::passwd) -> libc::c_int,
) -> Result<Option<User>, LookupError> {
    let mut size = initial_size(sysconf_name);
    loop {
        let mut record = MaybeUninit::<libc::passwd>::zeroed();
        let mut result = ptr::null_mut();
        let mut buffer = vec![0_u8; size];
        let status = call(record.as_mut_ptr(), &mut buffer, &mut result);
        if status == libc::ERANGE {
            size = grow(size)?;
            continue;
        }
        if status == libc::ENOENT || (status == 0 && result.is_null()) {
            return Ok(None);
        }
        if status != 0 {
            return Err(LookupError::io(
                context,
                io::Error::from_raw_os_error(status),
            ));
        }
        if result != record.as_mut_ptr() {
            return Err(LookupError::io(
                &context,
                io::Error::other("native passwd lookup returned an unexpected result pointer"),
            ));
        }
        // Copy all pointer-backed data before `buffer` is dropped.
        // SAFETY: successful non-null result guarantees a valid record whose
        // pointer-backed fields remain live until the local buffer is dropped.
        let user = unsafe { user_from_passwd(&*result) };
        return Ok(Some(user));
    }
}

unsafe fn user_from_passwd(record: &libc::passwd) -> User {
    // SAFETY: the caller keeps the successful libc result buffer live.
    let username = unsafe { copy_c_string(record.pw_name) };
    // SAFETY: the caller keeps the successful libc result buffer live.
    let mut name = unsafe { copy_c_string(record.pw_gecos) };
    if let Some(index) = name.as_bytes().iter().position(|byte| *byte == b',') {
        name = OsString::from_vec(name.as_bytes()[..index].to_vec());
    }
    // SAFETY: the caller keeps the successful libc result buffer live.
    let home = unsafe { copy_c_string(record.pw_dir) };
    User {
        uid: (record.pw_uid as u64).to_string(),
        gid: (record.pw_gid as u64).to_string(),
        username,
        name,
        home_dir: PathBuf::from(home),
    }
}

#[cfg(target_os = "linux")]
unsafe fn group_to_group(record: &libc::group) -> Group {
    Group {
        gid: (record.gr_gid as u64).to_string(),
        // SAFETY: the caller keeps the successful libc result buffer live.
        name: unsafe { copy_c_string(record.gr_name) },
    }
}

unsafe fn copy_c_string(pointer: *const libc::c_char) -> OsString {
    if pointer.is_null() {
        return OsString::new();
    }
    // SAFETY: caller established that `pointer` is a valid NUL-terminated
    // field in the live libc result buffer.
    OsString::from_vec(unsafe { CStr::from_ptr(pointer) }.to_bytes().to_vec())
}

fn initial_size(name: libc::c_int) -> usize {
    // SAFETY: sysconf accepts these fixed platform constants.
    let value = unsafe { libc::sysconf(name) };
    if value == -1 {
        1024
    } else if value <= 0 || value as u128 > MAX_BUFFER as u128 {
        MAX_BUFFER
    } else {
        value as usize
    }
}

fn grow(size: usize) -> Result<usize, LookupError> {
    let next = size.saturating_mul(2);
    if next == 0 || next > MAX_BUFFER {
        Err(LookupError::BufferLimit)
    } else {
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_rules_match_go() {
        assert_eq!(grow(1024).unwrap(), 2048);
        assert_eq!(grow(MAX_BUFFER / 2).unwrap(), MAX_BUFFER);
        assert!(matches!(
            grow(MAX_BUFFER).unwrap_err(),
            LookupError::BufferLimit
        ));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn native_current_and_named_lookup_agree() {
        let current = current_user().unwrap();
        let named = lookup_user(&current.username).unwrap();
        assert_eq!(current, named);
        assert!(matches!(
            lookup_group(OsStr::new("__ployz_missing_group__")),
            Err(LookupError::UnknownGroup(name))
                if name == OsStr::new("__ployz_missing_group__")
        ));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn embedded_nul_truncates_native_name_query() {
        let current = current_user().unwrap();
        let mut name = current.username.as_bytes().to_vec();
        name.extend_from_slice(b"\0ignored");
        assert_eq!(lookup_user(OsStr::from_bytes(&name)).unwrap(), current);
    }

    #[test]
    fn retry_loop_maps_no_result_errno_and_buffer_ceiling() {
        let missing = lookup_passwd(libc::_SC_GETPW_R_SIZE_MAX, "probe".into(), |_, _, _| {
            libc::ENOENT
        })
        .unwrap();
        assert!(missing.is_none());

        let denied = lookup_passwd(libc::_SC_GETPW_R_SIZE_MAX, "probe".into(), |_, _, _| {
            libc::EACCES
        })
        .unwrap_err();
        assert!(matches!(
            denied,
            LookupError::Io { source, .. }
                if source.raw_os_error() == Some(libc::EACCES)
        ));

        let limited = lookup_passwd(libc::_SC_GETPW_R_SIZE_MAX, "probe".into(), |_, _, _| {
            libc::ERANGE
        })
        .unwrap_err();
        assert!(matches!(limited, LookupError::BufferLimit));
    }
}
