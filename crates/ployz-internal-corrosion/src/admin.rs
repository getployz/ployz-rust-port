use std::{
    collections::HashMap,
    fmt,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use serde_json::{Map, Number, Value};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::UnixStream,
};

use crate::transport::{ClientError, ClientErrorKind};

const MEMBERSHIP_STATES_COMMAND: &[u8] = br#"{"Cluster":"MembershipStates"}"#;
const MEMBER_RTTS_COMMAND: &[u8] = br#"{"Cluster":"Members"}"#;

#[derive(Clone, Debug)]
pub struct AdminClient {
    socket_path: PathBuf,
}

impl AdminClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub async fn cluster_membership_states(
        &self,
        latest: bool,
    ) -> Result<Vec<ClusterMembershipState>, BatchError<Vec<ClusterMembershipState>>> {
        let mut responses = self
            .send_command(MEMBERSHIP_STATES_COMMAND)
            .await
            .map_err(BatchError::fatal)?;
        let mut states = Vec::new();
        let mut errors = Vec::new();
        while let Some(response) = responses.next().await {
            let response = response.map_err(BatchError::fatal)?;
            match parse_cluster_membership_state(&response) {
                Ok(state) => states.push(state),
                Err(error) => errors.push(error),
            }
        }

        if latest {
            let mut by_id = HashMap::new();
            for state in states {
                let replace =
                    by_id
                        .get(&state.id)
                        .is_none_or(|existing: &ClusterMembershipState| {
                            existing.timestamp < state.timestamp
                        });
                if replace {
                    by_id.insert(state.id.clone(), state);
                }
            }
            states = by_id.into_values().collect();
        }

        if errors.is_empty() {
            Ok(states)
        } else {
            Err(BatchError {
                partial: states,
                errors,
            })
        }
    }

    pub async fn cluster_member_rtts(
        &self,
    ) -> Result<Vec<MemberRttStats>, BatchError<Vec<MemberRttStats>>> {
        let mut responses = self
            .send_command(MEMBER_RTTS_COMMAND)
            .await
            .map_err(BatchError::fatal)?;
        let mut stats = Vec::new();
        let mut errors = Vec::new();
        while let Some(response) = responses.next().await {
            let response = response.map_err(BatchError::fatal)?;
            match parse_cluster_member_rtt(&response) {
                Ok((address, samples)) if !samples.is_empty() => {
                    let (median, standard_deviation) = compute_rtt_stats_ms(&samples);
                    stats.push(MemberRttStats {
                        address,
                        median: Duration::from_nanos((median * 1_000_000.0) as u64),
                        standard_deviation: Duration::from_nanos(
                            (standard_deviation * 1_000_000.0) as u64,
                        ),
                    });
                }
                Ok(_) => {}
                Err(error) => errors.push(error),
            }
        }

        if errors.is_empty() {
            Ok(stats)
        } else {
            Err(BatchError {
                partial: stats,
                errors,
            })
        }
    }

    /// Sends one length-delimited command and returns its incremental response reader.
    /// Dropping the reader closes the Unix connection.
    pub async fn send_command(&self, command: &[u8]) -> Result<AdminResponses, ClientError> {
        let mut connection = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|error| {
                ClientError::with_source(ClientErrorKind::Admin, "connect to admin socket", error)
            })?;
        connection
            .write_all(&encode_frame(command))
            .await
            .map_err(|error| {
                ClientError::with_source(ClientErrorKind::Admin, "send command", error)
            })?;
        Ok(AdminResponses {
            connection,
            finished: false,
            frame_head: [0; 4],
            frame_head_read: 0,
            frame_data: None,
            frame_data_read: 0,
        })
    }
}

/// Pull-driven responses from a Corrosion admin command.
pub struct AdminResponses {
    connection: UnixStream,
    finished: bool,
    frame_head: [u8; 4],
    frame_head_read: usize,
    frame_data: Option<Vec<u8>>,
    frame_data_read: usize,
}

