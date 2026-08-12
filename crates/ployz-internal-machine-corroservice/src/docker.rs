use std::collections::HashMap;
use std::error::Error as StdError;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bollard::Docker;
use bollard::errors::Error as BollardError;
use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, HostConfig, HostConfigLogConfig, Mount,
    MountType, RestartPolicy, RestartPolicyNameEnum,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, RemoveContainerOptionsBuilder,
};
use futures_util::{Stream, StreamExt as _};

use crate::{Error, Result, Service, ServiceFuture, wait_ready};

pub const IMAGE: &str = "ghcr.io/unlabs-dev/corrosion:2026.6.15";
pub const CONTAINER_NAME: &str = "uncloud-corrosion";

const TIMEOUT_NOTIFY_INTERVAL: Duration = Duration::from_secs(10);
const TIMEOUT_NOTIFY_MAX: Duration = Duration::from_secs(5 * 60);
const EXTEND_TIMEOUT_MESSAGE: &str = "EXTEND_TIMEOUT_USEC=30000000";

type BoxError = Box<dyn StdError + Send + Sync>;
type EngineFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, BoxError>> + Send + 'a>>;
type PullStream = Pin<Box<dyn Stream<Item = std::result::Result<(), BoxError>> + Send>>;

trait Engine: Send + Sync {
    fn inspect<'a>(&'a self, name: &'a str) -> EngineFuture<'a, ContainerInspectResponse>;
    fn create<'a>(&'a self, name: &'a str, config: ContainerCreateBody) -> EngineFuture<'a, ()>;
    fn start<'a>(&'a self, name: &'a str) -> EngineFuture<'a, ()>;
    fn stop<'a>(&'a self, name: &'a str) -> EngineFuture<'a, ()>;
    fn restart<'a>(&'a self, name: &'a str) -> EngineFuture<'a, ()>;
    fn remove<'a>(&'a self, name: &'a str, volumes: bool) -> EngineFuture<'a, ()>;
    fn pull(&self, image: &str) -> PullStream;
}

struct BollardEngine(Docker);

impl Engine for BollardEngine {
    fn inspect<'a>(&'a self, name: &'a str) -> EngineFuture<'a, ContainerInspectResponse> {
        Box::pin(async move { self.0.inspect_container(name, None).await.map_err(boxed) })
    }

    fn create<'a>(&'a self, name: &'a str, config: ContainerCreateBody) -> EngineFuture<'a, ()> {
        Box::pin(async move {
            let options = CreateContainerOptionsBuilder::default().name(name).build();
            self.0
                .create_container(Some(options), config)
                .await
                .map(|_| ())
                .map_err(boxed)
        })
    }

    fn start<'a>(&'a self, name: &'a str) -> EngineFuture<'a, ()> {
        Box::pin(async move { self.0.start_container(name, None).await.map_err(boxed) })
    }

    fn stop<'a>(&'a self, name: &'a str) -> EngineFuture<'a, ()> {
        Box::pin(async move { self.0.stop_container(name, None).await.map_err(boxed) })
    }

    fn restart<'a>(&'a self, name: &'a str) -> EngineFuture<'a, ()> {
        Box::pin(async move { self.0.restart_container(name, None).await.map_err(boxed) })
    }

    fn remove<'a>(&'a self, name: &'a str, volumes: bool) -> EngineFuture<'a, ()> {
        Box::pin(async move {
            let options = RemoveContainerOptionsBuilder::default().v(volumes).build();
            self.0
                .remove_container(name, Some(options))
                .await
                .map_err(boxed)
        })
    }

    fn pull(&self, image: &str) -> PullStream {
        let options = CreateImageOptionsBuilder::default()
            .from_image(image)
            .build();
        let docker = self.0.clone();
        Box::pin(
            docker
                .create_image(Some(options), None, None)
                .map(|item| item.map(|_| ()).map_err(boxed)),
        )
    }
}

pub struct DockerService {
    engine: Arc<dyn Engine>,
    image: String,
    name: String,
    data_dir: PathBuf,
    run_dir: PathBuf,
    user: String,
}

