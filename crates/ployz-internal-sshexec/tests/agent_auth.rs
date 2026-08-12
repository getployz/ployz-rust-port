use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ployz_internal_sshexec::connect;
use russh::keys::ssh_key::{Algorithm, LineEnding};
use russh::server::{self, Auth};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::timeout;

#[derive(Clone, Copy)]
struct AcceptPublicKey;

impl server::Handler for AcceptPublicKey {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        _public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }
}

struct AgentEnvironment {
    socket: String,
    pid: String,
}

impl AgentEnvironment {
    fn start() -> Self {
        let output = Command::new("ssh-agent").arg("-s").output().unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        let socket = assignment(&text, "SSH_AUTH_SOCK");
        let pid = assignment(&text, "SSH_AGENT_PID");
        // SAFETY: this integration test is its own process and mutates the
        // environment before starting any worker tasks.
        unsafe {
            std::env::set_var("SSH_AUTH_SOCK", &socket);
            std::env::set_var("SSH_AGENT_PID", &pid);
        }
        Self { socket, pid }
    }
}

impl Drop for AgentEnvironment {
    fn drop(&mut self) {
        let _ = Command::new("ssh-agent")
            .arg("-k")
            .env("SSH_AUTH_SOCK", &self.socket)
            .env("SSH_AGENT_PID", &self.pid)
            .output();
    }
}

fn assignment(output: &str, name: &str) -> String {
    output
        .split(';')
        .find_map(|field| field.trim().strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("ssh-agent output omitted {name}: {output}"))
        .to_owned()
}

#[tokio::test]
async fn agent_held_rsa_authenticates_without_a_file_fallback() {
    use std::os::unix::fs::PermissionsExt;

    let agent = AgentEnvironment::start();
    let mut rng = russh::keys::key::safe_rng();
    let host_key = russh::keys::PrivateKey::random(&mut rng, Algorithm::Ed25519).unwrap();
    let agent_key =
        russh::keys::PrivateKey::random(&mut rng, Algorithm::Rsa { hash: None }).unwrap();
    let key_path = unique_key_path();
    std::fs::write(
        &key_path,
        agent_key.to_openssh(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let added = Command::new("ssh-add").arg(&key_path).status().unwrap();
    assert!(added.success(), "ssh-add rejected the generated RSA key");

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Arc::new(server::Config {
        auth_rejection_time: Duration::ZERO,
        keys: vec![host_key],
        ..server::Config::default()
    });
    let (finished_tx, finished) = oneshot::channel();
    tokio::spawn(async move {
        let result = async {
            let (stream, _) = listener.accept().await?;
            server::run_stream(config, stream, AcceptPublicKey)
                .await?
                .await
        }
        .await;
        assert!(result.is_ok(), "test SSH server failed: {result:?}");
        let _ = finished_tx.send(());
    });

    let client = connect("tester", "127.0.0.1", port, PathBuf::new())
        .await
        .expect("agent-held RSA authentication failed");
    client.close().await.unwrap();
    timeout(Duration::from_secs(3), finished)
        .await
        .expect("server connection task did not terminate")
        .unwrap();
    std::fs::remove_file(key_path).unwrap();
    drop(agent);
}

fn unique_key_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "ployz-agent-rsa-{}-{nonce}.key",
        std::process::id()
    ))
}
