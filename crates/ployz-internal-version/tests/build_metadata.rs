#![cfg(unix)]

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

const BUILD_VARIABLES: &[&str] = &[
    "PLOYZ_VERSION",
    "PLOYZ_GIT_COMMIT",
    "PLOYZ_GIT_DIRTY",
    "PLOYZ_BUILD_DATE",
    "PLOYZ_BUILT_BY",
    "VERGEN_GIT_SHA",
    "VERGEN_GIT_DIRTY",
    "VERGEN_GIT_COMMIT_TIMESTAMP",
    "VERGEN_IDEMPOTENT",
    "VERGEN_DEFAULT_ON_ERROR",
    "SOURCE_DATE_EPOCH",
];

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("ployz-version-{name}-{}-{id}", std::process::id()));
        fs::create_dir(&path).expect("create isolated test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_crate(destination: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"));
    fs::copy(source.join(".gitignore"), destination.join(".gitignore")).expect("copy ignore rules");
    fs::copy(source.join("Cargo.toml"), destination.join("Cargo.toml")).expect("copy manifest");
    fs::copy(source.join("build.rs"), destination.join("build.rs")).expect("copy build script");
    copy_tree(&source.join("src"), &destination.join("src"));
    copy_tree(&source.join("examples"), &destination.join("examples"));
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir(destination).expect("create copied source directory");
    for entry in fs::read_dir(source).expect("read source directory") {
        let entry = entry.expect("read source entry");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type().expect("read source type").is_dir() {
            copy_tree(&source_path, &destination_path);
        } else {
            fs::copy(source_path, destination_path).expect("copy source file");
        }
    }
}

fn run(command: &mut Command) -> Output {
    let output = command.output().expect("start command");
    assert!(
        output.status.success(),
        "command failed: {command:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = run(Command::new("git").args(arguments).current_dir(directory));
    String::from_utf8(output.stdout)
        .expect("Git output is UTF-8")
        .trim()
        .to_owned()
}

fn init_repository(directory: &Path) {
    git(directory, &["init", "--initial-branch=main"]);
    git(directory, &["config", "user.name", "Ployz Test"]);
    git(directory, &["config", "user.email", "test@ployz.invalid"]);
    fs::write(directory.join("tracked.txt"), "clean\n").expect("write tracked fixture");
    git(directory, &["add", "."]);
    commit(directory, "initial", "2026-08-10T01:02:03Z");
}

fn commit(directory: &Path, message: &str, timestamp: &str) {
    run(Command::new("git")
        .args(["commit", "--allow-empty", "-m", message])
        .current_dir(directory)
        .env("GIT_AUTHOR_DATE", timestamp)
        .env("GIT_COMMITTER_DATE", timestamp));
}

fn cargo_path() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

fn rustup_tool(tool: &str) -> PathBuf {
    let output = run(Command::new("rustup").args(["which", tool, "--toolchain", "1.96.0"]));
    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("rustup output is UTF-8")
            .trim(),
    )
}

fn probe(
    directory: &Path,
    field: &str,
    variables: &[(&str, &OsStr)],
    environment: &[(&str, &OsStr)],
) -> String {
    let mut command = Command::new(cargo_path());
    command
        .args([
            "run",
            "--offline",
            "--quiet",
            "--example",
            "probe",
            "--",
            field,
        ])
        .current_dir(directory)
        .env("CARGO_TARGET_DIR", directory.join("target"));
    for variable in BUILD_VARIABLES {
        command.env_remove(variable);
    }
    for (name, value) in variables.iter().chain(environment) {
        command.env(name, value);
    }

    String::from_utf8(run(&mut command).stdout).expect("probe output is UTF-8")
}

fn plain_probe(directory: &Path, field: &str) -> String {
    probe(directory, field, &[], &[])
}

fn assert_generated_unknown(directory: &Path, environment: &[(&str, &OsStr)]) {
    for field in ["commit", "dirty", "date"] {
        assert_eq!(
            probe(directory, field, &[], environment),
            "unknown",
            "generated {field} should be unknown"
        );
    }
}

