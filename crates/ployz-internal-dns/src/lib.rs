//! Client and wire types for the Uncloud public DNS service.

mod api;
mod client;
mod error;
mod transport;
mod url;

pub use api::{DomainResponse, RecordRequest, RecordResponse, RecordType};
pub use client::Client;
pub use error::{CreateRecordsError, Error};
pub use transport::{Header, Request, Response, Transport, TransportError};
