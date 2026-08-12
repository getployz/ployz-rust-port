//! Async client for Corrosion's query, subscription, and Unix admin APIs.

mod admin;
mod backoff;
mod json;
mod query;
mod transport;

pub use admin::{
    AdminClient, AdminResponses, BatchError, ClusterMembershipState, MemberRttStats,
    MembershipState, NtpTimestamp,
};
pub use json::{GoBytes, JsonColumn, RawSqlValue, SqlValue};
pub use query::{
    ApiClient, ChangeEvent, ChangeStream, ChangeType, EndOfQuery, ExecError, ExecResponse,
    ExecResult, Row, Rows, Statement, Subscription,
};
pub use transport::{ClientError, ClientErrorKind};
