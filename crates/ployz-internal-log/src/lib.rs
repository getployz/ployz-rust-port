//! Structured application logging with the frozen Uncloud text format.
//!
//! Applications own installation of the process-global tracing subscriber. This
//! crate supplies a reloadable subscriber so the application can install it once
//! and subsequently apply the CLI `DEBUG` policy or the daemon's unconditional
//! debug policy.
//!
//! Fields on entered spans are inherited in root-to-leaf order. A span field
//! named `ployz.group` marks a named group; the marker itself is omitted and the
//! span's fields plus descendant fields receive its dotted prefix:
//!
//! ```
//! let request = tracing::info_span!("request", ployz.group = "request", id = 7_u64);
//! let _entered = request.enter();
//! tracing::info!(kind = "A", "received");
//! // INFO  received request.id=7 request.kind=A
//! ```

use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, Layer, Layered};
use tracing_subscriber::registry::{LookupSpan, Registry};
use tracing_subscriber::{Registry as RegistrySubscriber, reload};

const GROUP_FIELD: &str = "ployz.group";

/// One structured field in a low-level log record.
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    key: String,
    value: AttributeValue,
}

impl Attribute {
    /// Creates a string-valued attribute.
    pub fn string(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: AttributeValue::String(value.into()),
        }
    }

    /// Creates a boolean-valued attribute.
    pub fn boolean(key: impl Into<String>, value: bool) -> Self {
        Self {
            key: key.into(),
            value: AttributeValue::Boolean(value),
        }
    }

    /// Creates a signed-integer-valued attribute.
    pub fn signed(key: impl Into<String>, value: i64) -> Self {
        Self {
            key: key.into(),
            value: AttributeValue::Signed(value),
        }
    }

    /// Creates an unsigned-integer-valued attribute.
    pub fn unsigned(key: impl Into<String>, value: u64) -> Self {
        Self {
            key: key.into(),
            value: AttributeValue::Unsigned(value),
        }
    }

    /// Creates a floating-point-valued attribute.
    pub fn float(key: impl Into<String>, value: f64) -> Self {
        Self {
            key: key.into(),
            value: AttributeValue::Float(value),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum AttributeValue {
    String(String),
    Boolean(bool),
    Signed(i64),
    Unsigned(u64),
    Float(f64),
}

/// A directly callable, fallible handler for the application text format.
///
/// Clones and derived [`TextLayer`] values share one writer lock. Each accepted
/// record performs the prefix write before the structured suffix write, so the
/// original error from either boundary is returned to direct callers.
pub struct TextHandler<W> {
    writer: Arc<Mutex<W>>,
    minimum_level: LevelFilter,
}

impl<W> Clone for TextHandler<W> {
    fn clone(&self) -> Self {
        Self {
            writer: Arc::clone(&self.writer),
            minimum_level: self.minimum_level,
        }
    }
}

impl<W: Write> TextHandler<W> {
    /// Creates a handler with the oracle's default INFO threshold.
    pub fn with_default_level(writer: W) -> Self {
        Self::new(writer, LevelFilter::INFO)
    }

    /// Creates a handler. `LevelFilter::INFO` matches the oracle's default.
    pub fn new(writer: W, minimum_level: LevelFilter) -> Self {
        Self {
            writer: Arc::new(Mutex::new(writer)),
            minimum_level,
        }
    }

    /// Writes a record, or returns the originating writer error.
    pub fn handle(
        &self,
        level: &tracing::Level,
        message: &str,
        attributes: &[Attribute],
    ) -> io::Result<()> {
        if !level_is_enabled(level, self.minimum_level) {
            return Ok(());
        }

        let prefix = format!("{level:<5} {message} ");
        let suffix = format_attributes(attributes);
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        writer.write_all(prefix.as_bytes())?;
        writer.write_all(suffix.as_bytes())
    }
}

/// A tracing layer that renders events through a shared [`TextHandler`].
pub struct TextLayer<W> {
    handler: TextHandler<W>,
}

impl<W> Clone for TextLayer<W> {
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.clone(),
        }
    }
}

impl<W: Write> TextLayer<W> {
    /// Creates a layer with the oracle's default INFO threshold.
    pub fn with_default_level(writer: W) -> Self {
        Self::new(writer, LevelFilter::INFO)
    }

    /// Creates a formatting layer and its shared writer.
    pub fn new(writer: W, minimum_level: LevelFilter) -> Self {
        Self {
            handler: TextHandler::new(writer, minimum_level),
        }
    }

    /// Creates a layer from a low-level handler.
    pub fn from_handler(handler: TextHandler<W>) -> Self {
        Self { handler }
    }