#[test]
fn repository_metadata_restarts_for_every_worktree_change_without_clean() {
    let repository = TempDir::new("ordinary");
    copy_crate(repository.path());
    init_repository(repository.path());

    let initial_sha = git(repository.path(), &["rev-parse", "HEAD"]);
    assert_eq!(plain_probe(repository.path(), "commit"), initial_sha);
    assert_eq!(plain_probe(repository.path(), "dirty"), "clean");
    assert_eq!(
        plain_probe(repository.path(), "date"),
        "2026-08-10T01:02:03"
    );

    fs::write(repository.path().join("tracked.txt"), "modified\n").expect("modify tracked file");
    assert_eq!(plain_probe(repository.path(), "dirty"), "dirty");
    git(repository.path(), &["add", "tracked.txt"]);
    assert_eq!(plain_probe(repository.path(), "dirty"), "dirty");
    git(repository.path(), &["restore", "--staged", "tracked.txt"]);
    git(repository.path(), &["restore", "tracked.txt"]);
    assert_eq!(plain_probe(repository.path(), "dirty"), "clean");

    let untracked = repository.path().join("untracked.txt");
    fs::write(&untracked, "untracked\n").expect("write untracked file");
    assert_eq!(plain_probe(repository.path(), "dirty"), "dirty");
    fs::remove_file(untracked).expect("remove controlled untracked fixture");
    assert_eq!(plain_probe(repository.path(), "dirty"), "clean");

    commit(repository.path(), "second", "2026-08-11T04:05:06+02:00");
    let second_sha = git(repository.path(), &["rev-parse", "HEAD"]);
    assert_ne!(second_sha, initial_sha);
    assert_eq!(plain_probe(repository.path(), "commit"), second_sha);
    assert_eq!(
        plain_probe(repository.path(), "date"),
        "2026-08-11T02:05:06"
    );

    git(repository.path(), &["checkout", "--detach"]);
    commit(repository.path(), "detached", "2026-08-12T07:08:09Z");
    let detached_sha = git(repository.path(), &["rev-parse", "HEAD"]);
    assert_eq!(plain_probe(repository.path(), "commit"), detached_sha);
    assert_eq!(plain_probe(repository.path(), "dirty"), "clean");
}

#[test]
fn release_injections_obey_precedence_and_never_become_directives() {
    let repository = TempDir::new("injections");
    copy_crate(repository.path());
    init_repository(repository.path());
    let generated_sha = git(repository.path(), &["rev-parse", "HEAD"]);

    let values: [(&str, &OsStr); 5] = [
        ("PLOYZ_VERSION", OsStr::new("v9.8.7")),
        ("PLOYZ_GIT_COMMIT", OsStr::new("release-commit")),
        ("PLOYZ_GIT_DIRTY", OsStr::new("true")),
        ("PLOYZ_BUILD_DATE", OsStr::new("release-date")),
        ("PLOYZ_BUILT_BY", OsStr::new("goreleaser")),
    ];
    assert_eq!(probe(repository.path(), "version", &values, &[]), "v9.8.7");
    assert_eq!(
        probe(repository.path(), "commit", &values, &[]),
        "release-commit"
    );
    assert_eq!(probe(repository.path(), "dirty", &values, &[]), "dirty");
    assert_eq!(
        probe(repository.path(), "date", &values, &[]),
        "release-date"
    );
    assert_eq!(
        probe(repository.path(), "built-by", &values, &[]),
        "goreleaser"
    );

    let empty: [(&str, &OsStr); 5] = [
        ("PLOYZ_VERSION", OsStr::new("")),
        ("PLOYZ_GIT_COMMIT", OsStr::new("")),
        ("PLOYZ_GIT_DIRTY", OsStr::new("")),
        ("PLOYZ_BUILD_DATE", OsStr::new("")),
        ("PLOYZ_BUILT_BY", OsStr::new("")),
    ];
    assert_eq!(
        probe(repository.path(), "version", &empty, &[]),
        "999.0.0-dev"
    );
    assert_eq!(
        probe(repository.path(), "commit", &empty, &[]),
        generated_sha
    );
    assert_eq!(probe(repository.path(), "dirty", &empty, &[]), "clean");
    assert_eq!(
        probe(repository.path(), "date", &empty, &[]),
        "2026-08-10T01:02:03"
    );
    assert_eq!(probe(repository.path(), "built-by", &empty, &[]), "unknown");

    let invalid_dirty = [("PLOYZ_GIT_DIRTY", OsStr::new("invalid"))];
    assert_eq!(
        probe(repository.path(), "dirty", &invalid_dirty, &[]),
        "clean"
    );
    fs::write(repository.path().join("tracked.txt"), "dirty\n").expect("dirty tracked fixture");
    assert_eq!(
        probe(repository.path(), "dirty", &invalid_dirty, &[]),
        "dirty"
    );
    git(repository.path(), &["restore", "tracked.txt"]);

    for payload in [
        "fixed\ncargo:rustc-cfg=INJECTED",
        "fixed\rcargo:rustc-cfg=INJECTED",
        "fixed\r\ncargo:rustc-cfg=INJECTED",
    ] {
        let malicious = [("PLOYZ_GIT_COMMIT", OsStr::new(payload))];
        assert_eq!(probe(repository.path(), "commit", &malicious, &[]), payload);
        assert_eq!(
            probe(repository.path(), "injected-cfg", &malicious, &[]),
            "false",
            "package injection escaped into the Cargo directive protocol"
        );
    }
}

