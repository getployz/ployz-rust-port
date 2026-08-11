use super::{Group, LookupError, User};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

const PASSWD: &str = "/etc/passwd";
const GROUP: &str = "/etc/group";

pub(super) fn current_user() -> Result<User, LookupError> {
    let uid = rustix::process::getuid().as_raw().to_string();
    current_user_with(uid, lookup_user_id, || {
        (
            rustix::process::getgid().as_raw().to_string(),
            env::var_os("USER"),
            env::var_os("HOME"),
        )
    })
}

fn current_user_with(
    uid: String,
    lookup: impl FnOnce(&str) -> Result<User, LookupError>,
    fallback: impl FnOnce() -> (String, Option<OsString>, Option<OsString>),
) -> Result<User, LookupError> {
    if let Ok(user) = lookup(&uid) {
        return Ok(user);
    }

    let (gid, username, home) = fallback();
    let username = username.filter(|value| !value.is_empty());
    let home = home.filter(|value| !value.is_empty());
    let mut missing = Vec::new();
    if username.is_none() {
        missing.push("$USER");
    }
    if home.is_none() {
        missing.push("$HOME");
    }
    if !missing.is_empty() {
        return Err(LookupError::CurrentEnvironment(missing));
    }

    Ok(User {
        uid,
        gid,
        username: username.expect("validated above"),
        name: OsString::new(),
        home_dir: PathBuf::from(home.expect("validated above")),
    })
}

pub(super) fn lookup_user(name: &OsStr) -> Result<User, LookupError> {
    let file = File::open(PASSWD).map_err(|error| LookupError::io("open /etc/passwd", error))?;
    find_user_by_name(BufReader::new(file), name.as_bytes())
        .map_err(|error| LookupError::io("read /etc/passwd", error))?
        .ok_or_else(|| LookupError::UnknownUser(name.to_owned()))
}

pub(super) fn lookup_user_id(id: &str) -> Result<User, LookupError> {
    let parsed = id
        .parse::<i64>()
        .map_err(|_| LookupError::InvalidUserId(id.to_owned()))?;
    let file = File::open(PASSWD).map_err(|error| LookupError::io("open /etc/passwd", error))?;
    find_user_by_id(BufReader::new(file), id.as_bytes())
        .map_err(|error| LookupError::io("read /etc/passwd", error))?
        .ok_or_else(|| LookupError::UnknownUserId(parsed.to_string()))
}

pub(super) fn lookup_group(name: &OsStr) -> Result<Group, LookupError> {
    let file = File::open(GROUP).map_err(|error| LookupError::io("open /etc/group", error))?;
    find_group_by_name(BufReader::new(file), name.as_bytes())
        .map_err(|error| LookupError::io("read /etc/group", error))?
        .ok_or_else(|| LookupError::UnknownGroup(name.to_owned()))
}

fn find_user_by_name(reader: impl BufRead, name: &[u8]) -> io::Result<Option<User>> {
    read_colon_file(reader, 6, |line| parse_user(line, name, 0))
}

fn find_user_by_id(reader: impl BufRead, id: &[u8]) -> io::Result<Option<User>> {
    read_colon_file(reader, 6, |line| parse_user(line, id, 2))
}

fn find_group_by_name(reader: impl BufRead, name: &[u8]) -> io::Result<Option<Group>> {
    read_colon_file(reader, 3, |line| parse_group(line, name, 0))
}

fn parse_user(line: &[u8], wanted: &[u8], index: usize) -> Option<User> {
    let parts: Vec<_> = line.splitn(7, |byte| *byte == b':').collect();
    if line.iter().filter(|byte| **byte == b':').count() < 6
        || parts[index] != wanted
        || parts[0].is_empty()
        || matches!(parts[0][0], b'+' | b'-')
        || parse_i64(parts[2]).is_none()
        || parse_i64(parts[3]).is_none()
    {
        return None;
    }
    let display = parts[4]
        .split(|byte| *byte == b',')
        .next()
        .unwrap_or_default();
    Some(User {
        uid: String::from_utf8(parts[2].to_vec()).expect("validated decimal"),
        gid: String::from_utf8(parts[3].to_vec()).expect("validated decimal"),
        username: OsString::from_vec(parts[0].to_vec()),
        name: OsString::from_vec(display.to_vec()),
        home_dir: PathBuf::from(OsString::from_vec(parts[5].to_vec())),
    })
}

