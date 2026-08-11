use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use ployz_internal_version::Info;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives beneath the repository root")
        .to_owned()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn go_oracle() -> BTreeMap<String, String> {
    let root = repository_root();
    let package = root.join("upstream/uncloud/internal/version");
    let imaginary_test = package.join("ployz_oracle_test.go");
    let actual_test = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle_version_test.go");
    let overlay = std::env::temp_dir().join(format!(
        "ployz-version-go-overlay-{}.json",
        std::process::id()
    ));
    let go_cache =
        std::env::temp_dir().join(format!("ployz-version-go-cache-{}", std::process::id()));
    std::fs::create_dir_all(&go_cache).expect("create writable Go cache");
    let overlay_json = format!(
        "{{\"Replace\":{{\"{}\":\"{}\"}}}}",
        imaginary_test.display(),
        actual_test.display()
    );
    std::fs::write(&overlay, overlay_json).expect("write Go overlay");

    let mise_go = Command::new("mise")
        .args(["where", "go@1.26.1"])
        .current_dir(&root)
        .output()
        .expect("locate pinned Go toolchain");
    assert!(
        mise_go.status.success(),
        "locating pinned Go failed: {}",
        String::from_utf8_lossy(&mise_go.stderr)
    );
    let go = PathBuf::from(
        String::from_utf8(mise_go.stdout)
            .expect("mise output is UTF-8")
            .trim(),
    )
    .join("bin/go");

    let output = Command::new(go)
        .args([
            "test",
            "-run=^TestPloyzVersionOracle$",
            "-count=1",
            "-v",
            "-overlay",
        ])
        .arg(&overlay)
        .arg("./internal/version")
        .current_dir(root.join("upstream/uncloud"))
        .env("GOCACHE", &go_cache)
        .output()
        .expect("run pinned Go oracle");
    let _ = std::fs::remove_file(overlay);
    let _ = std::fs::remove_dir_all(go_cache);
    assert!(
        output.status.success(),
        "Go oracle failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout)
        .expect("Go output is UTF-8")
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(key, _)| key.starts_with("PLOYZ_ORACLE_"))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

#[test]
fn human_json_version_and_dirty_rules_match_the_go_oracle() {
    let oracle = go_oracle();
    let info = Info {
        version: "v1.2.3",
        git_commit: "0123456789abcdef",
        git_state: "dirty",
        build_date: "2026-08-11T01:02:03".to_owned(),
        built_by: "goreleaser",
        go_version: "go1.26.1",
        platform: "linux/amd64".to_owned(),
    };

    assert_eq!(
        oracle["PLOYZ_ORACLE_TEXT"],
        hex(info.to_string().as_bytes())
    );
    assert_eq!(
        oracle["PLOYZ_ORACLE_JSON"],
        hex(info.json_string().as_bytes())
    );
    let control_info = Info {
        version: "v\tX",
        git_commit: "commit\ncontinued\tcolumn",
        git_state: "dirty",
        build_date: "2026-08-11T01:02:03".to_owned(),
        built_by: "builder\u{0b}soft\u{0c}form",
        go_version: "go1.26.1",
        platform: "linux/amd64".to_owned(),
    };
    assert_eq!(
        oracle["PLOYZ_ORACLE_CONTROL_TEXT"],
        hex(control_info.to_string().as_bytes())
    );
    assert_eq!(oracle["PLOYZ_ORACLE_DEVEL"], hex(b"999.0.0-dev"));
    assert_eq!(oracle["PLOYZ_ORACLE_RELEASE"], hex(b"v9.8.7"));

    assert_eq!(oracle["PLOYZ_ORACLE_DIRTY_74727565"], hex(b"dirty"));
    assert_eq!(oracle["PLOYZ_ORACLE_DIRTY_66616c7365"], hex(b"clean"));

    let fallback_for_empty = &oracle["PLOYZ_ORACLE_DIRTY_"];
    assert_eq!(
        oracle["PLOYZ_ORACLE_DIRTY_696e76616c6964"],
        *fallback_for_empty
    );
    assert_eq!(oracle["PLOYZ_ORACLE_DIRTY_54525545"], *fallback_for_empty);
}