impl fmt::Debug for AdminResponses {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdminResponses")
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl AdminResponses {
    pub async fn next(&mut self) -> Option<Result<Map<String, Value>, ClientError>> {
        if self.finished {
            return None;
        }
        loop {
            let data = match self.read_frame().await {
                Ok(data) => data,
                Err(error) => {
                    self.finished = true;
                    return Some(Err(error));
                }
            };
            let mut decoded: Value = match serde_json::from_slice(&data) {
                Ok(decoded) => decoded,
                Err(error) => {
                    self.finished = true;
                    return Some(Err(ClientError::with_source(
                        ClientErrorKind::Json,
                        "unmarshal response",
                        error,
                    )));
                }
            };
            normalize_numbers_to_f64(&mut decoded);
            match decoded {
                Value::String(value) if value == "Success" => {
                    self.finished = true;
                    return None;
                }
                Value::Object(object) => {
                    if let Some(Value::Object(error)) = object.get("Error") {
                        self.finished = true;
                        if let Some(Value::String(message)) = error.get("msg") {
                            return Some(Err(ClientError::new(ClientErrorKind::Admin, message)));
                        }
                        return Some(Err(ClientError::new(
                            ClientErrorKind::Admin,
                            format!("invalid error response: {error:?}"),
                        )));
                    }
                    if let Some(Value::Object(json)) = object.get("Json") {
                        return Some(Ok(json.clone()));
                    }
                }
                _ => {}
            }
        }
    }

    /// Uses cancellation-safe `read` calls and retains every consumed byte in
    /// `self`, so cancelling `next` cannot desynchronize the frame boundary.
    async fn read_frame(&mut self) -> Result<Vec<u8>, ClientError> {
        while self.frame_head_read < self.frame_head.len() {
            let read = self
                .connection
                .read(&mut self.frame_head[self.frame_head_read..])
                .await
                .map_err(|error| {
                    ClientError::with_source(ClientErrorKind::Admin, "read frame head", error)
                })?;
            if read == 0 {
                return Err(ClientError::new(
                    ClientErrorKind::Admin,
                    "read frame head: unexpected EOF",
                ));
            }
            self.frame_head_read += read;
        }

        if self.frame_data.is_none() {
            self.frame_data = Some(vec![0; u32::from_be_bytes(self.frame_head) as usize]);
            self.frame_data_read = 0;
        }
        let data = self.frame_data.as_mut().expect("frame data initialized");
        while self.frame_data_read < data.len() {
            let read = self
                .connection
                .read(&mut data[self.frame_data_read..])
                .await
                .map_err(|error| {
                    ClientError::with_source(ClientErrorKind::Admin, "read frame data", error)
                })?;
            if read == 0 {
                return Err(ClientError::new(
                    ClientErrorKind::Admin,
                    "read frame data: unexpected EOF",
                ));
            }
            self.frame_data_read += read;
        }

        self.frame_head = [0; 4];
        self.frame_head_read = 0;
        self.frame_data_read = 0;
        Ok(self.frame_data.take().expect("complete frame data exists"))
    }
}

#[derive(Debug)]
pub struct BatchError<T> {
    pub partial: T,
    pub errors: Vec<ClientError>,
}

impl<T: Default> BatchError<T> {
    fn fatal(error: ClientError) -> Self {
        Self {
            partial: T::default(),
            errors: vec![error],
        }
    }
}

impl<T> fmt::Display for BatchError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.errors.iter().enumerate() {
            if index != 0 {
                formatter.write_str("\n")?;
            }
            write!(formatter, "{error}")?;
        }
        Ok(())
    }
}

impl<T: fmt::Debug> std::error::Error for BatchError<T> {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipState {
    Alive,
    Suspect,
    Down,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct NtpTimestamp {
    pub unix_seconds: u32,
    pub nanoseconds: u32,
}

impl NtpTimestamp {
    pub fn from_ntp64(value: u64) -> Self {
        let fraction = value as u32;
        Self {
            unix_seconds: (value >> 32) as u32,
            nanoseconds: ((u64::from(fraction) * 1_000_000_000) >> 32) as u32,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClusterMembershipState {
    pub id: String,
    pub address: SocketAddr,
    pub state: MembershipState,
    pub timestamp: NtpTimestamp,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MemberRttStats {
    pub address: SocketAddr,
    pub median: Duration,
    pub standard_deviation: Duration,
}

fn encode_frame(data: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(4 + data.len());
    encoded.extend_from_slice(&(data.len() as u32).to_be_bytes());
    encoded.extend_from_slice(data);
    encoded
}

fn normalize_numbers_to_f64(value: &mut Value) {
    match value {
        Value::Number(number) => {
            if let Some(float) = number.as_f64().and_then(Number::from_f64) {
                *number = float;
            }
        }
        Value::Array(values) => values.iter_mut().for_each(normalize_numbers_to_f64),
        Value::Object(values) => values.values_mut().for_each(normalize_numbers_to_f64),
        _ => {}
    }
}

fn parse_cluster_membership_state(
    object: &Map<String, Value>,
) -> Result<ClusterMembershipState, ClientError> {
    let id_object = object.get("id").and_then(Value::as_object).ok_or_else(|| {
        ClientError::new(ClientErrorKind::Protocol, "missing or invalid 'id' field")
    })?;
    let id = id_object
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ClientError::new(ClientErrorKind::Protocol, "missing or invalid 'id' field")
        })?
        .to_owned();
    let address = id_object
        .get("addr")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ClientError::new(ClientErrorKind::Protocol, "missing or invalid 'addr' field")
        })?
        .parse()
        .map_err(|error| {
            ClientError::with_source(ClientErrorKind::Protocol, "parse 'addr' field", error)
        })?;
    let state = match object.get("state").and_then(Value::as_str) {
        Some("Alive") => MembershipState::Alive,
        Some("Suspect") => MembershipState::Suspect,
        Some("Down") => MembershipState::Down,
        Some(value) => {
            return Err(ClientError::new(
                ClientErrorKind::Protocol,
                format!("invalid 'state' field: {value}"),
            ));
        }
        None => {
            return Err(ClientError::new(
                ClientErrorKind::Protocol,
                "missing or invalid 'state' field",
            ));
        }
    };
    let timestamp = id_object.get("ts").and_then(Value::as_f64).ok_or_else(|| {
        ClientError::new(ClientErrorKind::Protocol, "missing or invalid 'ts' field")
    })? as u64;