fn parse_group(line: &[u8], wanted: &[u8], index: usize) -> Option<Group> {
    let parts: Vec<_> = line.splitn(4, |byte| *byte == b':').collect();
    if parts.len() < 4
        || parts[index] != wanted
        || parts[0].is_empty()
        || matches!(parts[0][0], b'+' | b'-')
        || parse_i64(parts[2]).is_none()
    {
        return None;
    }
    Some(Group {
        gid: String::from_utf8(parts[2].to_vec()).expect("validated decimal"),
        name: OsString::from_vec(parts[0].to_vec()),
    })
}

fn parse_i64(value: &[u8]) -> Option<i64> {
    std::str::from_utf8(value).ok()?.parse().ok()
}

fn read_colon_file<T, F>(
    mut reader: impl BufRead,
    required_colons: usize,
    mut parse: F,
) -> io::Result<Option<T>>
where
    F: FnMut(&[u8]) -> Option<T>,
{
    let mut prefix = Vec::new();
    let mut colons = 0;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if prefix.is_empty() {
                return Ok(None);
            }
            let line = trim_go_space(&prefix);
            return if line.is_empty() || line[0] == b'#' {
                Ok(None)
            } else {
                Ok(parse(line))
            };
        }

        let mut consumed = 0;
        let mut reached_newline = false;
        while consumed < available.len() {
            let byte = available[consumed];
            consumed += 1;
            if byte == b'\n' {
                reached_newline = true;
                break;
            }
            prefix.push(byte);
            colons += usize::from(byte == b':');
            if colons >= required_colons {
                break;
            }
        }
        reader.consume(consumed);

        if colons >= required_colons || reached_newline {
            let line = trim_go_space(&prefix);
            if !line.is_empty()
                && line[0] != b'#'
                && let Some(value) = parse(line)
            {
                return Ok(Some(value));
            }
            if !reached_newline {
                drain_line(&mut reader)?;
            }
            prefix.clear();
            colons = 0;
        }
    }
}

fn drain_line(reader: &mut impl BufRead) -> io::Result<()> {
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(());
        }
        if let Some(index) = available.iter().position(|byte| *byte == b'\n') {
            reader.consume(index + 1);
            return Ok(());
        }
        let len = available.len();
        reader.consume(len);
    }
}

fn trim_go_space(mut value: &[u8]) -> &[u8] {
    while let Some(len) = leading_space_len(value) {
        value = &value[len..];
    }
    while let Some(len) = trailing_space_len(value) {
        value = &value[..value.len() - len];
    }
    value
}

fn leading_space_len(value: &[u8]) -> Option<usize> {
    let first = *value.first()?;
    if first.is_ascii() {
        return go_ascii_space(first).then_some(1);
    }
    let width = utf8_width(first)?;
    let text = std::str::from_utf8(value.get(..width)?).ok()?;
    let character = text.chars().next()?;
    character.is_whitespace().then_some(character.len_utf8())
}

fn trailing_space_len(value: &[u8]) -> Option<usize> {
    let last = *value.last()?;
    if last.is_ascii() {
        return go_ascii_space(last).then_some(1);
    }
    let lower = value.len().saturating_sub(4);
    let start = (lower..value.len()).find(|start| std::str::from_utf8(&value[*start..]).is_ok())?;
    let text = std::str::from_utf8(&value[start..]).ok()?;
    let character = text.chars().next()?;
    (character.is_whitespace() && character.len_utf8() == value.len() - start)
        .then_some(character.len_utf8())
}