#[test]
fn source_archives_bad_shells_shallow_clones_and_linked_worktrees_are_safe() {
    let archive = TempDir::new("archive");
    copy_crate(archive.path());
    assert_eq!(plain_probe(archive.path(), "commit"), "unknown");
    assert_eq!(plain_probe(archive.path(), "dirty"), "unknown");
    assert_eq!(plain_probe(archive.path(), "date"), "unknown");

    let repository = TempDir::new("edge-repo");
    copy_crate(repository.path());
    init_repository(repository.path());
    assert_generated_unknown(repository.path(), &[("SHELL", OsStr::new("/bin/false"))]);

    let no_git_path = TempDir::new("no-git-path");
    for tool in ["cc", "as", "ld"] {
        std::os::unix::fs::symlink(
            Path::new("/usr/bin").join(tool),
            no_git_path.path().join(tool),
        )
        .expect("link required build tool into Git-free PATH");
    }
    let rustc = rustup_tool("rustc");
    let missing_git_environment = [
        ("PATH", no_git_path.path().as_os_str()),
        ("SHELL", OsStr::new("/bin/sh")),
        ("RUSTC", rustc.as_os_str()),
    ];
    assert_generated_unknown(repository.path(), &missing_git_environment);

    let expected_sha = git(repository.path(), &["rev-parse", "HEAD"]);
    let shallow = TempDir::new("shallow");
    run(Command::new("git")
        .args(["clone", "--depth", "1"])
        .arg(format!("file://{}", repository.path().display()))
        .arg(shallow.path()));
    assert_eq!(plain_probe(shallow.path(), "commit"), expected_sha);
    assert_eq!(plain_probe(shallow.path(), "dirty"), "clean");
    assert_eq!(plain_probe(shallow.path(), "date"), "2026-08-10T01:02:03");

    let linked = TempDir::new("linked");
    fs::remove_dir(linked.path()).expect("worktree destination must not exist");
    run(Command::new("git")
        .args(["worktree", "add", "-b", "linked"])
        .arg(linked.path())
        .current_dir(repository.path()));
    let linked_initial = git(linked.path(), &["rev-parse", "HEAD"]);
    assert_eq!(plain_probe(linked.path(), "commit"), linked_initial);
    commit(linked.path(), "linked commit", "2026-08-13T10:11:12Z");
    let linked_updated = git(linked.path(), &["rev-parse", "HEAD"]);
    assert_ne!(linked_updated, linked_initial);
    assert_eq!(plain_probe(linked.path(), "commit"), linked_updated);
    assert_eq!(plain_probe(linked.path(), "dirty"), "clean");
}
