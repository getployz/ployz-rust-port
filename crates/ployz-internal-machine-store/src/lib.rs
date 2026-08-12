//! Cluster state stored in Corrosion's distributed SQLite database.

use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use base64::{Engine as _, alphabet, engine};
use ployz_internal_corrosion::{ApiClient, GoBytes, SqlValue};
use ployz_internal_machine_api_pb::{Ip, IpPort, IpPrefix, MachineInfo, NetworkConfig};
use ployz_pkg_api::ServiceContainer;
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::value::RawValue;
use tokio::sync::{mpsc, oneshot};

pub const SCHEMA: &str = include_str!("schema.sql");
pub const SYNC_STATUS_SYNCED: &str = "synced";
pub const SYNC_STATUS_OUTDATED: &str = "outdated";

type BoxError = Box<dyn StdError + Send + Sync>;
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type ChangeFuture<'a> = Pin<Box<dyn Future<Output = Option<Result<(), BoxError>>> + 'a>>;
type SubscriptionSnapshot = (Vec<DbRow>, Box<dyn ChangeSource>);

#[derive(Debug)]
pub enum Error {
    KeyNotFound,
    MachineNotFound(String),
    InvalidInput(&'static str),
    InvalidData(String),
    Operation {
        context: &'static str,
        source: BoxError,
    },
}

impl Error {
    fn operation(context: &'static str, source: BoxError) -> Self {
        Self::Operation { context, source }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyNotFound => formatter.write_str("key not found"),
            Self::MachineNotFound(id) => write!(formatter, "machine not found: {id}"),
            Self::InvalidInput(message) => formatter.write_str(message),
            Self::InvalidData(message) => formatter.write_str(message),
            Self::Operation { context, source } => write!(formatter, "{context}: {source}"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Operation { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

pub trait FromClusterValue: Sized {
    fn from_cluster_json(value: &str) -> Result<Self, Error>;
}

pub trait ToClusterValue {
    fn to_cluster_value(&self) -> SqlValue;
}

impl FromClusterValue for String {
    fn from_cluster_json(value: &str) -> Result<Self, Error> {
        decode_nullable_column(value, "cluster TEXT value")
    }
}

impl FromClusterValue for Vec<u8> {
    fn from_cluster_json(value: &str) -> Result<Self, Error> {
        decode_nullable_column::<GoBytes>(value, "cluster BLOB value")
            .map(|bytes| bytes.0.unwrap_or_default())
    }
}

impl ToClusterValue for str {
    fn to_cluster_value(&self) -> SqlValue {
        SqlValue::String(self.to_owned())
    }
}

impl ToClusterValue for String {
    fn to_cluster_value(&self) -> SqlValue {
        SqlValue::String(self.clone())
    }
}

impl ToClusterValue for [u8] {
    fn to_cluster_value(&self) -> SqlValue {
        SqlValue::Bytes(self.to_vec())
    }
}

impl ToClusterValue for Vec<u8> {
    fn to_cluster_value(&self) -> SqlValue {
        SqlValue::Bytes(self.clone())
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StoreTimestamp(String);

impl StoreTimestamp {
    pub fn parse(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        let shaped = value.len() == 19
            && value.bytes().enumerate().all(|(index, byte)| match index {
                4 | 7 => byte == b'-',
                10 => byte == b' ',
                13 | 16 => byte == b':',
                _ => byte.is_ascii_digit(),
            });
        let valid = shaped && valid_store_timestamp(&value);
        if valid {
            Ok(Self(value))
        } else {
            Err(Error::InvalidData(format!(
                "parse updated_at: invalid timestamp {value:?}"
            )))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StoreTimestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MissingChange {
    pub actor_id: String,
    pub start_version: i64,
    pub end_version: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerRecord {
    pub container: ServiceContainer,
    pub machine_id: String,
    pub sync_status: String,
    pub updated_at: StoreTimestamp,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ListOptions {
    pub machine_ids: Vec<String>,
    pub service_id_or_name: ServiceIdOrNameOptions,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ServiceIdOrNameOptions {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeleteOptions {
    pub ids: Vec<String>,
    pub machine_ids: Vec<String>,
}

#[derive(Clone)]
pub struct Store {
    backend: Arc<dyn Backend>,
}

impl fmt::Debug for Store {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Store").finish_non_exhaustive()
    }
}

impl Store {
    #[must_use]
    pub fn new(client: ApiClient) -> Self {
        Self {
            backend: Arc::new(CorrosionBackend { client }),
        }
    }

    #[cfg(test)]
    fn with_backend(backend: Arc<dyn Backend>) -> Self {
        Self { backend }
    }

    pub async fn get<T: FromClusterValue>(&self, key: &str) -> Result<T, Error> {
        let result = self
            .backend
            .query(
                "SELECT value FROM cluster WHERE key = ?",
                Some(vec![key.into()]),
                &[ColumnKind::Raw],
            )
            .await
            .map_err(|source| Error::operation("query cluster value", source))?;
        let Some(row) = result.rows.into_iter().next() else {
            if let Some(source) = result.terminal_error {
                return Err(Error::operation("query cluster value", source));
            }
            return Err(Error::KeyNotFound);
        };
        let value = match row.into_iter().next() {
            Some(DbValue::Raw(value)) => value,
            _ => {
                return Err(Error::InvalidData(
                    "cluster value has unsupported SQL type".into(),
                ));
            }
        };
        T::from_cluster_json(&value)
    }

    pub async fn put<T: ToClusterValue + ?Sized>(&self, key: &str, value: &T) -> Result<(), Error> {
        self.exec(
            "put cluster value",
            "INSERT OR REPLACE INTO cluster (key, value, updated_at) VALUES (?, ?, datetime('now'))",
            Some(vec![key.into(), value.to_cluster_value()]),
        )
        .await?;
        Ok(())
    }

    pub async fn delete(&self, key: &str) -> Result<(), Error> {
        self.exec(
            "delete cluster value",
            "DELETE FROM cluster WHERE key = ?",
            Some(vec![key.into()]),
        )
        .await?;
        Ok(())
    }

    pub async fn version(&self) -> Result<std::collections::BTreeMap<String, i64>, Error> {
        let rows = self
            .query(
                "query crsql_db_versions",
                "SELECT site_id, db_version FROM crsql_db_versions",
                None,
                &[ColumnKind::Blob, ColumnKind::Integer],
            )
            .await?;
        let mut versions = std::collections::BTreeMap::new();
        for row in rows {
            let [DbValue::Bytes(site_id), DbValue::Integer(version)] = row.as_slice() else {
                return Err(Error::InvalidData(
                    "scan actor version: invalid columns".into(),
                ));
            };
            versions.insert(format_uuid(site_id)?, *version);
        }
        Ok(versions)
    }

    pub async fn known_missing_changes(&self) -> Result<Vec<MissingChange>, Error> {
        let rows = self
            .query(
                "query missing changes",
                "SELECT actor_id, start, end FROM __corro_bookkeeping_gaps",
                None,
                &[ColumnKind::Blob, ColumnKind::Integer, ColumnKind::Integer],
            )
            .await?;
        rows.into_iter()
            .map(|row| match row.as_slice() {
                [
                    DbValue::Bytes(actor),
                    DbValue::Integer(start),
                    DbValue::Integer(end),
                ] => Ok(MissingChange {
                    actor_id: encode_hex(actor),
                    start_version: *start,
                    end_version: *end,
                }),
                _ => Err(Error::InvalidData(
                    "scan missing change: invalid columns".into(),
                )),
            })
            .collect()
    }

    pub async fn create_machine(&self, machine: &MachineInfo) -> Result<(), Error> {
        let json = encode_machine(machine)?;
        self.exec(
            "insert query",
            "INSERT INTO machines (id, info, created_at, updated_at) VALUES (?, ?, datetime('now'), datetime('now'))",
            Some(vec![machine.id.as_str().into(), json.into()]),
        )
        .await?;
        Ok(())
    }

    pub async fn get_machine(&self, machine_id: &str) -> Result<MachineInfo, Error> {
        if machine_id.is_empty() {
            return Err(Error::InvalidInput("machine ID cannot be empty"));
        }
        let result = self
            .query_result(
                "query machine",
                "SELECT info FROM machines WHERE id = ?",
                Some(vec![machine_id.into()]),
                &[ColumnKind::Text],
            )
            .await?;
        let Some(row) = result.rows.into_iter().next() else {
            if let Some(source) = result.terminal_error {
                return Err(Error::operation("query machine", source));
            }
            return Err(Error::MachineNotFound(machine_id.to_owned()));
        };
        let Some(DbValue::Text(json)) = row.into_iter().next() else {
            return Err(Error::InvalidData(
                "scan machine info: invalid column".into(),
            ));
        };
        if json.is_empty() {
            return Err(Error::InvalidData(format!(
                "machine info is empty for id {machine_id}"
            )));
        }
        let machine = decode_machine(&json).map_err(|error| {
            Error::InvalidData(format!(
                "unmarshal machine info for id {machine_id}: {error}"
            ))
        })?;
        if machine.id != machine_id {
            return Err(Error::InvalidData(format!(
                "machine ID mismatch: expected {machine_id}, got {}",
                machine.id
            )));
        }
        if let Some(network) = &machine.network {
            network.validate().map_err(|error| {
                Error::InvalidData(format!(
                    "invalid network configuration for machine {}: {error}",
                    machine.id
                ))
            })?;
        }
        Ok(machine)
    }

    pub async fn list_machines(&self) -> Result<Vec<MachineInfo>, Error> {
        let rows = self
            .query(
                "list machines",
                "SELECT id, info FROM machines ORDER BY name",
                None,
                &[ColumnKind::Text, ColumnKind::Text],
            )
            .await?;
        decode_machine_rows(rows, true)
    }

    pub async fn update_machine(&self, machine: &MachineInfo) -> Result<(), Error> {
        if machine.id.is_empty() {
            return Err(Error::InvalidInput("machine ID cannot be empty"));
        }
        let json = encode_machine(machine)?;
        let affected = self
            .exec(
                "update machine",
                "UPDATE machines SET info = ?, updated_at = datetime('now') WHERE id = ?",
                Some(vec![json.into(), machine.id.as_str().into()]),
            )
            .await?;
        if affected == 0 {
            return Err(Error::MachineNotFound(machine.id.clone()));
        }
        Ok(())
    }

    pub async fn delete_machine(&self, id: &str) -> Result<(), Error> {
        let affected = self
            .exec(
                "delete machine",
                "DELETE FROM machines WHERE id = ?",
                Some(vec![id.into()]),
            )
            .await?;
        if affected == 0 {
            return Err(Error::MachineNotFound(id.to_owned()));
        }
        Ok(())
    }

    pub async fn subscribe_machines(
        &self,
    ) -> Result<(Vec<MachineInfo>, ChangeNotifications), Error> {
        let (rows, source) = self
            .backend
            .subscribe(
                "SELECT id, info FROM machines ORDER BY name",
                None,
                &[ColumnKind::Text, ColumnKind::Text],
            )
            .await
            .map_err(|source| Error::operation("subscribe to machines", source))?;
        let machines = decode_machine_rows(rows, false)?;
        Ok((machines, ChangeNotifications::new(source)))
    }

    pub async fn create_or_update_container(
        &self,
        mut container: ServiceContainer,
        machine_id: &str,
    ) -> Result<(), Error> {
        normalise_container_for_store(&mut container);
        let json = serde_json::to_vec(&container)
            .map(go_escape_json)
            .map_err(|error| Error::InvalidData(format!("marshal container: {error}")))?;
        let json = String::from_utf8(json).expect("JSON escaping preserves UTF-8");
        self.exec(
            "upsert query",
            r#"
        INSERT INTO containers (id, container, machine_id, sync_status, updated_at)
        VALUES (?, ?, ?, ?, datetime('now'))
        ON CONFLICT (id) DO UPDATE SET container   = excluded.container,
                                       machine_id  = excluded.machine_id,
                                       sync_status = excluded.sync_status,
                                       updated_at  = excluded.updated_at
        WHERE containers.container != excluded.container
          OR containers.machine_id != excluded.machine_id"#,
            Some(vec![
                container.container.id.as_str().into(),
                json.into(),
                machine_id.into(),
                SYNC_STATUS_SYNCED.into(),
            ]),
        )
        .await?;
        Ok(())
    }

    pub async fn list_containers(
        &self,
        options: &ListOptions,
    ) -> Result<Vec<ContainerRecord>, Error> {
        let (query, params) = list_containers_query(options);
        let rows = self
            .query("select query", &query, Some(params), &container_columns())
            .await?;
        decode_container_rows(rows)
    }

    pub async fn delete_containers(&self, options: &DeleteOptions) -> Result<(), Error> {
        let (query, params) = delete_containers_query(options);
        let params = (!params.is_empty()).then_some(params);
        self.exec("delete query", &query, params).await?;
        Ok(())
    }

    pub async fn subscribe_containers(
        &self,
    ) -> Result<(Vec<ContainerRecord>, ChangeNotifications), Error> {
        let (query, params) = list_containers_query(&ListOptions::default());
        let (rows, source) = self
            .backend
            .subscribe(&query, Some(params), &container_columns())
            .await
            .map_err(|source| Error::operation("subscribe to containers", source))?;
        Ok((
            decode_container_rows(rows)?,
            ChangeNotifications::new(source),
        ))
    }

    async fn exec(
        &self,
        context: &'static str,
        query: &str,
        params: Option<Vec<SqlValue>>,
    ) -> Result<u64, Error> {
        self.backend
            .exec(query, params)
            .await
            .map_err(|source| Error::operation(context, source))
    }

    async fn query(
        &self,
        context: &'static str,
        query: &str,
        params: Option<Vec<SqlValue>>,
        columns: &[ColumnKind],
    ) -> Result<Vec<DbRow>, Error> {
        self.query_result(context, query, params, columns)
            .await
            .map(|result| result.rows)
    }

    async fn query_result(
        &self,
        context: &'static str,
        query: &str,
        params: Option<Vec<SqlValue>>,
        columns: &[ColumnKind],
    ) -> Result<QueryRows, Error> {
        self.backend
            .query(query, params, columns)
            .await
            .map_err(|source| Error::operation(context, source))
    }
}

pub struct ChangeNotifications {
    receiver: mpsc::Receiver<Result<(), BoxError>>,
    cancellation: Option<oneshot::Sender<()>>,
    worker: Option<tokio::task::JoinHandle<()>>,
    error: Option<BoxError>,
    closed: bool,
}

impl fmt::Debug for ChangeNotifications {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChangeNotifications")
            .finish_non_exhaustive()
    }
}

impl ChangeNotifications {
    fn new(source: Box<dyn ChangeSource>) -> Self {
        let (sender, receiver) = mpsc::channel(1);
        let (cancellation, cancelled) = oneshot::channel();
        let worker = tokio::task::spawn_blocking(move || {
            run_change_worker(source, sender, cancelled);
        });
        Self {
            receiver,
            cancellation: Some(cancellation),
            worker: Some(worker),
            error: None,
            closed: false,
        }
    }

    pub async fn recv(&mut self) -> Option<()> {
        if self.closed {
            return None;
        }
        match self.receiver.recv().await {
            Some(Ok(())) => Some(()),
            Some(Err(error)) => {
                self.error = Some(error);
                self.closed = true;
                self.finish_worker().await;
                None
            }
            None => {
                self.closed = true;
                self.finish_worker().await;
                None
            }
        }
    }

    #[must_use]
    pub fn error(&self) -> Option<&(dyn StdError + Send + Sync + 'static)> {
        self.error.as_deref()
    }

    async fn finish_worker(&mut self) {
        self.cancellation.take();
        if let Some(worker) = self.worker.take()
            && let Err(error) = worker.await
            && self.error.is_none()
        {
            self.error = Some(Box::new(error));
        }
    }
}

impl Drop for ChangeNotifications {
    fn drop(&mut self) {
        if let Some(cancellation) = self.cancellation.take() {
            let _ = cancellation.send(());
        }
    }
}

fn run_change_worker(
    mut source: Box<dyn ChangeSource>,
    sender: mpsc::Sender<Result<(), BoxError>>,
    mut cancelled: oneshot::Receiver<()>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = sender.blocking_send(Err(Box::new(error)));
            return;
        }
    };
    runtime.block_on(async move {
        loop {
            let event = tokio::select! {
                biased;
                _ = &mut cancelled => break,
                event = source.next() => event,
            };
            let Some(event) = event else {
                break;
            };
            let failed = event.is_err();
            let sent = tokio::select! {
                biased;
                _ = &mut cancelled => false,
                result = sender.send(event) => result.is_ok(),
            };
            if !sent || failed {
                break;
            }
        }
    });
}

#[derive(Clone, Copy, Debug)]
enum ColumnKind {
    Text,
    Blob,
    Integer,
    Raw,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DbValue {
    Text(String),
    Bytes(Vec<u8>),
    Integer(i64),
    Raw(String),
}

type DbRow = Vec<DbValue>;

struct QueryRows {
    rows: Vec<DbRow>,
    terminal_error: Option<BoxError>,
}

trait ChangeSource: Send {
    fn next(&mut self) -> ChangeFuture<'_>;
}

trait Backend: Send + Sync {
    fn exec<'a>(
        &'a self,
        query: &'a str,
        params: Option<Vec<SqlValue>>,
    ) -> BoxFuture<'a, Result<u64, BoxError>>;

    fn query<'a>(
        &'a self,
        query: &'a str,
        params: Option<Vec<SqlValue>>,
        columns: &'a [ColumnKind],
    ) -> BoxFuture<'a, Result<QueryRows, BoxError>>;

    fn subscribe<'a>(
        &'a self,
        query: &'a str,
        params: Option<Vec<SqlValue>>,
        columns: &'a [ColumnKind],
    ) -> BoxFuture<'a, Result<SubscriptionSnapshot, BoxError>>;
}

struct CorrosionBackend {
    client: ApiClient,
}

impl Backend for CorrosionBackend {
    fn exec<'a>(
        &'a self,
        query: &'a str,
        params: Option<Vec<SqlValue>>,
    ) -> BoxFuture<'a, Result<u64, BoxError>> {
        Box::pin(async move {
            self.client
                .exec(query, params)
                .await
                .map(|result| result.rows_affected)
                .map_err(|error| Box::new(error) as BoxError)
        })
    }

    fn query<'a>(
        &'a self,
        query: &'a str,
        params: Option<Vec<SqlValue>>,
        columns: &'a [ColumnKind],
    ) -> BoxFuture<'a, Result<QueryRows, BoxError>> {
        Box::pin(async move {
            let mut rows = self
                .client
                .query(query, params)
                .await
                .map_err(|error| Box::new(error) as BoxError)?;
            let mut decoded = Vec::new();
            let terminal_error = loop {
                match rows.next().await {
                    Ok(Some(row)) => decoded.push(decode_corro_row(&row, columns)?),
                    Ok(None) => break None,
                    Err(error) => break Some(Box::new(error) as BoxError),
                }
            };
            Ok(QueryRows {
                rows: decoded,
                terminal_error,
            })
        })
    }

    fn subscribe<'a>(
        &'a self,
        query: &'a str,
        params: Option<Vec<SqlValue>>,
        columns: &'a [ColumnKind],
    ) -> BoxFuture<'a, Result<(Vec<DbRow>, Box<dyn ChangeSource>), BoxError>> {
        Box::pin(async move {
            let mut subscription = self
                .client
                .subscribe(query, params, false)
                .await
                .map_err(|error| Box::new(error) as BoxError)?;
            let rows = subscription.rows_mut().ok_or_else(|| {
                Box::new(SimpleError("subscription snapshot is missing".into())) as BoxError
            })?;
            let mut decoded = Vec::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(|error| Box::new(error) as BoxError)?
            {
                decoded.push(decode_corro_row(&row, columns)?);
            }
            let changes = subscription
                .into_changes()
                .map_err(|error| Box::new(error) as BoxError)?;
            let changes: Box<dyn ChangeSource> = Box::new(CorrosionChanges(changes));
            Ok((decoded, changes))
        })
    }
}

struct CorrosionChanges(ployz_internal_corrosion::ChangeStream);

impl ChangeSource for CorrosionChanges {
    fn next(&mut self) -> ChangeFuture<'_> {
        Box::pin(async move {
            self.0.next().await.map(|result| {
                result
                    .map(|_| ())
                    .map_err(|error| Box::new(error) as BoxError)
            })
        })
    }
}

#[derive(Debug)]
struct SimpleError(String);

impl fmt::Display for SimpleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl StdError for SimpleError {}

fn decode_nullable_column<T>(value: &str, kind: &str) -> Result<T, Error>
where
    T: serde::de::DeserializeOwned + Default,
{
    if value.trim() == "null" {
        return Ok(T::default());
    }
    serde_json::from_str(value)
        .map_err(|error| Error::InvalidData(format!("decode {kind}: {error}")))
}

fn decode_corro_row(
    row: &ployz_internal_corrosion::Row,
    columns: &[ColumnKind],
) -> Result<DbRow, BoxError> {
    row.expect_columns(columns.len())
        .map_err(|error| Box::new(error) as BoxError)?;
    columns
        .iter()
        .enumerate()
        .map(|(index, kind)| match kind {
            ColumnKind::Text => row
                .get::<String>(index)
                .map(DbValue::Text)
                .map_err(|error| Box::new(error) as BoxError),
            ColumnKind::Blob => row
                .get::<GoBytes>(index)
                .map(|value| DbValue::Bytes(value.0.unwrap_or_default()))
                .map_err(|error| Box::new(error) as BoxError),
            ColumnKind::Integer => row
                .get::<i64>(index)
                .map(DbValue::Integer)
                .map_err(|error| Box::new(error) as BoxError),
            ColumnKind::Raw => {
                let value = row
                    .values
                    .as_ref()
                    .and_then(|values| values.get(index))
                    .ok_or_else(|| {
                        Box::new(SimpleError(format!(
                            "column index {index} is out of bounds"
                        ))) as BoxError
                    })?;
                Ok(DbValue::Raw(value.raw().to_owned()))
            }
        })
        .collect()
}

fn decode_machine_rows(
    rows: Vec<DbRow>,
    validate_network: bool,
) -> Result<Vec<MachineInfo>, Error> {
    let mut machines = Vec::new();
    for row in rows {
        let [DbValue::Text(_id), DbValue::Text(json)] = row.as_slice() else {
            return Err(Error::InvalidData(
                "scan machine info: invalid columns".into(),
            ));
        };
        if json.is_empty() || json == "{}" {
            continue;
        }
        let machine = decode_machine(json)
            .map_err(|error| Error::InvalidData(format!("unmarshal machine info: {error}")))?;
        if validate_network {
            let network = machine
                .network
                .as_ref()
                .expect("machine network is absent in replicated store data");
            if network.validate().is_err() {
                continue;
            }
        }
        machines.push(machine);
    }
    Ok(machines)
}

fn normalise_container_for_store(container: &mut ServiceContainer) {
    if let Some(config) = &mut container.container.config {
        config.env = Default::default();
    }
    container.service_spec.container.env = Default::default();
    if !container.container.mounts.is_empty() {
        container.container.mounts.sort_by(|left, right| {
            left.destination
                .cmp(&right.destination)
                .then_with(|| left.source.cmp(&right.source))
        });
    }
}

fn container_columns() -> [ColumnKind; 5] {
    [
        ColumnKind::Text,
        ColumnKind::Text,
        ColumnKind::Text,
        ColumnKind::Text,
        ColumnKind::Text,
    ]
}

fn decode_container_rows(rows: Vec<DbRow>) -> Result<Vec<ContainerRecord>, Error> {
    let mut containers = Vec::new();
    for row in rows {
        let [
            DbValue::Text(_id),
            DbValue::Text(json),
            DbValue::Text(machine_id),
            DbValue::Text(sync_status),
            DbValue::Text(updated_at),
        ] = row.as_slice()
        else {
            return Err(Error::InvalidData(
                "scan container record: invalid columns".into(),
            ));
        };
        if json.is_empty() || json == "{}" {
            continue;
        }
        let container = ServiceContainer::from_json(json.as_bytes())
            .map_err(|error| Error::InvalidData(format!("unmarshal container: {error}")))?;
        containers.push(ContainerRecord {
            container,
            machine_id: machine_id.clone(),
            sync_status: sync_status.clone(),
            updated_at: StoreTimestamp::parse(updated_at.clone())?,
        });
    }
    Ok(containers)
}

fn list_containers_query(options: &ListOptions) -> (String, Vec<SqlValue>) {
    let mut query = String::from(
        "SELECT c.id, c.container, c.machine_id, c.sync_status, c.updated_at FROM containers c JOIN machines m ON m.id = c.machine_id WHERE c.sync_status = ?",
    );
    let mut params = vec![SYNC_STATUS_SYNCED.into()];
    if !options.machine_ids.is_empty() {
        query.push_str(" AND c.machine_id IN (");
        push_placeholders(&mut query, options.machine_ids.len());
        query.push(')');
        params.extend(options.machine_ids.iter().map(|id| id.as_str().into()));
    }
    let service = &options.service_id_or_name;
    if !service.id.is_empty() || !service.name.is_empty() {
        query.push_str(" AND (");
        if !service.id.is_empty() {
            query.push_str("c.service_id = ?");
            params.push(service.id.as_str().into());
        }
        if !service.id.is_empty() && !service.name.is_empty() {
            query.push_str(" OR ");
        }
        if !service.name.is_empty() {
            query.push_str("c.service_name = ?");
            params.push(service.name.as_str().into());
        }
        query.push(')');
    }
    (query, params)
}

fn delete_containers_query(options: &DeleteOptions) -> (String, Vec<SqlValue>) {
    let mut query = String::from("DELETE FROM containers");
    let mut params = Vec::new();
    let mut has_filter = false;
    if !options.ids.is_empty() {
        query.push_str(" WHERE id IN (");
        push_placeholders(&mut query, options.ids.len());
        query.push(')');
        params.extend(options.ids.iter().map(|id| id.as_str().into()));
        has_filter = true;
    }
    if !options.machine_ids.is_empty() {
        query.push_str(if has_filter {
            " AND machine_id IN ("
        } else {
            " WHERE machine_id IN ("
        });
        push_placeholders(&mut query, options.machine_ids.len());
        query.push(')');
        params.extend(options.machine_ids.iter().map(|id| id.as_str().into()));
    }
    (query, params)
}

fn push_placeholders(query: &mut String, count: usize) {
    for index in 0..count {
        if index > 0 {
            query.push_str(", ");
        }
        query.push('?');
    }
}

fn format_uuid(bytes: &[u8]) -> Result<String, Error> {
    if bytes.len() != 16 {
        return Err(Error::InvalidData(format!(
            "parse site_id as UUID: invalid length {}",
            bytes.len()
        )));
    }
    let hex = encode_hex(bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}

fn go_escape_json(encoded: Vec<u8>) -> Vec<u8> {
    let mut output = Vec::with_capacity(encoded.len());
    let mut index = 0;
    while index < encoded.len() {
        match encoded[index] {
            b'<' => output.extend_from_slice(br"\u003c"),
            b'>' => output.extend_from_slice(br"\u003e"),
            b'&' => output.extend_from_slice(br"\u0026"),
            0xe2 if encoded.get(index..index + 3) == Some(&[0xe2, 0x80, 0xa8]) => {
                output.extend_from_slice(br"\u2028");
                index += 2;
            }
            0xe2 if encoded.get(index..index + 3) == Some(&[0xe2, 0x80, 0xa9]) => {
                output.extend_from_slice(br"\u2029");
                index += 2;
            }
            byte => output.push(byte),
        }
        index += 1;
    }
    output
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MachineJson<'a> {
    #[serde(skip_serializing_if = "str::is_empty")]
    id: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    network: Option<NetworkJson<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_ip: Option<IpJson<'a>>,
    #[serde(skip_serializing_if = "str::is_empty")]
    daemon_version: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    docker_version: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    hostname: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    arch: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    os_pretty_name: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    kernel_version: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NetworkJson<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    subnet: Option<PrefixJson<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    management_ip: Option<IpJson<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    endpoints: Vec<PortJson<'a>>,
    #[serde(skip_serializing_if = "str::is_empty")]
    public_key: String,
}

#[derive(Serialize)]
struct IpJson<'a> {
    #[serde(skip_serializing_if = "str::is_empty")]
    ip: String,
    #[serde(skip)]
    marker: std::marker::PhantomData<&'a ()>,
}

#[derive(Serialize)]
struct PrefixJson<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    ip: Option<IpJson<'a>>,
    #[serde(skip_serializing_if = "is_zero_u32")]
    bits: u32,
}

