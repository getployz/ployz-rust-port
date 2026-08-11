use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const PROTOS: &[&str] = &[
    "proto/internal/machine/api/pb/caddy.proto",
    "proto/internal/machine/api/pb/cluster.proto",
    "proto/internal/machine/api/pb/common.proto",
    "proto/internal/machine/api/pb/docker.proto",
    "proto/internal/machine/api/pb/machine.proto",
    "proto/google/rpc/status.proto",
];

fn main() {
    println!("cargo:rerun-if-env-changed=PLOYZ_PROTO_REGENERATE");
    println!("cargo:rerun-if-env-changed=PLOYZ_PROTO_VERIFY");
    for proto in PROTOS {
        println!("cargo:rerun-if-changed={proto}");
    }
    println!("cargo:rerun-if-changed=src/generated/api.rs");
    println!("cargo:rerun-if-changed=src/generated/google.rpc.rs");
    println!("cargo:rerun-if-changed=schema/ployz-api.pb");

    let regenerate = env::var_os("PLOYZ_PROTO_REGENERATE").is_some();
    let verify = env::var_os("PLOYZ_PROTO_VERIFY").is_some();
    if !regenerate && !verify {
        return;
    }

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"))
        .join("ployz-proto-snapshot");
    if out.exists() {
        fs::remove_dir_all(&out).expect("remove prior generated snapshot");
    }
    fs::create_dir_all(&out).expect("create generated snapshot directory");

    let descriptor = out.join("ployz-api.pb");
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .btree_map(".")
        .file_descriptor_set_path(&descriptor)
        .out_dir(&out)
        .compile_protos(PROTOS, &["proto"])
        .expect("generate the Ployz machine API snapshot");

    let generated = [
        (out.join("api.rs"), manifest.join("src/generated/api.rs")),
        (
            out.join("google.rpc.rs"),
            manifest.join("src/generated/google.rpc.rs"),
        ),
        (descriptor, manifest.join("schema/ployz-api.pb")),
    ];

    if regenerate {
        for (actual, checked) in &generated {
            if let Some(parent) = checked.parent() {
                fs::create_dir_all(parent).expect("create checked snapshot directory");
            }
            fs::copy(actual, checked).expect("install generated snapshot");
        }
    } else {
        for (actual, checked) in &generated {
            assert_same(actual, checked).unwrap_or_else(|error| {
                panic!(
                    "checked protobuf snapshot differs for {}: {error}; regenerate with \
                     PLOYZ_PROTO_REGENERATE=1",
                    checked.display()
                )
            });
        }
    }
}

fn assert_same(actual: &Path, checked: &Path) -> io::Result<()> {
    let actual_bytes = fs::read(actual)?;
    let checked_bytes = fs::read(checked)?;
    if actual_bytes == checked_bytes {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "generated {} bytes, checked snapshot has {} bytes",
            actual_bytes.len(),
            checked_bytes.len()
        )))
    }
}
