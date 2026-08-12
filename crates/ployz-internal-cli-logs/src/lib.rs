//! CLI log selection, parsing, and terminal presentation.

mod formatter;
mod options;
mod service_args;

pub use formatter::{FormattedEntry, Formatter, FormatterError, OutputStream};
pub use options::{Options, TailError, parse_tail};
pub use service_args::{ParseServiceArgError, ServiceArg, parse_service_args};