#[derive(Serialize)]
struct PortJson<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    ip: Option<IpJson<'a>>,
    #[serde(skip_serializing_if = "is_zero_u32")]
    port: u32,
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn ip_json(ip: &Ip) -> IpJson<'_> {
    IpJson {
        ip: base64::engine::general_purpose::STANDARD.encode(&ip.ip),
        marker: std::marker::PhantomData,
    }
}

fn encode_machine(machine: &MachineInfo) -> Result<String, Error> {
    let network = machine.network.as_ref().map(|network| NetworkJson {
        subnet: network.subnet.as_ref().map(|prefix| PrefixJson {
            ip: prefix.ip.as_ref().map(ip_json),
            bits: prefix.bits,
        }),
        management_ip: network.management_ip.as_ref().map(ip_json),
        endpoints: network
            .endpoints
            .iter()
            .map(|port| PortJson {
                ip: port.ip.as_ref().map(ip_json),
                port: port.port,
            })
            .collect(),
        public_key: base64::engine::general_purpose::STANDARD.encode(&network.public_key),
    });
    let wire = MachineJson {
        id: &machine.id,
        name: &machine.name,
        network,
        public_ip: machine.public_ip.as_ref().map(ip_json),
        daemon_version: &machine.daemon_version,
        docker_version: &machine.docker_version,
        hostname: &machine.hostname,
        arch: &machine.arch,
        os_pretty_name: &machine.os_pretty_name,
        kernel_version: &machine.kernel_version,
    };
    let encoded = serde_json::to_string(&wire)
        .map_err(|error| Error::InvalidData(format!("marshal machine info: {error}")))?;
    Ok(encoded)
}

