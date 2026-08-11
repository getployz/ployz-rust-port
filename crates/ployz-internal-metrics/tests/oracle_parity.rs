use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use ployz_internal_metrics::{OK, metrics, registry};
use prometheus::{Encoder, TextEncoder};

fn temporary_overlay_path() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "ployz-metrics-go-overlay-{}-{unique}.json",
        std::process::id()
    ))
}

fn go_oracle_text(repository: &Path) -> String {
    let overlay_path = temporary_overlay_path();
    let go_cache = overlay_path.with_extension("go-cache");
    let virtual_test = repository
        .join("upstream/uncloud/internal/metrics/ployz_oracle_test.go")
        .canonicalize()
        .unwrap_or_else(|_| {
            repository.join("upstream/uncloud/internal/metrics/ployz_oracle_test.go")
        });
    let test_source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle_metrics_test.go");
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
            "-run=^TestPloyzMetricsOracle$",
            "-count=1",
            "-v",
            "./internal/metrics",
        ])
        .output()
        .expect("run pinned Go metrics oracle");
    fs::remove_file(&overlay_path).expect("remove Go overlay");
    fs::remove_dir_all(&go_cache).expect("remove isolated Go build cache");

    assert!(
        output.status.success(),
        "Go oracle failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("Go test output is UTF-8");
    stdout
        .split_once("PLOYZ_ORACLE_BEGIN\n")
        .and_then(|(_, rest)| rest.split_once("PLOYZ_ORACLE_END\n"))
        .map(|(metrics, _)| metrics.to_owned())
        .expect("Go oracle emitted metric markers")
}

fn rust_metrics_text() -> String {
    metrics().build_info("v1.2.3").set(1.0);
    metrics().dns_query("false", OK).inc();

    let families = registry()
        .gather()
        .into_iter()
        .filter(|family| family.name().starts_with("ployz_"))
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    TextEncoder::new()
        .encode(&families, &mut output)
        .expect("encode Rust metrics");
    String::from_utf8(output).expect("Prometheus text is UTF-8")
}

#[test]
fn rust_contract_differs_only_by_the_product_rename() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let renamed_oracle = go_oracle_text(&repository)
        .replace("uncloud_dns_", "ployz_dns_")
        .replace("uncloud_uncloudd_", "ployz_ployzd_");

    assert_eq!(rust_metrics_text(), renamed_oracle);
}
