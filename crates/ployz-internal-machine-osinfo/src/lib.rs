//! Best-effort host operating-system information for the Ployz machine daemon.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

const OS_RELEASE_PATHS: [&str; 2] = ["/etc/os-release", "/usr/lib/os-release"];
const DEBIAN_VERSION_PATH: &str = "/etc/debian_version";
const MAX_SCAN_TOKEN_SIZE: usize = 64 * 1024;

/// Returns a human-readable operating-system name and version.
///
/// The first readable `os-release` file is used. Failures are deliberately
/// represented by an empty string because machine information is collected on
/// a best-effort basis.
pub fn pretty_name() -> String {
    pretty_name_from(&OS_RELEASE_PATHS, Path::new(DEBIAN_VERSION_PATH))
}

fn pretty_name_from<P: AsRef<Path>>(os_release_paths: &[P], debian_version_path: &Path) -> String {
    let release = read_os_release(os_release_paths);
    if release.is_empty() {
        return String::new();
    }

    let debian_version = if release.get("ID").is_some_and(|id| id == "debian") {
        fs::read(debian_version_path)
            .ok()
            .map(|data| String::from_utf8_lossy(&data).trim().to_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };

    build_pretty_name(&release, &debian_version)
}

fn read_os_release<P: AsRef<Path>>(paths: &[P]) -> HashMap<String, String> {
    for path in paths {
        if let Ok(file) = File::open(path) {
            return parse_os_release(BufReader::new(file));
        }
    }

    HashMap::new()
}

fn parse_os_release(reader: impl BufRead) -> HashMap<String, String> {
    let mut release = HashMap::new();
    let mut reader = reader;
    let mut line = Vec::new();

    loop {
        line.clear();
        let bytes_read = {
            let mut limited_line = (&mut reader).take((MAX_SCAN_TOKEN_SIZE + 1) as u64);
            match limited_line.read_until(b'\n', &mut line) {
                Ok(bytes_read) => bytes_read,
                Err(_) => break,
            }
        };
        if bytes_read == 0 {
            break;
        }

        // Go's bufio.Scanner silently stops at its default 64 KiB token limit.
        // Preserve that limitation and retain fields parsed before the long line.
        let has_newline = line.last() == Some(&b'\n');
        if (has_newline && line.len() > MAX_SCAN_TOKEN_SIZE)
            || (!has_newline && line.len() >= MAX_SCAN_TOKEN_SIZE)
        {
            break;
        }

        let line = String::from_utf8_lossy(&line);
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_owned();
        let value = value.trim().trim_matches(['"', '\'']).to_owned();
        release.insert(key, value);
    }

    release
}

fn build_pretty_name(release: &HashMap<String, String>, debian_version: &str) -> String {
    if release.get("ID").is_some_and(|id| id == "debian") {
        return format!("Debian {debian_version}");
    }

    if let Some(pretty_name) = nonempty(release, "PRETTY_NAME") {
        return pretty_name.to_owned();
    }

    let mut parts = Vec::with_capacity(3);
    if let Some(name) = nonempty(release, "NAME").or_else(|| nonempty(release, "ID")) {
        parts.push(name.to_owned());
    }
    if let Some(version) = nonempty(release, "VERSION_ID") {
        parts.push(version.to_owned());
    }
    if let Some(codename) = nonempty(release, "VERSION_CODENAME") {
        parts.push(format!("({codename})"));
    }
    parts.join(" ")
}

fn nonempty<'a>(release: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    release
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

/// Returns the running Linux kernel release.
///
/// On non-Linux hosts, and if the Linux `uname` operation fails, this returns
/// an empty string.
pub fn kernel_version() -> String {
    platform::kernel_version()
}

#[cfg(target_os = "linux")]
mod platform {
    use std::ffi::{c_char, c_int};
    use std::mem::MaybeUninit;

    const UTS_FIELD_LEN: usize = 65;

    #[repr(C)]
    struct UtsName {
        sysname: [c_char; UTS_FIELD_LEN],
        nodename: [c_char; UTS_FIELD_LEN],
        release: [c_char; UTS_FIELD_LEN],
        version: [c_char; UTS_FIELD_LEN],
        machine: [c_char; UTS_FIELD_LEN],
        domainname: [c_char; UTS_FIELD_LEN],
    }

    unsafe extern "C" {
        fn uname(name: *mut UtsName) -> c_int;
    }

    pub(super) fn kernel_version() -> String {
        let mut name = MaybeUninit::<UtsName>::zeroed();

        // SAFETY: `name` points to writable, correctly sized and aligned storage
        // for Linux libc's `struct utsname`. A zero return means libc initialized it.
        if unsafe { uname(name.as_mut_ptr()) } != 0 {
            return String::new();
        }

        // SAFETY: the successful `uname` call above initialized the entire value.
        let name = unsafe { name.assume_init() };
        let release = name
            .release
            .iter()
            .take_while(|byte| **byte != 0)
            .map(|byte| *byte as u8)
            .collect::<Vec<_>>();
        String::from_utf8_lossy(&release).into_owned()
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    pub(super) fn kernel_version() -> String {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn parse(content: &str) -> HashMap<String, String> {
        parse_os_release(Cursor::new(content))
    }

    fn map(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn temporary_directory(test_name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ployz-osinfo-{}-{test_name}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temporary test directory");
        path
    }

    #[test]
    fn parses_ubuntu_release() {
        let actual = parse(
            r#"PRETTY_NAME="Ubuntu 24.04.4 LTS"
NAME="Ubuntu"
VERSION_ID="24.04"
VERSION="24.04.4 LTS (Noble Numbat)"
VERSION_CODENAME=noble
ID=ubuntu"#,
        );
        assert_eq!(
            actual,
            map(&[
                ("PRETTY_NAME", "Ubuntu 24.04.4 LTS"),
                ("NAME", "Ubuntu"),
                ("VERSION_ID", "24.04"),
                ("VERSION", "24.04.4 LTS (Noble Numbat)"),
                ("VERSION_CODENAME", "noble"),
                ("ID", "ubuntu"),
            ])
        );
    }

    #[test]
    fn parses_debian_release() {
        let actual = parse(
            r#"PRETTY_NAME="Debian GNU/Linux 13 (trixie)"
NAME="Debian GNU/Linux"
VERSION_ID="13"
VERSION="13 (trixie)"
VERSION_CODENAME=trixie
ID=debian"#,
        );
        assert_eq!(
            actual,
            map(&[
                ("PRETTY_NAME", "Debian GNU/Linux 13 (trixie)"),
                ("NAME", "Debian GNU/Linux"),
                ("VERSION_ID", "13"),
                ("VERSION", "13 (trixie)"),
                ("VERSION_CODENAME", "trixie"),
                ("ID", "debian"),
            ])
        );
    }

    #[test]
    fn parses_alpine_release() {
        assert_eq!(
            parse(
                r#"NAME="Alpine Linux"
ID=alpine
VERSION_ID=3.20.0
PRETTY_NAME="Alpine Linux v3.20""#
            ),
            map(&[
                ("NAME", "Alpine Linux"),
                ("ID", "alpine"),
                ("VERSION_ID", "3.20.0"),
                ("PRETTY_NAME", "Alpine Linux v3.20"),
            ])
        );
    }

    #[test]
    fn parse_skips_comments_blank_lines_and_strips_quotes() {
        assert_eq!(
            parse(
                "# This is a comment\n\nID=ubuntu\n\n# Another comment\nPRETTY_NAME='Ubuntu 24.04.4 LTS'"
            ),
            map(&[("ID", "ubuntu"), ("PRETTY_NAME", "Ubuntu 24.04.4 LTS"),])
        );
    }

    #[test]
    fn parses_empty_release() {
        assert!(parse("").is_empty());
    }

    #[test]
    fn ubuntu_uses_pretty_name_with_point_release() {
        assert_eq!(
            build_pretty_name(
                &map(&[("ID", "ubuntu"), ("PRETTY_NAME", "Ubuntu 24.04.4 LTS"),]),
                "",
            ),
            "Ubuntu 24.04.4 LTS"
        );
    }

    #[test]
    fn debian_uses_debian_version_for_point_release() {
        assert_eq!(
            build_pretty_name(
                &map(&[
                    ("ID", "debian"),
                    ("PRETTY_NAME", "Debian GNU/Linux 13 (trixie)"),
                ]),
                "13.5",
            ),
            "Debian 13.5"
        );
    }

    #[test]
    fn debian_unstable_uses_codename_from_debian_version() {
        assert_eq!(
            build_pretty_name(
                &map(&[
                    ("ID", "debian"),
                    ("PRETTY_NAME", "Debian GNU/Linux 13 (trixie)"),
                ]),
                "trixie/sid",
            ),
            "Debian trixie/sid"
        );
    }

    #[test]
    fn alpine_uses_pretty_name() {
        assert_eq!(
            build_pretty_name(
                &map(&[("ID", "alpine"), ("PRETTY_NAME", "Alpine Linux v3.20"),]),
                "",
            ),
            "Alpine Linux v3.20"
        );
    }

    #[test]
    fn fallback_composes_name_version_and_codename() {
        assert_eq!(
            build_pretty_name(
                &map(&[
                    ("NAME", "Foo Linux"),
                    ("VERSION_ID", "1.2"),
                    ("VERSION_CODENAME", "bar"),
                ]),
                "",
            ),
            "Foo Linux 1.2 (bar)"
        );
    }

    #[test]
    fn empty_release_has_empty_pretty_name() {
        assert_eq!(build_pretty_name(&HashMap::new(), ""), "");
    }

    #[test]
    fn missing_release_file_has_empty_pretty_name() {
        let directory = temporary_directory("missing-release");
        let result = pretty_name_from(&[directory.join("missing")], &directory.join("debian"));
        fs::remove_dir(directory).expect("remove temporary test directory");
        assert_eq!(result, "");
    }

    #[test]
    fn debian_point_release_is_read_and_trimmed() {
        let directory = temporary_directory("debian-point-release");
        let release_path = directory.join("os-release");
        let debian_path = directory.join("debian_version");
        fs::write(
            &release_path,
            "ID=debian\nPRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\nVERSION_ID=\"13\"",
        )
        .expect("write os-release fixture");
        fs::write(&debian_path, "13.5\n").expect("write debian_version fixture");

        let result = pretty_name_from(&[release_path], &debian_path);
        fs::remove_dir_all(directory).expect("remove temporary test directory");
        assert_eq!(result, "Debian 13.5");
    }

    #[test]
    fn first_readable_release_file_wins_even_when_empty() {
        let directory = temporary_directory("first-readable-wins");
        let first = directory.join("first");
        let second = directory.join("second");
        fs::write(&first, "# empty after parsing\n").expect("write first fixture");
        fs::write(&second, "PRETTY_NAME=Later").expect("write second fixture");

        let result = pretty_name_from(&[first, second], &directory.join("debian"));
        fs::remove_dir_all(directory).expect("remove temporary test directory");
        assert_eq!(result, "");
    }

    #[test]
    fn parser_stops_reading_at_scanner_token_limit() {
        let prefix = "ID=before\n";
        let content = format!("{prefix}{}\nPRETTY_NAME=after", "x".repeat(1024 * 1024));
        let mut reader = Cursor::new(content);

        assert_eq!(parse_os_release(&mut reader), map(&[("ID", "before")]));
        assert!(
            reader.position() <= (prefix.len() + MAX_SCAN_TOKEN_SIZE + 1) as u64,
            "oversized line must not be consumed past the scanner limit"
        );
    }

    #[test]
    fn parser_uses_first_equals_and_last_duplicate_value() {
        assert_eq!(
            parse(" BROKEN \n KEY = '=value=' \nKEY=last"),
            map(&[("KEY", "last")])
        );
    }

    #[test]
    fn debian_without_version_file_keeps_trailing_space() {
        let directory = temporary_directory("debian-version-missing");
        let release_path = directory.join("os-release");
        fs::write(&release_path, "ID=debian\nPRETTY_NAME=ignored")
            .expect("write os-release fixture");

        let result = pretty_name_from(&[release_path], &directory.join("missing"));
        fs::remove_dir_all(directory).expect("remove temporary test directory");
        assert_eq!(result, "Debian ");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn kernel_version_matches_linux_kernel_interface() {
        let expected = fs::read_to_string("/proc/sys/kernel/osrelease")
            .expect("read Linux kernel release from procfs");
        assert_eq!(kernel_version(), expected.trim_end_matches('\n'));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn kernel_version_is_empty_off_linux() {
        assert_eq!(kernel_version(), "");
    }
}