fn decode_machine(json: &str) -> Result<MachineInfo, String> {
    let object = parse_object(json, "machine")?;
    Ok(MachineInfo {
        id: string_field(&object, "id", "id")?,
        name: string_field(&object, "name", "name")?,
        network: message_field(&object, "network", "network", decode_network)?,
        public_ip: message_field(&object, "publicIp", "public_ip", decode_ip)?,
        daemon_version: string_field(&object, "daemonVersion", "daemon_version")?,
        docker_version: string_field(&object, "dockerVersion", "docker_version")?,
        hostname: string_field(&object, "hostname", "hostname")?,
        arch: string_field(&object, "arch", "arch")?,
        os_pretty_name: string_field(&object, "osPrettyName", "os_pretty_name")?,
        kernel_version: string_field(&object, "kernelVersion", "kernel_version")?,
    })
}

fn decode_network(value: &RawValue) -> Result<NetworkConfig, String> {
    let object = parse_object(value.get(), "network")?;
    let endpoints = match protobuf_field(&object, "endpoints", "endpoints")? {
        None => Vec::new(),
        Some(value) => serde_json::from_str::<Option<Vec<Box<RawValue>>>>(value.get())
            .map_err(|error| format!("endpoints: {error}"))?
            .unwrap_or_default()
            .iter()
            .map(|value| decode_port(value))
            .collect::<Result<_, _>>()?,
    };
    Ok(NetworkConfig {
        subnet: message_field(&object, "subnet", "subnet", decode_prefix)?,
        management_ip: message_field(&object, "managementIp", "management_ip", decode_ip)?,
        endpoints,
        public_key: bytes_field(&object, "publicKey", "public_key")?,
    })
}