fn utf8_width(first: u8) -> Option<usize> {
    match first {
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

fn go_ascii_space(byte: u8) -> bool {
    matches!(byte, b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' ')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor, Read};

    #[test]
    fn user_fixture_matches_go_parser_edges() {
        let fixture = b"\n # comment\r\n+nis:x:1:2::/:/bin/sh\n-invalid:x:1:2::/:/bin/sh\n\
bad:x:no:2::/:/bin/sh\nbad:x:7:no::/:/bin/sh\nbad:x:+7:8:Display,Room:/home/bad:/bin/sh\n\
bad:x:9223372036854775808:8::/:/bin/sh\n\xC2\xA0good\xff:x:-1:4294967295:Name,Else:/home/\xff:/bin/sh\xE2\x80\x83\n";
        let user = find_user_by_name(BufReader::new(&fixture[..]), b"good\xff")
            .unwrap()
            .unwrap();
        assert_eq!(user.username.as_bytes(), b"good\xff");
        assert_eq!(user.uid, "-1");
        assert_eq!(user.gid, "4294967295");
        assert_eq!(user.name.as_bytes(), b"Name");
        assert_eq!(user.home_dir.as_os_str().as_bytes(), b"/home/\xff");
    }

    #[test]
    fn malformed_duplicate_is_skipped_for_later_valid_record() {
        let fixture = b"same:x:no:3::/:/bin/sh\nsame:x:2:3::/ok:/bin/sh\n";
        let user = find_user_by_name(BufReader::new(&fixture[..]), b"same")
            .unwrap()
            .unwrap();
        assert_eq!((user.uid.as_str(), user.gid.as_str()), ("2", "3"));
    }

    #[test]
    fn group_fixture_handles_nul_and_signed_boundaries() {
        let fixture = b"nul\0name:x:-9223372036854775808:member\n";
        let group = find_group_by_name(BufReader::new(&fixture[..]), b"nul\0name")
            .unwrap()
            .unwrap();
        assert_eq!(group.name.as_bytes(), b"nul\0name");
        assert_eq!(group.gid, "-9223372036854775808");
    }

    #[test]
    fn accepts_newline_free_final_record_and_large_prefix() {
        let mut fixture = vec![b'x'; 32 * 1024];
        fixture.extend_from_slice(b":x:1:2::/home:/bin/sh");
        let user = find_user_by_name(
            BufReader::with_capacity(17, Cursor::new(fixture)),
            &vec![b'x'; 32 * 1024],
        )
        .unwrap()
        .unwrap();
        assert_eq!(user.uid, "1");
    }

    struct FailAfter {
        bytes: Cursor<Vec<u8>>,
        allowed: usize,
        read: usize,
    }

    impl Read for FailAfter {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.read >= self.allowed {
                return Err(io::Error::other("injected"));
            }
            let remaining = self.allowed - self.read;
            let limit = buffer.len().min(remaining);
            let count = self.bytes.read(&mut buffer[..limit])?;
            self.read += count;
            Ok(count)
        }
    }

    #[test]
    fn matched_suffix_error_is_hidden_but_nonmatch_drain_observes_it() {
        let row = b"match:x:1:2::/home:".to_vec();
        let reader = BufReader::with_capacity(
            8,
            FailAfter {
                allowed: row.len(),
                bytes: Cursor::new(row.clone()),
                read: 0,
            },
        );
        assert!(find_user_by_name(reader, b"match").unwrap().is_some());

        let reader = BufReader::with_capacity(
            8,
            FailAfter {
                allowed: row.len(),
                bytes: Cursor::new(row),
                read: 0,
            },
        );
        assert_eq!(
            find_user_by_name(reader, b"other").unwrap_err().kind(),
            io::ErrorKind::Other
        );
    }

    #[test]
    fn overflow_and_short_records_are_unknown() {
        let fixture = b"short:x:1\nover:x:9223372036854775808:1::/:/bin/sh\n";
        assert!(
            find_user_by_name(BufReader::new(&fixture[..]), b"short")
                .unwrap()
                .is_none()
        );
        assert!(
            find_user_by_name(BufReader::new(&fixture[..]), b"over")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn focused_rows_cover_name_filters_whitespace_and_numeric_edges() {
        for rejected in [b"+plus".as_slice(), b"-minus".as_slice()] {
            let mut row = rejected.to_vec();
            row.extend_from_slice(b":x:1:2::/:/bin/sh\n");
            assert!(
                find_user_by_name(BufReader::new(row.as_slice()), rejected)
                    .unwrap()
                    .is_none()
            );
        }

        let plus = b"plus:x:+7:+8:Display:/home/plus:/bin/sh\n";
        let user = find_user_by_name(BufReader::new(&plus[..]), b"plus")
            .unwrap()
            .unwrap();
        assert_eq!((user.uid.as_str(), user.gid.as_str()), ("+7", "+8"));

        let maximum = b"max:x:9223372036854775807:-9223372036854775808::/:/bin/sh\n";
        let user = find_user_by_name(BufReader::new(&maximum[..]), b"max")
            .unwrap()
            .unwrap();
        assert_eq!(user.uid, "9223372036854775807");
        assert_eq!(user.gid, "-9223372036854775808");

        let whitespace = b"\xC2\xA0space:x:1:2::/:/bin/sh\xE2\x80\x83\r\n";
        assert!(
            find_user_by_name(BufReader::new(&whitespace[..]), b"space")
                .unwrap()
                .is_some()
        );
        let only_ignored = b"\t\r\n# comment:x:1:2::/:/bin/sh\n";
        assert!(
            find_user_by_name(BufReader::new(&only_ignored[..]), b"comment")
                .unwrap()
                .is_none()
        );

        for comment in [
            b"#shadow:x:1:2::/:/bin/sh\n".as_slice(),
            b"  #shadow:x:1:2::/:/bin/sh".as_slice(),
        ] {
            assert!(
                find_user_by_name(BufReader::new(comment), b"#shadow")
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            find_group_by_name(
                BufReader::new(b"\t#shadow:x:7:member\n".as_slice()),
                b"#shadow",
            )
            .unwrap()
            .is_none()
        );

        let five_colons = b"short:x:1:2:Name:/home\n";
        assert!(
            find_user_by_name(BufReader::new(&five_colons[..]), b"short")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn current_prefers_file_record_and_falls_back_on_every_lookup_error() {
        let from_file = User {
            uid: "7".into(),
            gid: "8".into(),
            username: OsString::from("file"),
            name: OsString::from("File User"),
            home_dir: PathBuf::from("/file"),
        };
        let selected = current_user_with(
            "7".into(),
            |_| Ok(from_file.clone()),
            || panic!("successful lookup must not observe fallback inputs"),
        )
        .unwrap();
        assert_eq!(selected, from_file);

        let fallback = current_user_with(
            "7".into(),
            |_| {
                Err(LookupError::io(
                    "injected",
                    io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
                ))
            },
            || {
                (
                    "9".into(),
                    Some(OsString::from_vec(vec![b'u', 0xff])),
                    Some(OsString::from_vec(vec![b'/', 0xfe])),
                )
            },
        )
        .unwrap();
        assert_eq!(fallback.uid, "7");
        assert_eq!(fallback.gid, "9");
        assert_eq!(fallback.username.as_bytes(), b"u\xff");
        assert_eq!(fallback.home_dir.as_os_str().as_bytes(), b"/\xfe");
    }

    #[test]
    fn current_fallback_reports_missing_environment_in_go_order() {
        let missing = current_user_with(
            "7".into(),
            |_| Err(LookupError::UnknownUserId("7".into())),
            || ("9".into(), None, Some(OsString::new())),
        )
        .unwrap_err();
        assert_eq!(
            missing.to_string(),
            "user: Current requires cgo or $USER, $HOME set in environment"
        );
    }
}
