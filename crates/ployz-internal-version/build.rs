#[path = "src/build_support.rs"]
mod build_support;

use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::process::Command;

use vergen_gitcl::{Emitter, Gitcl};

fn main() -> Result<(), Box<dyn Error>> {
    let sentinel = build_support::missing_sentinel(env::var_os("OUT_DIR"))?;
    println!("cargo:rerun-if-changed={sentinel}");

    emit_rustc_version(env::var_os("RUSTC"))?;

    let git = Gitcl::builder()
        .sha(false)
        .dirty(true)
        .commit_timestamp(true)
        .build();
    Emitter::default().add_instructions(&git)?.emit()?;

    Ok(())
}

fn emit_rustc_version(rustc: Option<OsString>) -> Result<(), Box<dyn Error>> {
    let rustc = rustc.ok_or("Cargo did not provide RUSTC")?;
    let output = Command::new(rustc).arg("--version").output()?;
    if !output.status.success() {
        return Err("rustc --version failed".into());
    }

    let version = String::from_utf8(output.stdout)?;
    let version = version.trim_end_matches(['\r', '\n']);
    let version = build_support::single_line(version)?;
    println!("cargo:rustc-env=PLOYZ_RUSTC_VERSION={version}");
    println!("cargo:rerun-if-env-changed=RUSTC");
    Ok(())
}