fn decode_ip(value: &RawValue) -> Result<Ip, String> {
    let object = parse_object(value.get(), "ip")?;
    Ok(Ip {
        ip: bytes_field(&object, "ip", "ip")?,
    })
}

fn decode_prefix(value: &RawValue) -> Result<IpPrefix, String> {
    let object = parse_object(value.get(), "prefix")?;
    Ok(IpPrefix {
        ip: message_field(&object, "ip", "ip", decode_ip)?,
        bits: u32_field(&object, "bits", "bits")?,
    })
}

fn decode_port(value: &RawValue) -> Result<IpPort, String> {
    let object = parse_object(value.get(), "endpoint")?;
    Ok(IpPort {
        ip: message_field(&object, "ip", "ip", decode_ip)?,
        port: u32_field(&object, "port", "port")?,
    })
}

struct ObjectMembers(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for ObjectMembers {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ObjectVisitor;

        impl<'de> de::Visitor<'de> for ObjectVisitor {
            type Value = ObjectMembers;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut members = Vec::new();
                while let Some(member) = access.next_entry()? {
                    members.push(member);
                }
                Ok(ObjectMembers(members))
            }
        }

        deserializer.deserialize_map(ObjectVisitor)
    }
}

fn parse_object(value: &str, context: &str) -> Result<ObjectMembers, String> {
    serde_json::from_str(value).map_err(|error| format!("{context}: {error}"))
}

