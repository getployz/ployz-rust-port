use std::process::Command;

use ployz_internal_machine_caddyconfig::CaddyAdminClient;

const CHILD_CASE: &str = "PLOYZ_CADDY_PROVIDER_CASE";

#[test]
fn fresh_process_provider_orders_and_incompatibility() {
    let executable = std::env::current_exe().unwrap();
    for case in ["caddy-first", "docker-first", "incompatible"] {
        let output = Command::new(&executable)
            .args(["--exact", "provider_child", "--nocapture"])
            .env(CHILD_CASE, case)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "case {case} failed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn provider_child() {
    let Ok(case) = std::env::var(CHILD_CASE) else {
        return;
    };
    match case.as_str() {
        "caddy-first" => {
            CaddyAdminClient::new("/tmp/ployz-caddy-provider.sock").unwrap();
            docker_style_client();
        }
        "docker-first" => {
            docker_style_client();
            CaddyAdminClient::new("/tmp/ployz-caddy-provider.sock").unwrap();
        }
        "incompatible" => {
            let mut provider = rustls::crypto::ring::default_provider();
            provider.secure_random = &DIFFERENT_RANDOM;
            provider.install_default().unwrap();
            let error = CaddyAdminClient::new("/tmp/ployz-caddy-provider.sock").unwrap_err();
            assert_eq!(
                error.to_string(),
                "incompatible Rustls crypto provider already installed"
            );
        }
        _ => panic!("unknown provider case {case}"),
    }
}

fn docker_style_client() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    reqwest::Client::builder().build().unwrap();
}

#[derive(Debug)]
struct DifferentRandom;

impl rustls::crypto::SecureRandom for DifferentRandom {
    fn fill(&self, buffer: &mut [u8]) -> Result<(), rustls::crypto::GetRandomFailed> {
        rustls::crypto::ring::default_provider()
            .secure_random
            .fill(buffer)
    }
}

static DIFFERENT_RANDOM: DifferentRandom = DifferentRandom;
