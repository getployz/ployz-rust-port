#[cfg(not(target_os = "macos"))]
compile_error!("the macOS NSS probe must execute natively on macOS");

use std::env;
use std::ffi::{CStr, c_char};
use std::fmt::Write as _;
use std::io;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::ptr;

const MAX_BUFFER: usize = 1 << 20;

#[derive(Clone, Debug)]
struct User {
    uid: u64,
    gid: u64,
    username: Vec<u8>,
    name: Vec<u8>,
    home: Vec<u8>,
}

fn c_bytes(value: *const c_char) -> Vec<u8> {
    if value.is_null() {
        Vec::new()
    } else {
        // SAFETY: After a successful reentrant passwd lookup, every non-null
        // string field points to a NUL-terminated value in the live buffer.
        unsafe { CStr::from_ptr(value) }.to_bytes().to_vec()
    }
}

fn initial_size() -> usize {
    // SAFETY: sysconf has no pointer arguments and the selector is valid on
    // the required macOS targets.
    let value = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    match usize::try_from(value) {
        Ok(value @ 1..=MAX_BUFFER) => value,
        _ if value == -1 => 1024,
        _ => MAX_BUFFER,
    }
}

fn build_user(passwd: &libc::passwd) -> User {
    let mut name = c_bytes(passwd.pw_gecos);
    if let Some(comma) = name.iter().position(|byte| *byte == b',') {
        name.truncate(comma);
    }

    User {
        uid: passwd.pw_uid.into(),
        gid: passwd.pw_gid.into(),
        username: c_bytes(passwd.pw_name),
        name,
        home: c_bytes(passwd.pw_dir),
    }
}

fn lookup(
    mut call: impl FnMut(
        *mut libc::passwd,
        *mut c_char,
        libc::size_t,
        *mut *mut libc::passwd,
    ) -> libc::c_int,
) -> io::Result<Option<User>> {
    let mut size = initial_size();

    loop {
        let mut passwd = MaybeUninit::<libc::passwd>::zeroed();
        let passwd_pointer = passwd.as_mut_ptr();
        let mut result = ptr::null_mut();
        let mut buffer = vec![0_u8; size];

        let status = call(
            passwd_pointer,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        );

        if status == libc::ERANGE {
            size = size
                .checked_mul(2)
                .filter(|next| *next <= MAX_BUFFER)
                .ok_or_else(|| io::Error::other("internal buffer exceeds 1048576 bytes"))?;
            continue;
        }
        if status == libc::ENOENT {
            return Ok(None);
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        if result.is_null() {
            return Ok(None);
        }
        if result != passwd_pointer {
            return Err(io::Error::other(
                "native passwd lookup returned an unexpected result pointer",
            ));
        }

        // SAFETY: A successful call with the expected non-null result pointer
        // initialized the supplied passwd value. Its strings are copied while
        // the backing buffer remains live.
        let passwd = unsafe { passwd.assume_init() };
        return Ok(Some(build_user(&passwd)));
    }
}

fn lookup_uid(uid: libc::uid_t) -> io::Result<Option<User>> {
    lookup(|passwd, buffer, length, result| {
        // SAFETY: lookup supplies valid writable out-pointers and a live,
        // exclusive scratch buffer for the duration of this call.
        unsafe { libc::getpwuid_r(uid, passwd, buffer, length, result) }
    })
}

fn lookup_name(name: &[u8]) -> io::Result<Option<User>> {
    let mut terminated = Vec::with_capacity(name.len() + 1);
    terminated.extend_from_slice(name);
    terminated.push(0);

    lookup(|passwd, buffer, length, result| {
        // SAFETY: terminated is NUL-terminated and remains live. lookup
        // supplies the other valid pointers and scratch-buffer length.
        unsafe { libc::getpwnam_r(terminated.as_ptr().cast(), passwd, buffer, length, result) }
    })
}

fn required(result: io::Result<Option<User>>, operation: &str) -> io::Result<User> {
    result?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{operation}: native directory record not found"),
        )
    })
}

fn hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}")
            .expect("writing hexadecimal bytes to a String cannot fail");
    }
    encoded
}

fn emit(operation: &str, user: &User) {
    println!(
        "{operation}\t{}\t{}\t{}\t{}\t{}",
        user.uid,
        user.gid,
        hex(&user.username),
        hex(&user.name),
        hex(&user.home),
    );
}

fn main() -> io::Result<()> {
    let directory_name = env::args_os().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: ployz-macos-nss-probe DIRECTORY_USER",
        )
    })?;

    // SAFETY: getuid has no preconditions and returns the real process UID,
    // matching Go's Darwin os/user Current implementation.
    let real_uid = unsafe { libc::getuid() };
    let current = required(lookup_uid(real_uid), "Current")?;

    // Go Lookup and LookupId return copies of the process-global Current cache
    // when the requested current name or UID matches.
    let current_by_name = current.clone();
    let current_by_id = current.clone();

    let directory_by_name = required(
        lookup_name(directory_name.as_os_str().as_bytes()),
        "Lookup(directory username)",
    )?;
    let directory_uid = libc::uid_t::try_from(directory_by_name.uid).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "directory UID does not fit the native uid_t",
        )
    })?;
    let directory_by_id = required(lookup_uid(directory_uid), "LookupId(directory UID)")?;

    emit("current", &current);
    emit("lookup_current_name", &current_by_name);
    emit("lookup_current_id", &current_by_id);
    emit("lookup_directory_name", &directory_by_name);
    emit("lookup_directory_id", &directory_by_id);

    Ok(())
}