fn protobuf_field<'a>(
    object: &'a ObjectMembers,
    json_name: &str,
    proto_name: &str,
) -> Result<Option<&'a RawValue>, String> {
    let mut found = None;
    for (name, value) in &object.0 {
        if name == json_name || name == proto_name {
            if found.is_some() {
                return Err(format!("duplicate field {json_name}"));
            }
            found = Some(value.as_ref());
        }
    }
    Ok(found)
}

fn string_field(
    object: &ObjectMembers,
    json_name: &str,
    proto_name: &str,
) -> Result<String, String> {
    match protobuf_field(object, json_name, proto_name)? {
        None => Ok(String::new()),
        Some(value) => serde_json::from_str::<Option<String>>(value.get())
            .map_err(|error| format!("{json_name}: {error}"))
            .map(Option::unwrap_or_default),
    }
}

fn bytes_field(
    object: &ObjectMembers,
    json_name: &str,
    proto_name: &str,
) -> Result<Vec<u8>, String> {
    let encoded = string_field(object, json_name, proto_name)?;
    if encoded.is_empty() {
        return Ok(Vec::new());
    }
    let filtered: String = encoded
        .chars()
        .filter(|character| !matches!(character, '\r' | '\n'))
        .collect();
    let config = engine::GeneralPurposeConfig::new()
        .with_decode_padding_mode(engine::DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true);
    [alphabet::STANDARD, alphabet::URL_SAFE]
        .into_iter()
        .find_map(|alphabet| {
            engine::GeneralPurpose::new(&alphabet, config)
                .decode(&filtered)
                .ok()
        })
        .ok_or_else(|| format!("{json_name}: invalid base64"))
}