impl std::fmt::Debug for DockerService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DockerService")
            .field("image", &self.image)
            .field("name", &self.name)
            .field("data_dir", &self.data_dir)
            .field("run_dir", &self.run_dir)
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

impl DockerService {
    pub fn new(
        client: Docker,
        image: impl Into<String>,
        name: impl Into<String>,
        data_dir: impl Into<PathBuf>,
        run_dir: impl Into<PathBuf>,
        user: impl Into<String>,
    ) -> Self {
        Self {
            engine: Arc::new(BollardEngine(client)),
            image: image.into(),
            name: name.into(),
            data_dir: data_dir.into(),
            run_dir: run_dir.into(),
            user: user.into(),
        }
    }

    async fn start_inner(&self) -> Result<()> {
        match self.engine.inspect(&self.name).await {
            Err(error) if is_not_found(error.as_ref()) => self.create_and_start().await?,
            Err(error) => {
                return Err(Error::boxed(
                    format!("inspect container {:?}", self.name),
                    error,
                ));
            }
            Ok(container) if container_image(&container) != Some(self.image.as_str()) => {
                if let Err(error) = self.engine.stop(&self.name).await
                    && !is_not_found(error.as_ref())
                {
                    return Err(Error::boxed(
                        format!("stop container {:?}", self.name),
                        error,
                    ));
                }
                if let Err(error) = self.engine.remove(&self.name, true).await
                    && !is_not_found(error.as_ref())
                {
                    return Err(Error::boxed(
                        format!("remove container {:?}", self.name),
                        error,
                    ));
                }
                self.create_and_start().await?;
            }
            Ok(container) if !container_running(&container) => {
                self.engine.start(&self.name).await.map_err(|error| {
                    Error::boxed(format!("start container {:?}", self.name), error)
                })?;
            }
            Ok(_) => {}
        }

        wait_ready(&self.data_dir).await
    }

    async fn create_and_start(&self) -> Result<()> {
        match self
            .engine
            .create(&self.name, self.container_config())
            .await
        {
            Ok(()) => {}
            Err(error) if is_not_found(error.as_ref()) => {
                self.pull_image().await?;
                self.engine
                    .create(&self.name, self.container_config())
                    .await
                    .map_err(|error| Error::boxed("create container", error))?;
            }
            Err(error) => return Err(Error::boxed("create container", error)),
        }
        self.engine
            .start(&self.name)
            .await
            .map_err(|error| Error::boxed("start container", error))
    }

    async fn pull_image(&self) -> Result<()> {
        self.pull_image_with_schedule(
            NotificationSchedule::production(),
            notify_systemd,
            |error| {
                tracing::warn!(
                    error = %error,
                    "failed to extend the systemd service start timeout while starting Corrosion"
                );
            },
        )
        .await
    }

    async fn pull_image_with_schedule<N, D>(
        &self,
        schedule: NotificationSchedule,
        mut notify: N,
        mut diagnostic: D,
    ) -> Result<()>
    where
        N: FnMut() -> std::result::Result<bool, BoxError>,
        D: FnMut(&(dyn StdError + Send + Sync)),
    {
        let mut pull = self.engine.pull(&self.image);
        let started = Instant::now();
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + schedule.interval,
            schedule.interval,
        );
        let mut notifying = true;

        loop {
            tokio::select! {
                item = pull.next() => match item {
                    Some(Ok(())) => {}
                    Some(Err(error)) => return Err(Error::boxed("read pull response", error)),
                    None => return Ok(()),
                },
                _ = ticker.tick(), if notifying => {
                    if started.elapsed() >= schedule.maximum {
                        notifying = false;
                        continue;
                    }
                    match notify() {
                        Ok(true) => {}
                        Ok(false) => notifying = false,
                        Err(error) => {
                            diagnostic(error.as_ref());
                            notifying = false;
                        }
                    }
                }
            }
        }
    }

    fn container_config(&self) -> ContainerCreateBody {
        let mut labels = HashMap::new();
        labels.insert(
            ployz_pkg_api::LABEL_DAEMON_MANAGED.to_owned(),
            String::new(),
        );
        ContainerCreateBody {
            image: Some(self.image.clone()),
            cmd: Some(vec![
                "corrosion".into(),
                "agent".into(),
                "-c".into(),
                self.data_dir
                    .join("config.toml")
                    .to_string_lossy()
                    .into_owned(),
            ]),
            user: Some(self.user.clone()),
            labels: Some(labels),
            host_config: Some(HostConfig {
                network_mode: Some("host".into()),
                restart_policy: Some(RestartPolicy {
                    name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                    ..RestartPolicy::default()
                }),
                log_config: Some(HostConfigLogConfig {
                    typ: Some("local".into()),
                    ..HostConfigLogConfig::default()
                }),
                mounts: Some(vec![bind_mount(&self.data_dir), bind_mount(&self.run_dir)]),
                ..HostConfig::default()
            }),
            ..ContainerCreateBody::default()
        }
    }
}