    Ok(ClusterMembershipState {
        id,
        address,
        state,
        timestamp: NtpTimestamp::from_ntp64(timestamp),
    })
}

fn parse_cluster_member_rtt(
    object: &Map<String, Value>,
) -> Result<(SocketAddr, Vec<f64>), ClientError> {
    let state = object
        .get("state")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ClientError::new(
                ClientErrorKind::Protocol,
                "missing or invalid 'state' field",
            )
        })?;
    let address = state
        .get("addr")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ClientError::new(
                ClientErrorKind::Protocol,
                "missing or invalid 'addr' field in 'state'",
            )
        })?
        .parse()
        .map_err(|error| {
            ClientError::with_source(ClientErrorKind::Protocol, "parse 'addr' field", error)
        })?;
    let Some(rtts) = object.get("rtts") else {
        return Ok((address, Vec::new()));
    };
    if rtts.is_null() {
        return Ok((address, Vec::new()));
    }
    let rtts = rtts
        .as_array()
        .ok_or_else(|| ClientError::new(ClientErrorKind::Protocol, "invalid 'rtts' field type"))?;
    let samples = rtts
        .iter()
        .map(|value| {
            value.as_f64().ok_or_else(|| {
                ClientError::new(ClientErrorKind::Protocol, "invalid rtt value type")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((address, samples))
}

fn compute_rtt_stats_ms(samples: &[f64]) -> (f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = if sorted.len().is_multiple_of(2) {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
    } else {
        sorted[sorted.len() / 2]
    };
    let average = sorted.iter().sum::<f64>() / sorted.len() as f64;
    let variance = sorted
        .iter()
        .map(|sample| (sample - average).powi(2))
        .sum::<f64>()
        / sorted.len() as f64;
    (median, variance.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::future::Future;
    use tokio::net::UnixListener;

    fn run<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn computes_population_rtt_statistics() {
        let cases = [
            (vec![42.0], 42.0, 0.0),
            (vec![10.0, 20.0], 15.0, 5.0),
            (vec![3.0, 1.0, 2.0], 2.0, (2.0_f64 / 3.0).sqrt()),
            (vec![1.0, 2.0, 3.0, 4.0], 2.5, 1.25_f64.sqrt()),
            (
                vec![100.0, 1.0, 50.0],
                50.0,
                ((149.0_f64.powi(2) + 148.0_f64.powi(2) + 1.0) / 9.0 / 3.0).sqrt(),
            ),
        ];
        for (samples, expected_median, expected_deviation) in cases {
            let (median, deviation) = compute_rtt_stats_ms(&samples);
            assert!((median - expected_median).abs() < 1e-9);
            assert!((deviation - expected_deviation).abs() < 1e-9);
        }
    }

    #[test]
    fn parses_member_rtt_variants() {
        let valid = json!({"state":{"addr":"[fdcc:b618:5034:7afa:172a:1452:f2de:3c99]:51001"},"rtts":[10,20,30]});
        let object = valid.as_object().unwrap();
        let (address, samples) = parse_cluster_member_rtt(object).unwrap();
        assert_eq!(
            address.to_string(),
            "[fdcc:b618:5034:7afa:172a:1452:f2de:3c99]:51001"
        );
        assert_eq!(samples, [10.0, 20.0, 30.0]);

        for value in [
            json!({"state":{"addr":"127.0.0.1:51001"}}),
            json!({"state":{"addr":"127.0.0.1:51001"},"rtts":null}),
            json!({"state":{"addr":"127.0.0.1:51001"},"rtts":[]}),
        ] {
            assert!(
                parse_cluster_member_rtt(value.as_object().unwrap())
                    .unwrap()
                    .1
                    .is_empty()
            );
        }
        assert!(parse_cluster_member_rtt(json!({"rtts":[1]}).as_object().unwrap()).is_err());
        assert!(
            parse_cluster_member_rtt(json!({"state":{},"rtts":[1]}).as_object().unwrap()).is_err()
        );
        assert!(
            parse_cluster_member_rtt(
                json!({"state":{"addr":"bad"},"rtts":[1]})
                    .as_object()
                    .unwrap()
            )
            .is_err()
        );
        assert!(
            parse_cluster_member_rtt(
                json!({"state":{"addr":"127.0.0.1:1"},"rtts":"bad"})
                    .as_object()
                    .unwrap()
            )
            .is_err()
        );
        assert!(
            parse_cluster_member_rtt(
                json!({"state":{"addr":"127.0.0.1:1"},"rtts":[1,"bad"]})
                    .as_object()
                    .unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn ntp_fraction_uses_oracle_shift() {
        let timestamp = NtpTimestamp::from_ntp64((42_u64 << 32) | (1_u64 << 31));
        assert_eq!(timestamp.unix_seconds, 42);
        assert_eq!(timestamp.nanoseconds, 500_000_000);
    }

    #[test]
    fn admin_framing_and_latest_membership_selection() {
        run(async {
            let socket_path = std::env::temp_dir().join(format!(
                "ployz-corrosion-admin-{}-{}.sock",
                std::process::id(),
                fastrand::u64(..)
            ));
            let listener = UnixListener::bind(&socket_path).unwrap();
            let server = tokio::spawn(async move {
                let (mut connection, _) = listener.accept().await.unwrap();
                let mut head = [0_u8; 4];
                connection.read_exact(&mut head).await.unwrap();
                let mut command = vec![0; u32::from_be_bytes(head) as usize];
                connection.read_exact(&mut command).await.unwrap();
                assert_eq!(command, MEMBERSHIP_STATES_COMMAND);

                for response in [
                    br#"{"Json":{"id":{"id":"member","addr":"127.0.0.1:51001","ts":4294967296},"state":"Alive"}}"#.as_slice(),
                    br#"{"Json":{"id":{"id":"member","addr":"127.0.0.1:51001","ts":8589934592},"state":"Suspect"}}"#.as_slice(),
                    br#""Success""#.as_slice(),
                ] {
                    connection.write_all(&encode_frame(response)).await.unwrap();
                }
            });

            let states = AdminClient::new(&socket_path)
                .cluster_membership_states(true)
                .await
                .unwrap();
            assert_eq!(states.len(), 1);
            assert_eq!(states[0].state, MembershipState::Suspect);
            assert_eq!(states[0].timestamp.unix_seconds, 2);
            server.await.unwrap();
            std::fs::remove_file(socket_path).unwrap();
        });
    }

    #[test]
    fn cancelling_next_preserves_partial_frame_state() {
        run(async {
            let socket_path = std::env::temp_dir().join(format!(
                "ployz-corrosion-cancel-{}-{}.sock",
                std::process::id(),
                fastrand::u64(..)
            ));
            let listener = UnixListener::bind(&socket_path).unwrap();
            let server = tokio::spawn(async move {
                let (mut connection, _) = listener.accept().await.unwrap();
                let mut head = [0_u8; 4];
                connection.read_exact(&mut head).await.unwrap();
                let mut command = vec![0; u32::from_be_bytes(head) as usize];
                connection.read_exact(&mut command).await.unwrap();

                let first = encode_frame(br#"{"Json":{"value":1}}"#);
                connection.write_all(&first[..2]).await.unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
                connection.write_all(&first[2..]).await.unwrap();

                let second = encode_frame(br#"{"Json":{"value":2}}"#);
                connection.write_all(&second[..7]).await.unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
                connection.write_all(&second[7..]).await.unwrap();
                connection
                    .write_all(&encode_frame(br#""Success""#))
                    .await
                    .unwrap();
            });

            let mut responses = AdminClient::new(&socket_path)
                .send_command(b"test")
                .await
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(5), responses.next())
                    .await
                    .is_err()
            );
            assert_eq!(
                responses.next().await.unwrap().unwrap()["value"],
                Value::from(1.0)
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(5), responses.next())
                    .await
                    .is_err()
            );
            assert_eq!(
                responses.next().await.unwrap().unwrap()["value"],
                Value::from(2.0)
            );
            assert!(responses.next().await.is_none());
            server.await.unwrap();
            std::fs::remove_file(socket_path).unwrap();
        });
    }
}
