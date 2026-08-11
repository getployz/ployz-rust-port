use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use ployz_internal_grpcversion::{
    METADATA_KEY_CLIENT_VERSION, METADATA_KEY_MIN_SERVER_VERSION, RELEASE_URL, VersionPolicy,
};
use tonic::{Code, Request};

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
        "ployz-grpcversion-{}-{unique}-{suffix}",
        std::process::id()
    ))
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

fn go_oracle_output(repository: &Path) -> String {
    let overlay_path = unique_temp_path("overlay.json");
    let go_cache = unique_temp_path("go-cache");
    let virtual_test =
        repository.join("upstream/uncloud/internal/grpcversion/ployz_oracle_test.go");
    let test_source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/oracle_grpcversion_test.go")
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
            "-run=^TestPloyzGrpcVersionOracle$",
            "-count=1",
            "-v",
            "./internal/grpcversion",
        ])
        .output()
        .expect("run pinned Go grpcversion oracle");
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

fn hex(value: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn request(pairs: &[(&'static str, &'static str)]) -> Request<()> {
    let mut request = Request::new(());
    for (key, value) in pairs {
        request.metadata_mut().append(*key, value.parse().unwrap());
    }
    request
}

fn rust_output() -> String {
    let prefixes = ["", "v", "V", " "];
    let cores = [
        "",
        "0",
        "00",
        "1",
        "01",
        "1.2",
        "01.002",
        "1.2.3",
        "1.2.3.4",
        "9223372036854775807",
        "9223372036854775808",
    ];
    let suffixes = [
        "",
        "-alpha",
        "-00",
        "-a..b",
        "+build",
        "+001",
        "-alpha+build",
        "-é",
        " ",
    ];
    let mut output = String::new();
    for prefix in prefixes {
        for core in cores {
            for suffix in suffixes {
                let input = format!("{prefix}{core}{suffix}");
                let policy = VersionPolicy::new(&input);
                writeln!(
                    output,
                    "parse:{}={}",
                    hex(input.as_bytes()),
                    policy.current_version()
                )
                .unwrap();
            }
        }
    }

    let policy = VersionPolicy::new("999.0.0-dev");
    let cases: &[(&str, &[(&str, &str)])] = &[
        ("missing", &[]),
        ("malformed", &[(METADATA_KEY_CLIENT_VERSION, "bad")]),
        ("client-old", &[(METADATA_KEY_CLIENT_VERSION, "0.19.9")]),
        (
            "server-old",
            &[
                (METADATA_KEY_CLIENT_VERSION, "999.0.0"),
                (METADATA_KEY_MIN_SERVER_VERSION, "999.0.0"),
            ],
        ),
        (
            "accepted",
            &[
                (METADATA_KEY_CLIENT_VERSION, "999.0.0"),
                (METADATA_KEY_MIN_SERVER_VERSION, "0.20.0"),
            ],
        ),
        (
            "duplicate-first",
            &[
                (METADATA_KEY_CLIENT_VERSION, "999.0.0"),
                (METADATA_KEY_CLIENT_VERSION, "0.0.0"),
                (METADATA_KEY_MIN_SERVER_VERSION, "0.20.0"),
            ],
        ),
    ];
    for (name, pairs) in cases {
        match policy.validate_request(&request(pairs)) {
            Ok(()) => writeln!(output, "validate:{name}=ok").unwrap(),
            Err(status) => writeln!(
                output,
                "validate:{name}={}|{}",
                code_number(status.code()),
                status.message()
            )
            .unwrap(),
        }
    }
    output
}

fn code_number(code: Code) -> i32 {
    code as i32
}

#[test]
fn parser_and_policy_match_the_pinned_go_oracle_after_product_rename() {
    let repository = repository_root();
    let oracle = go_oracle_output(&repository).replace(
        "https://github.com/psviderski/uncloud/releases/latest",
        RELEASE_URL,
    );
    assert_eq!(rust_output(), oracle);
}