impl Service for DockerService {
    fn start(&self) -> ServiceFuture<'_, ()> {
        Box::pin(self.start_inner())
    }

    fn stop(&self) -> ServiceFuture<'_, ()> {
        Box::pin(async {
            match self.engine.stop(&self.name).await {
                Ok(()) => Ok(()),
                Err(error) if is_not_found(error.as_ref()) => Ok(()),
                Err(error) => Err(Error::boxed(
                    format!("stop container {:?}", self.name),
                    error,
                )),
            }
        })
    }

    fn restart(&self) -> ServiceFuture<'_, ()> {
        Box::pin(async {
            self.engine
                .restart(&self.name)
                .await
                .map_err(|error| Error::boxed(format!("restart container {:?}", self.name), error))
        })
    }

    fn cleanup(&self) -> ServiceFuture<'_, ()> {
        Box::pin(async {
            match self.engine.stop(&self.name).await {
                Err(error) if is_not_found(error.as_ref()) => return Ok(()),
                Err(error) => {
                    return Err(Error::boxed(
                        format!("stop container {:?}", self.name),
                        error,
                    ));
                }
                Ok(()) => {}
            }
            self.engine
                .remove(&self.name, true)
                .await
                .map_err(|error| Error::boxed(format!("remove container {:?}", self.name), error))
        })
    }

    fn running(&self) -> ServiceFuture<'_, bool> {
        Box::pin(async {
            Ok(self
                .engine
                .inspect(&self.name)
                .await
                .ok()
                .as_ref()
                .is_some_and(container_running))
        })
    }
}

#[derive(Clone, Copy)]
struct NotificationSchedule {
    interval: Duration,
    maximum: Duration,
}

impl NotificationSchedule {
    const fn production() -> Self {
        Self {
            interval: TIMEOUT_NOTIFY_INTERVAL,
            maximum: TIMEOUT_NOTIFY_MAX,
        }
    }
}

fn bind_mount(path: &Path) -> Mount {
    let path = path.to_string_lossy().into_owned();
    Mount {
        typ: Some(MountType::BIND),
        source: Some(path.clone()),
        target: Some(path),
        ..Mount::default()
    }
}

fn container_image(container: &ContainerInspectResponse) -> Option<&str> {
    container
        .config
        .as_ref()
        .and_then(|config| config.image.as_deref())
}

fn container_running(container: &ContainerInspectResponse) -> bool {
    container
        .state
        .as_ref()
        .and_then(|state| state.running)
        .unwrap_or(false)
}

fn boxed(error: BollardError) -> BoxError {
    Box::new(error)
}

