use ployz_internal_docker::{Cancellation, Client, DaemonConfig, PullOptions};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let config = DaemonConfig::from_env().expect("parse configured TLS Docker endpoint");
    let client = Client::connect(&config, &Cancellation::new())
        .await
        .expect("construct configured TLS Docker clients");
    let _progress = client
        .pull_image(
            "busybox",
            PullOptions {
                registry_auth: Some("acceptance-token".to_owned()),
                ..Default::default()
            },
            Cancellation::new(),
        )
        .await
        .expect("send an authenticated image request with configured mutual TLS");
}
