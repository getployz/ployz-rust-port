use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ployz_internal_log::TextLayer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("buffer lock").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn temporary_overlay_path() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "ployz-log-go-overlay-{}-{unique}.json",
        std::process::id()
    ))
}

fn go_oracle_output(repository: &Path) -> String {
    let overlay_path = temporary_overlay_path();
    let go_cache = overlay_path.with_extension("go-cache");
    let virtual_test = repository.join("upstream/uncloud/internal/log/ployz_oracle_test.go");
    let test_source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle_log_test.go");
    let overlay = format!(
        "{{\"Replace\":{{\"{}\":\"{}\"}}}}",
        virtual_test.display(),
        test_source.display()
    );
    fs::write(&overlay_path, overlay).expect("write Go overlay");
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
            "-run=^TestPloyzLogOracle$",
            "-count=1",
            "-v",
            "./internal/log",
        ])
        .output()
        .expect("run pinned Go log oracle");
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
    String::from_utf8(bytes).expect("oracle log is UTF-8")
}

fn rust_output() -> String {
    let writer = Buffer::default();
    let captured = writer.0.clone();
    let subscriber =
        tracing_subscriber::registry().with(TextLayer::new(writer, LevelFilter::DEBUG));

    tracing::subscriber::with_default(subscriber, || {
        let component = tracing::info_span!("component", component = "dns server");
        let _component = component.enter();
        let request =
            tracing::info_span!("request", ployz.group = "request", name = "example.org.");
        let _request = request.enter();
        let details = tracing::info_span!("details", ployz.group = "details");
        let _details = details.enter();

        tracing::debug!(
            kind = "A",
            empty = "",
            quoted = "a=b",
            line = "a\nb",
            ok = true,
            signed = -7_i64,
            unsigned = 8_u64,
            small = 0.00001_f64,
            large = 1_000_000_f64,
            inf = f64::INFINITY,
            time = "removed",
            level = "removed",
            msg = "removed",
            "received"
        );
        tracing::info!("no fields");
        tracing::warn!(
            unicode = "hello-world",
            combining = "x\u{301}",
            go_unassigned = "x\u{88f}",
            zero_width = "a\u{200b}b",
            "warning"
        );
        tracing::error!(err = %format_args!("bad value: {}", 3), "failure");
    });

    let empty_subscriber = tracing_subscriber::registry()
        .with(TextLayer::new(writer_for(&captured), LevelFilter::DEBUG));
    tracing::subscriber::with_default(empty_subscriber, || {
        let empty = tracing::info_span!("empty", ployz.group = "");
        let _empty = empty.enter();
        tracing::info!(key = "value", "root empty");
        let parent = tracing::info_span!("parent", ployz.group = "parent");
        let _parent = parent.enter();
        let nested = tracing::info_span!("nested", ployz.group = "");
        let _nested = nested.enter();
        tracing::info!(key = "value", "nested empty");
    });

    let bytes = captured.lock().expect("buffer lock").clone();
    String::from_utf8(bytes).expect("Rust log is UTF-8")
}

fn writer_for(captured: &Arc<Mutex<Vec<u8>>>) -> Buffer {
    Buffer(Arc::clone(captured))
}

#[test]
fn formatting_matches_the_pinned_go_oracle() {
    let repository = std::env::var_os("PLOYZ_REPOSITORY")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    assert_eq!(rust_output(), go_oracle_output(&repository));
}