fn is_not_found(error: &(dyn StdError + 'static)) -> bool {
    error.downcast_ref::<BollardError>().is_some_and(|error| {
        matches!(
            error,
            BollardError::DockerResponseServerError {
                status_code: 404,
                ..
            }
        )
    })
}

#[cfg(target_os = "linux")]
fn notify_systemd() -> std::result::Result<bool, BoxError> {
    use libsystemd::daemon::{self, NotifyState};

    daemon::notify(
        false,
        &[NotifyState::Other(EXTEND_TIMEOUT_MESSAGE.to_owned())],
    )
    .map_err(|error| Box::new(error) as BoxError)
}

#[cfg(not(target_os = "linux"))]
fn notify_systemd() -> std::result::Result<bool, BoxError> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::fmt;
    use std::process::Command;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use bollard::models::{ContainerConfig, ContainerState};
    use futures_util::stream;

    struct PendingPull {
        dropped: Arc<AtomicBool>,
    }

    impl Stream for PendingPull {
        type Item = std::result::Result<(), BoxError>;

        fn poll_next(
            self: Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            std::task::Poll::Pending
        }
    }

    impl Drop for PendingPull {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    #[derive(Debug)]
    struct TestError(&'static str);

    impl fmt::Display for TestError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.0)
        }
    }

    impl StdError for TestError {}

    #[derive(Default)]
    struct FakeEngine {
        calls: Mutex<Vec<String>>,
        inspections: Mutex<VecDeque<std::result::Result<ContainerInspectResponse, BoxError>>>,
        creates: Mutex<VecDeque<std::result::Result<(), BoxError>>>,
        pulls: Mutex<VecDeque<PullStream>>,
    }

    impl FakeEngine {
        fn record(&self, call: impl Into<String>) {
            self.calls.lock().unwrap().push(call.into());
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Engine for FakeEngine {
        fn inspect<'a>(&'a self, _name: &'a str) -> EngineFuture<'a, ContainerInspectResponse> {
            self.record("inspect");
            let result = self.inspections.lock().unwrap().pop_front().unwrap();
            Box::pin(async move { result })
        }

        fn create<'a>(
            &'a self,
            _name: &'a str,
            config: ContainerCreateBody,
        ) -> EngineFuture<'a, ()> {
            self.record(format!("create:{config:?}"));
            let result = self.creates.lock().unwrap().pop_front().unwrap_or(Ok(()));
            Box::pin(async move { result })
        }

        fn start<'a>(&'a self, _name: &'a str) -> EngineFuture<'a, ()> {
            self.record("start");
            Box::pin(async { Ok(()) })
        }

        fn stop<'a>(&'a self, _name: &'a str) -> EngineFuture<'a, ()> {
            self.record("stop");
            Box::pin(async { Ok(()) })
        }

        fn restart<'a>(&'a self, _name: &'a str) -> EngineFuture<'a, ()> {
            self.record("restart");
            Box::pin(async { Ok(()) })
        }

        fn remove<'a>(&'a self, _name: &'a str, volumes: bool) -> EngineFuture<'a, ()> {
            self.record(format!("remove:{volumes}"));
            Box::pin(async { Ok(()) })
        }

        fn pull(&self, _image: &str) -> PullStream {
            self.record("pull");
            self.pulls.lock().unwrap().pop_front().unwrap()
        }
    }

    fn missing() -> BoxError {
        Box::new(BollardError::DockerResponseServerError {
            status_code: 404,
            message: "not found".into(),
        })
    }

    fn inspection(image: &str, running: bool) -> ContainerInspectResponse {
        ContainerInspectResponse {
            config: Some(ContainerConfig {
                image: Some(image.into()),
                ..ContainerConfig::default()
            }),
            state: Some(ContainerState {
                running: Some(running),
                ..ContainerState::default()
            }),
            ..ContainerInspectResponse::default()
        }
    }

    fn service(engine: Arc<FakeEngine>) -> DockerService {
        DockerService {
            engine,
            image: "image:new".into(),
            name: "corrosion".into(),
            data_dir: "/data/corrosion".into(),
            run_dir: "/run/corrosion".into(),
            user: "123:456".into(),
        }
    }

    #[test]
    fn container_configuration_matches_managed_host_service_contract() {
        let engine = Arc::new(FakeEngine::default());
        let config = service(engine).container_config();
        assert_eq!(config.image.as_deref(), Some("image:new"));
        assert_eq!(config.user.as_deref(), Some("123:456"));
        assert_eq!(
            config.cmd.unwrap(),
            ["corrosion", "agent", "-c", "/data/corrosion/config.toml"]
        );
        assert_eq!(
            config
                .labels
                .unwrap()
                .get(ployz_pkg_api::LABEL_DAEMON_MANAGED),
            Some(&String::new())
        );
        let host = config.host_config.unwrap();
        assert_eq!(host.network_mode.as_deref(), Some("host"));
        assert_eq!(
            host.restart_policy.unwrap().name,
            Some(RestartPolicyNameEnum::UNLESS_STOPPED)
        );
        assert_eq!(host.log_config.unwrap().typ.as_deref(), Some("local"));
        assert_eq!(
            host.mounts.unwrap(),
            [
                bind_mount(Path::new("/data/corrosion")),
                bind_mount(Path::new("/run/corrosion"))
            ]
        );
    }

    #[tokio::test]
    async fn missing_image_is_pulled_then_container_is_created_again_and_started() {
        let engine = Arc::new(FakeEngine::default());
        engine
            .creates
            .lock()
            .unwrap()
            .extend([Err(missing()), Ok(())]);
        engine
            .pulls
            .lock()
            .unwrap()
            .push_back(Box::pin(stream::empty()));
        let service = service(engine.clone());
        service.create_and_start().await.unwrap();
        let calls = engine.calls();
        assert_eq!(calls[0].split(':').next(), Some("create"));
        assert_eq!(calls[1], "pull");
        assert_eq!(calls[2].split(':').next(), Some("create"));
        assert_eq!(calls[3], "start");
    }

    #[tokio::test]
    async fn image_change_stops_and_removes_old_container_before_recreation() {
        let engine = Arc::new(FakeEngine::default());
        engine
            .inspections
            .lock()
            .unwrap()
            .push_back(Ok(inspection("image:old", true)));
        engine.creates.lock().unwrap().push_back(Ok(()));
        let service = service(engine.clone());
        // Readiness fails after the lifecycle calls because this fixture has no config file.
        assert!(service.start_inner().await.is_err());
        let calls = engine.calls();
        assert_eq!(&calls[..3], ["inspect", "stop", "remove:true"]);
        assert_eq!(calls[3].split(':').next(), Some("create"));
        assert_eq!(calls[4], "start");
    }

    #[tokio::test]
    async fn existing_stopped_container_is_started_but_running_container_is_not() {
        for (running, expected) in [(false, vec!["inspect", "start"]), (true, vec!["inspect"])] {
            let engine = Arc::new(FakeEngine::default());
            engine
                .inspections
                .lock()
                .unwrap()
                .push_back(Ok(inspection("image:new", running)));
            let service = service(engine.clone());
            assert!(service.start_inner().await.is_err());
            assert_eq!(engine.calls(), expected);
        }
    }

    #[tokio::test]
    async fn timeout_notifications_stop_on_failure_without_failing_pull() {
        let engine = Arc::new(FakeEngine::default());
        engine
            .pulls
            .lock()
            .unwrap()
            .push_back(Box::pin(stream::once(async {
                tokio::time::sleep(Duration::from_millis(80)).await;
                Ok(())
            })));
        let service = service(engine);
        let notifications = Arc::new(Mutex::new(0_u32));
        let observed = notifications.clone();
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let observed_diagnostics = diagnostics.clone();
        service
            .pull_image_with_schedule(
                NotificationSchedule {
                    interval: Duration::from_millis(10),
                    maximum: Duration::from_millis(60),
                },
                move || {
                    let mut count = observed.lock().unwrap();
                    *count += 1;
                    if *count == 2 {
                        Err(Box::new(TestError("notify failed")))
                    } else {
                        Ok(true)
                    }
                },
                move |error| {
                    observed_diagnostics.lock().unwrap().push(error.to_string());
                },
            )
            .await
            .unwrap();
        assert_eq!(*notifications.lock().unwrap(), 2);
        assert_eq!(&*diagnostics.lock().unwrap(), &["notify failed"]);
    }

    #[tokio::test]
    async fn timeout_notifications_are_capped_and_pull_completion_is_prompt() {
        let engine = Arc::new(FakeEngine::default());
        engine
            .pulls
            .lock()
            .unwrap()
            .push_back(Box::pin(stream::once(async {
                tokio::time::sleep(Duration::from_millis(75)).await;
                Ok(())
            })));
        let service = service(engine);
        let notifications = Arc::new(Mutex::new(0_u32));
        let observed = notifications.clone();
        let started = Instant::now();
        service
            .pull_image_with_schedule(
                NotificationSchedule {
                    interval: Duration::from_millis(10),
                    maximum: Duration::from_millis(35),
                },
                move || {
                    *observed.lock().unwrap() += 1;
                    Ok(true)
                },
                |_| {},
            )
            .await
            .unwrap();
        assert_eq!(*notifications.lock().unwrap(), 3);
        assert!(started.elapsed() < Duration::from_millis(250));
    }

    #[tokio::test]
    async fn cancelling_pull_drops_the_stream_without_waiting_for_the_notify_tick() {
        let engine = Arc::new(FakeEngine::default());
        let dropped = Arc::new(AtomicBool::new(false));
        engine
            .pulls
            .lock()
            .unwrap()
            .push_back(Box::pin(PendingPull {
                dropped: dropped.clone(),
            }));
        let service = service(engine);
        let result = tokio::time::timeout(
            Duration::from_millis(20),
            service.pull_image_with_schedule(
                NotificationSchedule::production(),
                || Ok(true),
                |_| {},
            ),
        )
        .await;
        assert!(result.is_err());
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn systemd_notification_supports_abstract_socket_and_preserves_environment() {
        const HELPER: &str = "PLOYZ_CORROSERVICE_NOTIFY_HELPER";
        if std::env::var_os(HELPER).is_some() {
            use std::os::linux::net::SocketAddrExt as _;
            use std::os::unix::net::{SocketAddr, UnixDatagram};

            let notify_socket = std::env::var("NOTIFY_SOCKET").unwrap();
            let name = notify_socket.strip_prefix('@').unwrap();
            let address = SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
            let socket = UnixDatagram::bind_addr(&address).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            assert_eq!(std::env::var("NOTIFY_SOCKET").unwrap(), notify_socket);
            assert!(notify_systemd().unwrap());
            assert_eq!(std::env::var("NOTIFY_SOCKET").unwrap(), notify_socket);
            let mut message = [0_u8; 128];
            let length = socket.recv(&mut message).unwrap();
            assert_eq!(&message[..length], b"EXTEND_TIMEOUT_USEC=30000000\n");
            return;
        }

        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "docker::tests::systemd_notification_supports_abstract_socket_and_preserves_environment",
                "--nocapture",
            ])
            .env(HELPER, "1")
            .env(
                "NOTIFY_SOCKET",
                format!("@ployz-corroservice-notify-{}", std::process::id()),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "helper failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn stop_and_cleanup_ignore_a_missing_container() {
        let engine = Arc::new(FakeEngine::default());
        struct MissingEngine(Arc<FakeEngine>);
        impl Engine for MissingEngine {
            fn inspect<'a>(&'a self, name: &'a str) -> EngineFuture<'a, ContainerInspectResponse> {
                self.0.inspect(name)
            }
            fn create<'a>(
                &'a self,
                name: &'a str,
                config: ContainerCreateBody,
            ) -> EngineFuture<'a, ()> {
                self.0.create(name, config)
            }
            fn start<'a>(&'a self, name: &'a str) -> EngineFuture<'a, ()> {
                self.0.start(name)
            }
            fn stop<'a>(&'a self, _name: &'a str) -> EngineFuture<'a, ()> {
                Box::pin(async { Err(missing()) })
            }
            fn restart<'a>(&'a self, name: &'a str) -> EngineFuture<'a, ()> {
                self.0.restart(name)
            }
            fn remove<'a>(&'a self, name: &'a str, volumes: bool) -> EngineFuture<'a, ()> {
                self.0.remove(name, volumes)
            }
            fn pull(&self, image: &str) -> PullStream {
                self.0.pull(image)
            }
        }
        let mut service = service(engine.clone());
        service.engine = Arc::new(MissingEngine(engine.clone()));
        service.stop().await.unwrap();
        service.cleanup().await.unwrap();
        assert!(!engine.calls().iter().any(|call| call.starts_with("remove")));
    }

    #[test]
    fn constants_preserve_the_oracle_values() {
        assert_eq!(EXTEND_TIMEOUT_MESSAGE, "EXTEND_TIMEOUT_USEC=30000000");
        assert_eq!(TIMEOUT_NOTIFY_INTERVAL, Duration::from_secs(10));
        assert_eq!(TIMEOUT_NOTIFY_MAX, Duration::from_secs(300));
    }
}
