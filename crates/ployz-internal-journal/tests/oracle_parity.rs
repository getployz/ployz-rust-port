#![cfg(unix)]

use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use ployz_internal_journal::Journal;
use ployz_pkg_api::{LogEntry, ServiceLogsOptions};

fn repository_root() -> PathBuf {
    std::env::var_os("PLOYZ_REPOSITORY")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn unique_temp_path(suffix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "ployz-journal-{}-{unique}-{suffix}",
        std::process::id()
    ))
}

fn go_oracle_output(repository: &Path) -> String {
    let overlay_path = unique_temp_path("overlay.json");
    let go_cache = unique_temp_path("go-cache");
    let virtual_test = repository.join("upstream/uncloud/internal/journal/ployz_oracle_test.go");
    let test_source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/oracle_journal_test.go")
        .canonicalize()
        .expect("canonicalize Go oracle overlay source");
    fs::write(
        &overlay_path,
        format!(
            "{{\"Replace\":{{\"{}\":\"{}\"}}}}",
            virtual_test.display(),
            test_source.display()
        ),
    )
    .expect("write Go overlay");
    fs::create_dir(&go_cache).expect("create isolated Go build cache");

    let output = Command::new("mise")
        .current_dir(repository)
        .env("GOCACHE", &go_cache)
        .args([
            "exec",
            "--locked",
            "go@1.26.1",
            "--",
            "go",
            "-C",
            "upstream/uncloud",
            "test",
            &format!("-overlay={}", overlay_path.display()),
            "-run=^TestPloyzJournalOracle$",
            "-count=1",
            "-v",
            "./internal/journal",
        ])
        .output()
        .expect("run pinned Go journal oracle");
    fs::remove_file(&overlay_path).expect("remove Go overlay");
    fs::remove_dir_all(&go_cache).expect("remove isolated Go build cache");
    assert!(
        output.status.success(),
        "Go oracle failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("Go output is UTF-8");
    let encoded = stdout
        .split_once("PLOYZ_ORACLE_BEGIN\n")
        .and_then(|(_, rest)| rest.split_once("\nPLOYZ_ORACLE_END\n"))
        .map(|(encoded, _)| encoded)
        .expect("Go oracle emitted markers");
    decode_hex(encoded)
}

fn rust_output() -> String {
    let fixture = Fixture::new();
    let mut output = String::new();

    let normal = b"1769188773.687500 first\n-- Boot marker --\r\n\n0.000000 \xff\xfe\nfinal\r";
    fixture.write_input(normal);
    write_entries(
        &mut output,
        "normal",
        Journal::new(&fixture.executable)
            .logs("unused", &ServiceLogsOptions::default())
            .expect("start normal fixture"),
    );

    fixture.write_input(&vec![b'x'; 64 * 1024]);
    write_entries(
        &mut output,
        "long",
        Journal::new(&fixture.executable)
            .logs("unused", &ServiceLogsOptions::default())
            .expect("start long-line fixture"),
    );

    let options = ServiceLogsOptions {
        follow: true,
        tail: -1,
        since: "10 minutes ago".into(),
        until: "now".into(),
        ..ServiceLogsOptions::default()
    };
    fixture.write_input(b"");
    Journal::new(&fixture.executable)
        .logs("uncloud.service", &options)
        .expect("start argument fixture")
        .for_each(drop);
    writeln!(output, "command|{}", hex(b"journalctl")).unwrap();
    for argument in fs::read(&fixture.arguments)
        .expect("read captured arguments")
        .split(|byte| *byte == b'\n')
        .filter(|argument| !argument.is_empty())
    {
        writeln!(output, "arg|{}", hex(argument)).unwrap();
    }

    output
}

fn write_entries(output: &mut String, label: &str, entries: impl IntoIterator<Item = LogEntry>) {
    for entry in entries {
        let timestamp = entry.timestamp.map_or_else(
            || "-".to_owned(),
            |timestamp| {
                let elapsed = timestamp
                    .duration_since(UNIX_EPOCH)
                    .expect("fixture timestamp is after Unix epoch");
                format!("{}.{:09}", elapsed.as_secs(), elapsed.subsec_nanos())
            },
        );
        let error = entry
            .error
            .map_or_else(Vec::new, |error| error.to_string().into_bytes());
        writeln!(
            output,
            "{label}|{}|{timestamp}|{}|{}",
            entry.stream as i32,
            hex(&entry.message),
            hex(&error)
        )
        .unwrap();
    }
}

fn hex(value: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn decode_hex(encoded: &str) -> String {
    assert!(encoded.len().is_multiple_of(2));
    let bytes = encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII hex"), 16)
                .expect("valid hex")
        })
        .collect::<Vec<_>>();
    String::from_utf8(bytes).expect("oracle output is UTF-8")
}

struct Fixture {
    directory: PathBuf,
    executable: PathBuf,
    input: PathBuf,
    arguments: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = unique_temp_path("fixture");
        fs::create_dir(&directory).expect("create fixture directory");
        let executable = directory.join("journalctl");
        let input = directory.join("input");
        let arguments = directory.join("arguments");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n/bin/cat '{}'\nexit 19\n",
            arguments.display(),
            input.display()
        );
        fs::write(&executable, script).expect("write fixture executable");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
            .expect("make fixture executable");
        Self {
            directory,
            executable,
            input,
            arguments,
        }
    }

    fn write_input(&self, input: &[u8]) {
        fs::write(&self.input, input).expect("write fixture input");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn process_arguments_entries_and_scanner_limit_match_pinned_go_oracle() {
    assert_eq!(rust_output(), go_oracle_output(&repository_root()));
}