fn valid_store_timestamp(value: &str) -> bool {
    let number = |range: std::ops::Range<usize>| value[range].parse::<u32>().ok();
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        number(0..4),
        number(5..7),
        number(8..10),
        number(11..13),
        number(14..16),
        number(17..19),
    ) else {
        return false;
    };
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day) && hour < 24 && minute < 60 && second < 60
}

fn u32_field(object: &ObjectMembers, json_name: &str, proto_name: &str) -> Result<u32, String> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Numeric(u32),
        String(String),
    }
    match protobuf_field(object, json_name, proto_name)? {
        None => Ok(0),
        Some(value) if value.get().trim() == "null" => Ok(0),
        Some(value) => match serde_json::from_str::<Number>(value.get())
            .map_err(|error| format!("{json_name}: {error}"))?
        {
            Number::Numeric(value) => Ok(value),
            Number::String(value) => value
                .parse()
                .map_err(|error| format!("{json_name}: {error}")),
        },
    }
}

fn message_field<T>(
    object: &ObjectMembers,
    json_name: &str,
    proto_name: &str,
    decode: fn(&RawValue) -> Result<T, String>,
) -> Result<Option<T>, String> {
    match protobuf_field(object, json_name, proto_name)? {
        None => Ok(None),
        Some(value) if value.get().trim() == "null" => Ok(None),
        Some(value) => decode(value).map(Some),
    }
}

#[cfg(test)]
mod tests;