    /// Returns the layer's directly callable handler.
    pub fn handler(&self) -> &TextHandler<W> {
        &self.handler
    }
}

#[derive(Clone, Debug, Default)]
struct SpanFields {
    group: Option<String>,
    attributes: Vec<Attribute>,
}

#[derive(Default)]
struct FieldVisitor {
    message: Option<String>,
    group: Option<String>,
    attributes: Vec<Attribute>,
}

impl FieldVisitor {
    fn key(field: &Field) -> &str {
        field.name()
    }

    fn record_string(&mut self, field: &Field, value: String) {
        match field.name() {
            "message" => self.message = Some(value),
            GROUP_FIELD => self.group = Some(value),
            _ => self
                .attributes
                .push(Attribute::string(Self::key(field), value)),
        }
    }
}

impl Visit for FieldVisitor {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.attributes
            .push(Attribute::boolean(Self::key(field), value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.attributes
            .push(Attribute::signed(Self::key(field), value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.attributes
            .push(Attribute::unsigned(Self::key(field), value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.attributes
            .push(Attribute::float(Self::key(field), value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_string(field, value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record_string(field, format!("{value:?}"));
    }
}

impl<S, W> Layer<S> for TextLayer<W>
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    W: Write + Send + 'static,
{
    fn register_callsite(
        &self,
        metadata: &'static Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        if metadata.is_event() && !level_is_enabled(metadata.level(), self.handler.minimum_level) {
            tracing::subscriber::Interest::never()
        } else {
            tracing::subscriber::Interest::always()
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>, _context: Context<'_, S>) -> bool {
        !metadata.is_event() || level_is_enabled(metadata.level(), self.handler.minimum_level)
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        // Context spans must remain enabled even when their own level is below
        // the event threshold, so filtering happens at the event boundary.
        Some(LevelFilter::TRACE)
    }

    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, context: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        attributes.record(&mut visitor);
        if let Some(span) = context.span(id) {
            span.extensions_mut().insert(SpanFields {
                group: visitor.group,
                attributes: visitor.attributes,
            });
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, context: Context<'_, S>) {
        let Some(span) = context.span(id) else {
            return;
        };
        let mut visitor = FieldVisitor::default();
        values.record(&mut visitor);
        let mut extensions = span.extensions_mut();
        if let Some(fields) = extensions.get_mut::<SpanFields>() {
            if visitor.group.is_some() {
                fields.group = visitor.group;
            }
            fields.attributes.extend(visitor.attributes);
        }
    }

    fn on_event(&self, event: &Event<'_>, context: Context<'_, S>) {
        let mut attributes = Vec::new();
        let mut groups = Vec::new();

        if let Some(scope) = context.event_scope(event) {
            for span in scope.from_root() {
                let extensions = span.extensions();
                if let Some(fields) = extensions.get::<SpanFields>() {
                    if let Some(group) = &fields.group {
                        groups.push(group.clone());
                    }
                    attributes.extend(fields.attributes.iter().cloned().map(|mut attribute| {
                        if !groups.is_empty() {
                            attribute.key = format!("{}.{}", groups.join("."), attribute.key);
                        }
                        attribute
                    }));
                }
            }
        }

        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        attributes.extend(visitor.attributes.into_iter().map(|mut attribute| {
            if !groups.is_empty() {
                attribute.key = format!("{}.{}", groups.join("."), attribute.key);
            }
            attribute
        }));

        // Tracing layers intentionally cannot return sink failures. Discarding
        // this result matches Go's global slog event path; direct callers use
        // TextHandler::handle when they need the I/O error.
        let _ = self.handler.handle(
            event.metadata().level(),
            visitor.message.as_deref().unwrap_or_default(),
            &attributes,
        );
    }
}

/// Reloadable text layer type.
pub type ReloadLayer<W> = reload::Layer<TextLayer<W>, Registry>;
/// Concrete reloadable subscriber type.
pub type ReloadableSubscriber<W> = Layered<ReloadLayer<W>, Registry>;
/// Handle used for same-writer-type layer replacement.
pub type ReloadHandle<W> = reload::Handle<TextLayer<W>, Registry>;

/// Builds a reloadable subscriber without installing it globally.
pub fn reloadable_subscriber<W>(layer: TextLayer<W>) -> (ReloadableSubscriber<W>, ReloadHandle<W>)
where
    W: Write + Send + 'static,
{
    let (layer, handle) = reload::Layer::new(layer);
    (RegistrySubscriber::default().with(layer), handle)
}

/// Reloadable layer type used by the process subscriber.
pub type ProcessReloadLayer = ReloadLayer<io::Stderr>;
/// Concrete subscriber type applications install as the global default.
pub type ProcessSubscriber = Layered<ProcessReloadLayer, Registry>;
/// Handle retained by the application for same-type logger replacement.
pub type ProcessReloadHandle = reload::Handle<TextLayer<io::Stderr>, Registry>;

/// Builds a stderr subscriber and reload handle without installing it globally.
///
/// The executable must pass the returned subscriber to
/// `tracing::subscriber::set_global_default` and handle an unexpected prior
/// global subscriber as an initialization error.
pub fn process_subscriber(minimum_level: LevelFilter) -> (ProcessSubscriber, ProcessReloadHandle) {
    reloadable_subscriber(TextLayer::new(io::stderr(), minimum_level))
}

/// Applies the exact `DEBUG` environment policy and attempts the initialization event.
///
/// Returns `Ok(true)` when a truthy value installed the DEBUG stderr layer and
/// `Ok(false)` when the current configuration was deliberately left untouched.
pub fn init_from_env(handle: &ProcessReloadHandle) -> Result<bool, reload::Error> {
    let result = configure_from_debug_value(handle, env::var_os("DEBUG").as_deref(), || {
        TextLayer::new(io::stderr(), LevelFilter::DEBUG)
    });
    tracing::debug!("logger initialized");
    result
}

/// Unconditionally replaces the application layer with DEBUG stderr logging.
pub fn configure_daemon(handle: &ProcessReloadHandle) -> Result<(), reload::Error> {
    handle.reload(TextLayer::new(io::stderr(), LevelFilter::DEBUG))
}

fn configure_from_debug_value<W, F>(
    handle: &ReloadHandle<W>,
    value: Option<&OsStr>,
    debug_layer: F,
) -> Result<bool, reload::Error>
where
    W: Write + Send + 'static,
    F: FnOnce() -> TextLayer<W>,
{
    let enabled = debug_value_is_truthy(value);
    if enabled {
        handle.reload(debug_layer())?;
    }
    Ok(enabled)
}

fn debug_value_is_truthy(value: Option<&OsStr>) -> bool {
    value.and_then(OsStr::to_str).is_some_and(|value| {
        value.eq_ignore_ascii_case("1")
            || value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("yes")
    })
}

fn level_is_enabled(level: &tracing::Level, minimum: LevelFilter) -> bool {
    match minimum {
        LevelFilter::OFF => false,
        LevelFilter::ERROR => level == &tracing::Level::ERROR,
        LevelFilter::WARN => matches!(*level, tracing::Level::ERROR | tracing::Level::WARN),
        LevelFilter::INFO => matches!(
            *level,
            tracing::Level::ERROR | tracing::Level::WARN | tracing::Level::INFO
        ),
        LevelFilter::DEBUG => *level != tracing::Level::TRACE,
        LevelFilter::TRACE => true,
    }
}

fn format_attributes(attributes: &[Attribute]) -> String {
    let mut output = String::new();
    for (index, attribute) in attributes.iter().enumerate() {
        if index > 0 {
            output.push(' ');
        }
        append_quoted_if_needed(&mut output, &attribute.key);
        output.push('=');
        match &attribute.value {
            AttributeValue::String(value) => append_quoted_if_needed(&mut output, value),
            AttributeValue::Boolean(value) => {
                output.push_str(if *value { "true" } else { "false" })
            }
            AttributeValue::Signed(value) => output.push_str(&value.to_string()),
            AttributeValue::Unsigned(value) => output.push_str(&value.to_string()),
            AttributeValue::Float(value) => output.push_str(&format_go_float(*value)),
        }
    }
    output.push('\n');
    output
}

fn append_quoted_if_needed(output: &mut String, value: &str) {
    if needs_quoting(value) {
        append_go_quote(output, value);
    } else {
        output.push_str(value);
    }
}

fn needs_quoting(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    value.chars().any(|character| {
        character.is_whitespace()
            || character == '='
            || character == '"'
            || character.is_control()
            || !is_printable(character)
    })
}

fn is_printable(character: char) -> bool {
    character.is_ascii() || !character.escape_debug().to_string().starts_with("\\u{")
}

fn append_go_quote(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '\u{0007}' => output.push_str("\\a"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000C}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{000B}' => output.push_str("\\v"),
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            character if character <= '\u{001F}' || character == '\u{007F}' => {
                use fmt::Write as _;
                write!(output, "\\x{:02x}", character as u32)
                    .expect("writing to String cannot fail");
            }
            character if !is_printable(character) => {
                use fmt::Write as _;
                if character <= '\u{FFFF}' {
                    write!(output, "\\u{:04x}", character as u32)
                        .expect("writing to String cannot fail");
                } else {
                    write!(output, "\\U{:08x}", character as u32)
                        .expect("writing to String cannot fail");
                }
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

fn format_go_float(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value == f64::INFINITY {
        return "+Inf".to_owned();
    }
    if value == f64::NEG_INFINITY {
        return "-Inf".to_owned();
    }

    let shortest = value.to_string();
    if shortest.contains('e') || shortest.contains('E') {
        return normalize_exponent(&shortest);
    }

    let negative = shortest.starts_with('-');
    let unsigned = shortest.strip_prefix('-').unwrap_or(&shortest);
    let Some(dot) = unsigned.find('.') else {
        let digits = unsigned.trim_start_matches('0');
        let exponent = digits.len().saturating_sub(1) as i32;
        return if unsigned != "0" && exponent >= 6 {
            scientific_from_digits(negative, digits, exponent)
        } else {
            shortest
        };
    };

    let integer = &unsigned[..dot];
    let fraction = &unsigned[dot + 1..];
    if integer != "0" {
        let mut digits = format!("{integer}{fraction}");
        while digits.ends_with('0') {
            digits.pop();
        }
        let exponent = integer.len() as i32 - 1;
        if exponent >= 6 {
            scientific_from_digits(negative, &digits, exponent)
        } else {
            shortest
        }
    } else {
        let zeros = fraction
            .chars()
            .take_while(|character| *character == '0')
            .count();
        let exponent = -(zeros as i32) - 1;
        if exponent < -4 {
            scientific_from_digits(negative, &fraction[zeros..], exponent)
        } else {
            shortest
        }
    }
}

fn scientific_from_digits(negative: bool, digits: &str, exponent: i32) -> String {
    let mut output = String::new();
    if negative {
        output.push('-');
    }
    let mut characters = digits.chars();
    output.push(characters.next().unwrap_or('0'));
    let remainder = characters.as_str().trim_end_matches('0');
    if !remainder.is_empty() {
        output.push('.');
        output.push_str(remainder);
    }
    append_exponent(&mut output, exponent);
    output
}

fn normalize_exponent(value: &str) -> String {
    let (mantissa, exponent) = value
        .split_once(['e', 'E'])
        .expect("caller checked for exponent");
    let exponent = exponent.parse::<i32>().expect("float exponent is numeric");
    let mut output = mantissa.to_owned();
    append_exponent(&mut output, exponent);
    output
}

fn append_exponent(output: &mut String, exponent: i32) {
    use fmt::Write as _;
    let sign = if exponent < 0 { '-' } else { '+' };
    write!(output, "e{sign}{:02}", exponent.unsigned_abs()).expect("writing to String cannot fail");
}

use tracing_subscriber::prelude::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn debug_truth_table_is_case_insensitive_without_trimming() {
        for accepted in ["1", "true", "TRUE", "TrUe", "yes", "YES"] {
            assert!(
                debug_value_is_truthy(Some(OsStr::new(accepted))),
                "{accepted}"
            );
        }
        for rejected in ["", "0", "false", "no", " true", "true ", "yes\n"] {
            assert!(
                !debug_value_is_truthy(Some(OsStr::new(rejected))),
                "{rejected:?}"
            );
        }
        assert!(!debug_value_is_truthy(None));
    }

    #[test]
    fn false_debug_value_is_a_no_op_and_true_value_reloads() {
        let (_subscriber, handle) =
            reloadable_subscriber(TextLayer::new(io::sink(), LevelFilter::INFO));

        let false_factory_called = Cell::new(false);
        let changed = configure_from_debug_value(&handle, Some(OsStr::new("true ")), || {
            false_factory_called.set(true);
            TextLayer::new(io::sink(), LevelFilter::DEBUG)
        })
        .expect("false DEBUG value does not reload");
        assert!(!changed);
        assert!(!false_factory_called.get());

        let true_factory_called = Cell::new(false);
        let changed = configure_from_debug_value(&handle, Some(OsStr::new("YeS")), || {
            true_factory_called.set(true);
            TextLayer::new(io::sink(), LevelFilter::DEBUG)
        })
        .expect("truthy DEBUG value reloads");
        assert!(changed);
        assert!(true_factory_called.get());
    }

    #[test]
    fn go_float_format_uses_the_oracle_exponent_boundaries() {
        assert_eq!(format_go_float(1_000_000.0), "1e+06");
        assert_eq!(format_go_float(999_999.0), "999999");
        assert_eq!(format_go_float(0.0001), "0.0001");
        assert_eq!(format_go_float(0.00001), "1e-05");
        assert_eq!(format_go_float(f64::INFINITY), "+Inf");
        assert_eq!(format_go_float(f64::NEG_INFINITY), "-Inf");
        assert_eq!(format_go_float(f64::NAN), "NaN");
    }
}
