use std::fmt::{self, Write as _};
use std::io::Write as _;

use flate2::{Compression, write::GzEncoder};
use prometheus::{Encoder, ProtobufEncoder, TextEncoder, proto};
use protobuf::{Message, UnknownValueRef, well_known_types::timestamp::Timestamp};

const PROTO_PREFIX: &str =
    "application/vnd.google.protobuf; proto=io.prometheus.client.MetricFamily; encoding=";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Format {
    Text,
    ProtoDelimited,
    ProtoCompact,
    ProtoText,
}

impl Format {
    pub(crate) fn content_type(self, escaping: &str) -> String {
        let base = match self {
            Self::Text => "text/plain; version=0.0.4; charset=utf-8".to_owned(),
            Self::ProtoDelimited => format!("{PROTO_PREFIX}delimited"),
            Self::ProtoCompact => format!("{PROTO_PREFIX}compact-text"),
            Self::ProtoText => format!("{PROTO_PREFIX}text"),
        };
        format!("{base}; escaping={escaping}")
    }
}

#[derive(Debug)]
pub(crate) enum EncodeError {
    Prometheus(prometheus::Error),
    Protobuf(protobuf::Error),
    Compression(std::io::Error),
    Unsupported(String),
}

impl fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prometheus(error) => write!(formatter, "encode Prometheus metrics: {error}"),
            Self::Protobuf(error) => write!(formatter, "decode created timestamp: {error}"),
            Self::Compression(error) => write!(formatter, "compress metrics response: {error}"),
            Self::Unsupported(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for EncodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Prometheus(error) => Some(error),
            Self::Protobuf(error) => Some(error),
            Self::Compression(error) => Some(error),
            Self::Unsupported(_) => None,
        }
    }
}

impl From<prometheus::Error> for EncodeError {
    fn from(error: prometheus::Error) -> Self {
        Self::Prometheus(error)
    }
}

impl From<protobuf::Error> for EncodeError {
    fn from(error: protobuf::Error) -> Self {
        Self::Protobuf(error)
    }
}

pub(crate) fn negotiate_format(accept: &str) -> (Format, &'static str) {
    let mut clauses = accept
        .split(',')
        .filter_map(parse_accept_clause)
        .collect::<Vec<_>>();
    sort_accept_clauses(&mut clauses);

    let mut escaping = "underscores";
    for clause in clauses {
        escaping = clause
            .params
            .iter()
            .rev()
            .find(|(key, _)| *key == "escaping")
            .map_or(escaping, |(_, value)| match *value {
                "allow-utf-8" => "allow-utf-8",
                "dots" => "dots",
                "underscores" => "underscores",
                "values" => "values",
                _ => escaping,
            });
        let parameter = |name: &str| {
            clause
                .params
                .iter()
                .rev()
                .find_map(|(key, value)| (*key == name).then_some(*value))
                .unwrap_or("")
        };

        if clause.kind == "application"
            && clause.subtype == "vnd.google.protobuf"
            && parameter("proto") == "io.prometheus.client.MetricFamily"
        {
            let format = match parameter("encoding") {
                "delimited" => Some(Format::ProtoDelimited),
                "compact-text" => Some(Format::ProtoCompact),
                "text" => Some(Format::ProtoText),
                _ => None,
            };
            if let Some(format) = format {
                return (format, escaping);
            }
        }

        if clause.kind == "text"
            && clause.subtype == "plain"
            && matches!(parameter("version"), "" | "0.0.4")
        {
            return (Format::Text, escaping);
        }
    }

    (Format::Text, escaping)
}

struct AcceptClause<'a> {
    kind: &'a str,
    subtype: &'a str,
    quality: f32,
    params: Vec<(&'a str, &'a str)>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SortedHint {
    Unknown,
    Increasing,
    Decreasing,
}

// goautoneg v0.0.0-20191010083416-a7dc8b61c822 exposes the ordering of
// Go 1.26's unstable sort through expfmt.Negotiate. This is a bounded port of
// that pdqsort over AcceptClause, kept here because a different unstable sort
// changes escaping selection for equal-quality wildcard clauses.
fn sort_accept_clauses(clauses: &mut [AcceptClause<'_>]) {
    if clauses.len() > 1 {
        let limit = usize::BITS as usize - clauses.len().leading_zeros() as usize;
        pdqsort(clauses, 0, clauses.len(), limit);
    }
}

fn insertion_sort(clauses: &mut [AcceptClause<'_>], start: usize, end: usize) {
    for index in start + 1..end {
        let mut cursor = index;
        while cursor > start && accept_precedes(&clauses[cursor], &clauses[cursor - 1]) {
            clauses.swap(cursor, cursor - 1);
            cursor -= 1;
        }
    }
}

fn sift_down(clauses: &mut [AcceptClause<'_>], low: usize, high: usize, first: usize) {
    let mut root = low;
    loop {
        let mut child = 2 * root + 1;
        if child >= high {
            break;
        }
        if child + 1 < high && accept_precedes(&clauses[first + child], &clauses[first + child + 1])
        {
            child += 1;
        }
        if !accept_precedes(&clauses[first + root], &clauses[first + child]) {
            return;
        }
        clauses.swap(first + root, first + child);
        root = child;
    }
}

fn heap_sort(clauses: &mut [AcceptClause<'_>], start: usize, end: usize) {
    let high = end - start;
    for index in (0..=(high - 1) / 2).rev() {
        sift_down(clauses, index, high, start);
    }
    for index in (0..high).rev() {
        clauses.swap(start, start + index);
        sift_down(clauses, 0, index, start);
    }
}

fn pdqsort(clauses: &mut [AcceptClause<'_>], mut start: usize, mut end: usize, mut limit: usize) {
    let mut was_balanced = true;
    let mut was_partitioned = true;
    loop {
        let length = end - start;
        if length <= 12 {
            insertion_sort(clauses, start, end);
            return;
        }
        if limit == 0 {
            heap_sort(clauses, start, end);
            return;
        }
        if !was_balanced {
            break_patterns(clauses, start, end);
            limit -= 1;
        }
        let (mut pivot, mut hint) = choose_pivot(clauses, start, end);
        if hint == SortedHint::Decreasing {
            reverse_range(clauses, start, end);
            pivot = (end - 1) - (pivot - start);
            hint = SortedHint::Increasing;
        }
        if was_balanced
            && was_partitioned
            && hint == SortedHint::Increasing
            && partial_insertion_sort(clauses, start, end)
        {
            return;
        }
        if start > 0 && !accept_precedes(&clauses[start - 1], &clauses[pivot]) {
            start = partition_equal(clauses, start, end, pivot);
            continue;
        }
        let (middle, already_partitioned) = partition(clauses, start, end, pivot);
        was_partitioned = already_partitioned;
        let left_length = middle - start;
        let right_length = end - middle;
        let balance_threshold = length / 8;
        if left_length < right_length {
            was_balanced = left_length >= balance_threshold;
            pdqsort(clauses, start, middle, limit);
            start = middle + 1;
        } else {
            was_balanced = right_length >= balance_threshold;
            pdqsort(clauses, middle + 1, end, limit);
            end = middle;
        }
    }
}

fn partition(
    clauses: &mut [AcceptClause<'_>],
    start: usize,
    end: usize,
    pivot: usize,
) -> (usize, bool) {
    clauses.swap(start, pivot);
    let mut left = start + 1;
    let mut right = end - 1;
    while left <= right && accept_precedes(&clauses[left], &clauses[start]) {
        left += 1;
    }
    while left <= right && !accept_precedes(&clauses[right], &clauses[start]) {
        right -= 1;
    }
    if left > right {
        clauses.swap(right, start);
        return (right, true);
    }
    clauses.swap(left, right);
    left += 1;
    right -= 1;
    loop {
        while left <= right && accept_precedes(&clauses[left], &clauses[start]) {
            left += 1;
        }
        while left <= right && !accept_precedes(&clauses[right], &clauses[start]) {
            right -= 1;
        }
        if left > right {
            break;
        }
        clauses.swap(left, right);
        left += 1;
        right -= 1;
    }
    clauses.swap(right, start);
    (right, false)
}

fn partition_equal(
    clauses: &mut [AcceptClause<'_>],
    start: usize,
    end: usize,
    pivot: usize,
) -> usize {
    clauses.swap(start, pivot);
    let mut left = start + 1;
    let mut right = end - 1;
    loop {
        while left <= right && !accept_precedes(&clauses[start], &clauses[left]) {
            left += 1;
        }
        while left <= right && accept_precedes(&clauses[start], &clauses[right]) {
            right -= 1;
        }
        if left > right {
            break;
        }
        clauses.swap(left, right);
        left += 1;
        right -= 1;
    }
    left
}

fn partial_insertion_sort(clauses: &mut [AcceptClause<'_>], start: usize, end: usize) -> bool {
    let mut index = start + 1;
    for _ in 0..5 {
        while index < end && !accept_precedes(&clauses[index], &clauses[index - 1]) {
            index += 1;
        }
        if index == end {
            return true;
        }
        if end - start < 50 {
            return false;
        }
        clauses.swap(index, index - 1);
        if index - start >= 2 {
            let mut cursor = index - 1;
            while cursor >= 1 {
                if !accept_precedes(&clauses[cursor], &clauses[cursor - 1]) {
                    break;
                }
                clauses.swap(cursor, cursor - 1);
                cursor -= 1;
            }
        }
        if end - index >= 2 {
            for cursor in index + 1..end {
                if !accept_precedes(&clauses[cursor], &clauses[cursor - 1]) {
                    break;
                }
                clauses.swap(cursor, cursor - 1);
            }
        }
    }
    false
}

fn break_patterns(clauses: &mut [AcceptClause<'_>], start: usize, end: usize) {
    let length = end - start;
    if length >= 8 {
        let mut random = length;
        let modulus = 1_usize << (usize::BITS as usize - length.leading_zeros() as usize);
        let center = start + (length / 4) * 2;
        for index in center - 1..=center + 1 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let mut other = random & (modulus - 1);
            if other >= length {
                other -= length;
            }
            clauses.swap(index, start + other);
        }
    }
}

fn choose_pivot(clauses: &[AcceptClause<'_>], start: usize, end: usize) -> (usize, SortedHint) {
    let length = end - start;
    let mut swaps = 0;
    let mut first = start + length / 4;
    let mut middle = start + (length / 4) * 2;
    let mut last = start + (length / 4) * 3;
    if length >= 8 {
        if length >= 50 {
            first = median_adjacent(clauses, first, &mut swaps);
            middle = median_adjacent(clauses, middle, &mut swaps);
            last = median_adjacent(clauses, last, &mut swaps);
        }
        middle = median(clauses, first, middle, last, &mut swaps);
    }
    let hint = match swaps {
        0 => SortedHint::Increasing,
        12 => SortedHint::Decreasing,
        _ => SortedHint::Unknown,
    };
    (middle, hint)
}

fn order_two(
    clauses: &[AcceptClause<'_>],
    first: usize,
    second: usize,
    swaps: &mut usize,
) -> (usize, usize) {
    if accept_precedes(&clauses[second], &clauses[first]) {
        *swaps += 1;
        (second, first)
    } else {
        (first, second)
    }
}

fn median(
    clauses: &[AcceptClause<'_>],
    first: usize,
    middle: usize,
    last: usize,
    swaps: &mut usize,
) -> usize {
    let (first, middle) = order_two(clauses, first, middle, swaps);
    let (middle, last) = order_two(clauses, middle, last, swaps);
    let (_, middle) = order_two(clauses, first, middle, swaps);
    let _ = last;
    middle
}

fn median_adjacent(clauses: &[AcceptClause<'_>], index: usize, swaps: &mut usize) -> usize {
    median(clauses, index - 1, index, index + 1, swaps)
}

fn reverse_range(clauses: &mut [AcceptClause<'_>], start: usize, end: usize) {
    let mut left = start;
    let mut right = end - 1;
    while left < right {
        clauses.swap(left, right);
        left += 1;
        right -= 1;
    }
}

fn accept_precedes(left: &AcceptClause<'_>, right: &AcceptClause<'_>) -> bool {
    left.quality > right.quality
        || (left.kind != "*" && right.kind == "*")
        || (left.subtype != "*" && right.subtype == "*")
}

fn parse_accept_clause(raw: &str) -> Option<AcceptClause<'_>> {
    let mut parts = raw.trim_matches(' ').split(';');
    let media_type = parts.next()?.trim_matches(' ');
    let (kind, subtype) = media_type
        .split_once('/')
        .or_else(|| (media_type == "*").then_some(("*", "*")))?;
    if subtype.contains('/') {
        return None;
    }
    let kind = kind.trim_matches(' ');
    let subtype = subtype.trim_matches(' ');
    let mut quality = 1.0;
    let mut params = Vec::new();
    for part in parts {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        if value.contains('=') {
            continue;
        }
        let key = key.trim_matches(' ');
        if key == "q" {
            quality = value.parse().unwrap_or(0.0);
        } else {
            params.push((key, value.trim_matches(' ')));
        }
    }
    Some(AcceptClause {
        kind,
        subtype,
        quality,
        params,
    })
}

pub(crate) fn accepts_gzip(headers: &[&str]) -> bool {
    let mut best = ("identity", -1.0_f64);
    let specs = headers
        .iter()
        .flat_map(|header| parse_encoding_header(header))
        .collect::<Vec<_>>();
    for offer in ["identity", "gzip"] {
        for (value, quality) in &specs {
            if *value != "*" && *value != offer {
                continue;
            }
            if *quality > best.1 {
                best = (offer, *quality);
            }
        }
    }

    // Promhttp falls back to identity if negotiation produces no acceptable
    // offer, including when every recognized offer has q=0.
    best.1 > 0.0 && best.0 == "gzip"
}

fn parse_encoding_header(mut remaining: &str) -> Vec<(&str, f64)> {
    let mut specs = Vec::new();
    loop {
        remaining = remaining.trim_start_matches([' ', '\t', '\r', '\n']);
        let token_end = remaining
            .bytes()
            .position(|byte| !is_encoding_token(byte))
            .unwrap_or(remaining.len());
        if token_end == 0 {
            break;
        }
        let value = &remaining[..token_end];
        remaining = remaining[token_end..].trim_start_matches([' ', '\t', '\r', '\n']);
        let mut quality = 1.0;
        if let Some(after_semicolon) = remaining.strip_prefix(';') {
            remaining = after_semicolon.trim_start_matches([' ', '\t', '\r', '\n']);
            let Some(after_quality) = remaining.strip_prefix("q=") else {
                break;
            };
            let Some((parsed, rest)) = parse_quality_prefix(after_quality) else {
                break;
            };
            quality = parsed;
            remaining = rest;
        }
        specs.push((value, quality));
        remaining = remaining.trim_start_matches([' ', '\t', '\r', '\n']);
        let Some(after_comma) = remaining.strip_prefix(',') else {
            break;
        };
        remaining = after_comma;
    }
    specs
}

fn is_encoding_token(byte: u8) -> bool {
    byte.is_ascii_graphic() && !b"\"(),:;<=>?@[\\]{}".contains(&byte)
}

fn parse_quality_prefix(raw: &str) -> Option<(f64, &str)> {
    let whole = *raw.as_bytes().first()?;
    if !matches!(whole, b'0' | b'1') {
        return None;
    }
    let mut quality = if whole == b'1' { 1.0 } else { 0.0 };
    let mut remaining = &raw[1..];
    let Some(fraction) = remaining.strip_prefix('.') else {
        return Some((quality, remaining));
    };
    let fraction_end = fraction
        .bytes()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(fraction.len());
    if fraction_end > 0 {
        quality += fraction[..fraction_end].parse::<f64>().ok()? / 10_f64.powi(fraction_end as i32);
    }
    remaining = &fraction[fraction_end..];
    Some((quality, remaining))
}

pub(crate) fn encode(
    families: &[proto::MetricFamily],
    format: Format,
    escaping: &str,
) -> Result<Vec<u8>, EncodeError> {
    validate_families(families)?;
    let escaped;
    let families = if format == Format::ProtoDelimited || escaping == "allow-utf-8" {
        families
    } else {
        escaped = escape_metric_families(families, escaping);
        &escaped
    };
    let mut output = Vec::new();
    match format {
        Format::Text if text_needs_quoted_names(families) => {
            write_utf8_text(families, &mut output)?;
        }
        Format::Text => TextEncoder::new().encode(families, &mut output)?,
        Format::ProtoDelimited => ProtobufEncoder::new().encode(families, &mut output)?,
        Format::ProtoCompact => {
            for family in families {
                write_compact(family, &mut output)?;
            }
        }
        Format::ProtoText => {
            for family in families {
                write_pretty(family, &mut output)?;
            }
        }
    }
    Ok(output)
}

fn escape_metric_families(
    families: &[proto::MetricFamily],
    escaping: &str,
) -> Vec<proto::MetricFamily> {
    let mut escaped = families.to_vec();
    for family in &mut escaped {
        if let Some(name) = &mut family.name
            && !is_valid_legacy_name(name)
        {
            *name = escape_name(name, escaping);
        }
        for metric in &mut family.metric {
            for label in &mut metric.label {
                if label.name() == "__name__" {
                    if let Some(value) = &mut label.value
                        && !is_valid_legacy_name(value)
                    {
                        *value = escape_name(value, escaping);
                    }
                } else if let Some(name) = &mut label.name
                    && !is_valid_legacy_name(name)
                {
                    *name = escape_name(name, escaping);
                }
            }
        }
    }
    escaped
}

fn is_valid_legacy_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .enumerate()
            .all(|(index, character)| is_valid_legacy_character(character, index))
}

fn escape_name(name: &str, escaping: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    match escaping {
        "underscores" => name
            .chars()
            .enumerate()
            .map(|(index, character)| {
                if is_valid_legacy_character(character, index) {
                    character
                } else {
                    '_'
                }
            })
            .collect(),
        "dots" => {
            let mut escaped = String::with_capacity(name.len());
            for (index, character) in name.chars().enumerate() {
                match character {
                    '_' => escaped.push_str("__"),
                    '.' => escaped.push_str("_dot_"),
                    _ if is_valid_legacy_character(character, index) => escaped.push(character),
                    _ => escaped.push_str("__"),
                }
            }
            escaped
        }
        "values" => {
            let mut escaped = String::with_capacity(name.len() + 3);
            escaped.push_str("U__");
            for (index, character) in name.chars().enumerate() {
                if character == '_' {
                    escaped.push_str("__");
                } else if is_valid_legacy_character(character, index) {
                    escaped.push(character);
                } else {
                    write!(escaped, "_{:x}_", character as u32)
                        .expect("writing to a string cannot fail");
                }
            }
            escaped
        }
        _ => unreachable!("negotiation returned an unknown escaping scheme"),
    }
}

fn is_valid_legacy_character(character: char, index: usize) -> bool {
    character.is_ascii_alphabetic()
        || matches!(character, '_' | ':')
        || (index > 0 && character.is_ascii_digit())
}

fn text_needs_quoted_names(families: &[proto::MetricFamily]) -> bool {
    families.iter().any(|family| {
        !is_valid_legacy_name(family.name())
            || family.metric.iter().any(|metric| {
                metric
                    .label
                    .iter()
                    .any(|label| !is_valid_legacy_name(label.name()))
            })
    })
}

fn write_utf8_text(
    families: &[proto::MetricFamily],
    output: &mut Vec<u8>,
) -> Result<(), EncodeError> {
    let mut text = String::new();
    for family in families {
        let name = family.name();
        if name.is_empty() {
            return Err(unsupported("metric family has no name"));
        }
        if !family.help().is_empty() {
            text.push_str("# HELP ");
            write_text_name(&mut text, name);
            text.push(' ');
            write_text_escaped(&mut text, family.help(), false);
            text.push('\n');
        }
        text.push_str("# TYPE ");
        write_text_name(&mut text, name);
        match family.type_.unwrap().enum_value().unwrap() {
            proto::MetricType::COUNTER => text.push_str(" counter\n"),
            proto::MetricType::GAUGE => text.push_str(" gauge\n"),
            _ => unreachable!("unsupported metric type was validated"),
        }
        for metric in &family.metric {
            write_text_sample_name_and_labels(&mut text, name, &metric.label);
            text.push(' ');
            let value = metric
                .counter
                .as_ref()
                .map(|counter| counter.value())
                .or_else(|| metric.gauge.as_ref().map(|gauge| gauge.value()))
                .expect("metric shape was validated");
            text.push_str(&canonical_float(value));
            text.push('\n');
        }
    }
    output.extend_from_slice(text.as_bytes());
    Ok(())
}

fn write_text_sample_name_and_labels(text: &mut String, name: &str, labels: &[proto::LabelPair]) {
    let name_inside_braces = !is_valid_legacy_name(name);
    if name_inside_braces {
        text.push('{');
        write_text_name(text, name);
    } else {
        text.push_str(name);
    }
    if !labels.is_empty() && !name_inside_braces {
        text.push('{');
    }
    for (index, label) in labels.iter().enumerate() {
        if name_inside_braces || index > 0 {
            text.push(',');
        }
        write_text_name(text, label.name());
        text.push_str("=\"");
        write_text_escaped(text, label.value(), true);
        text.push('"');
    }
    if name_inside_braces || !labels.is_empty() {
        text.push('}');
    }
}

fn write_text_name(text: &mut String, name: &str) {
    if is_valid_legacy_name(name) {
        text.push_str(name);
    } else {
        text.push('"');
        write_text_escaped(text, name, true);
        text.push('"');
    }
}

fn write_text_escaped(text: &mut String, value: &str, include_double_quote: bool) {
    for character in value.chars() {
        match character {
            '\\' => text.push_str("\\\\"),
            '\n' => text.push_str("\\n"),
            '"' if include_double_quote => text.push_str("\\\""),
            _ => text.push(character),
        }
    }
}

pub(crate) fn gzip(payload: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(payload)?;
    encoder.finish()
}

fn validate_families(families: &[proto::MetricFamily]) -> Result<(), EncodeError> {
    for family in families {
        ensure_no_unknown("metric family", family.special_fields.unknown_fields(), &[])?;
        if family.name.is_none() || family.help.is_none() || family.type_.is_none() {
            return Err(unsupported("metric family is missing name, help, or type"));
        }
        let metric_type = family
            .type_
            .expect("type checked above")
            .enum_value()
            .map_err(|value| unsupported(format!("unknown metric type {value}")))?;
        if !matches!(
            metric_type,
            proto::MetricType::COUNTER | proto::MetricType::GAUGE
        ) {
            return Err(unsupported(format!(
                "metric family {:?} has unsupported type {metric_type:?}",
                family.name()
            )));
        }

        for metric in &family.metric {
            ensure_no_unknown("metric", metric.special_fields.unknown_fields(), &[])?;
            if metric.timestamp_ms.is_some()
                || metric.summary.is_some()
                || metric.untyped.is_some()
                || metric.histogram.is_some()
            {
                return Err(unsupported(format!(
                    "metric family {:?} contains an unsupported metric shape",
                    family.name()
                )));
            }
            for label in &metric.label {
                ensure_no_unknown("label", label.special_fields.unknown_fields(), &[])?;
                if label.name.is_none() || label.value.is_none() {
                    return Err(unsupported("label is missing its name or value"));
                }
            }

            match metric_type {
                proto::MetricType::COUNTER => {
                    if metric.gauge.is_some() || metric.counter.is_none() {
                        return Err(unsupported("counter family has a non-counter metric"));
                    }
                    let counter = metric.counter.as_ref().expect("counter checked above");
                    if counter.value.is_none() {
                        return Err(unsupported("counter is missing its value"));
                    }
                    ensure_finite(counter.value(), "counter")?;
                    validate_created_timestamp(counter)?;
                }
                proto::MetricType::GAUGE => {
                    if metric.counter.is_some() || metric.gauge.is_none() {
                        return Err(unsupported("gauge family has a non-gauge metric"));
                    }
                    let gauge = metric.gauge.as_ref().expect("gauge checked above");
                    if gauge.value.is_none() {
                        return Err(unsupported("gauge is missing its value"));
                    }
                    ensure_no_unknown("gauge", gauge.special_fields.unknown_fields(), &[])?;
                    ensure_finite(gauge.value(), "gauge")?;
                }
                _ => unreachable!("unsupported family types returned above"),
            }
        }
    }
    Ok(())
}

fn ensure_finite(value: f64, kind: &str) -> Result<(), EncodeError> {
    value
        .is_finite()
        .then_some(())
        .ok_or_else(|| unsupported(format!("{kind} value {value} is not supported")))
}

fn validate_created_timestamp(counter: &proto::Counter) -> Result<Timestamp, EncodeError> {
    ensure_no_unknown("counter", counter.special_fields.unknown_fields(), &[3])?;
    let mut values = counter
        .special_fields
        .unknown_fields()
        .iter()
        .filter(|(number, _)| *number == 3);
    let Some((_, UnknownValueRef::LengthDelimited(bytes))) = values.next() else {
        return Err(unsupported(
            "counter is missing length-delimited created_timestamp field 3",
        ));
    };
    if values.next().is_some() {
        return Err(unsupported(
            "counter contains more than one created_timestamp field 3",
        ));
    }
    let timestamp = Timestamp::parse_from_bytes(bytes)?;
    ensure_no_unknown(
        "created timestamp",
        timestamp.special_fields.unknown_fields(),
        &[],
    )?;
    if !(-62_135_596_800..=253_402_300_799).contains(&timestamp.seconds)
        || !(0..=999_999_999).contains(&timestamp.nanos)
    {
        return Err(unsupported(format!(
            "counter created_timestamp is invalid: {}s {}ns",
            timestamp.seconds, timestamp.nanos
        )));
    }
    Ok(timestamp)
}

fn ensure_no_unknown(
    kind: &str,
    fields: &protobuf::UnknownFields,
    allowed: &[u32],
) -> Result<(), EncodeError> {
    if let Some((number, _)) = fields.iter().find(|(number, _)| !allowed.contains(number)) {
        return Err(unsupported(format!(
            "{kind} contains unexpected unknown field {number}"
        )));
    }
    Ok(())
}

fn unsupported(reason: impl Into<String>) -> EncodeError {
    EncodeError::Unsupported(reason.into())
}

fn write_compact(family: &proto::MetricFamily, output: &mut Vec<u8>) -> Result<(), EncodeError> {
    let mut text = String::new();
    compact_field(&mut text, "name", &quoted(family.name()));
    compact_field(&mut text, "help", &quoted(family.help()));
    compact_field(
        &mut text,
        "type",
        &format!("{:?}", family.type_.unwrap().enum_value().unwrap()),
    );
    for metric in &family.metric {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str("metric:{");
        for label in &metric.label {
            text.push_str("label:{name:");
            text.push_str(&quoted(label.name()));
            text.push_str(" value:");
            text.push_str(&quoted(label.value()));
            text.push('}');
            text.push(' ');
        }
        if let Some(counter) = metric.counter.as_ref() {
            let timestamp = validate_created_timestamp(counter)?;
            write!(
                text,
                "counter:{{value:{} created_timestamp:{{",
                canonical_float(counter.value())
            )
            .expect("writing to String cannot fail");
            write_compact_timestamp(&mut text, &timestamp);
            text.push_str("}}");
        } else if let Some(gauge) = metric.gauge.as_ref() {
            write!(text, "gauge:{{value:{}}}", canonical_float(gauge.value()))
                .expect("writing to String cannot fail");
        }
        text.push('}');
    }
    text.push('\n');
    output.extend_from_slice(text.as_bytes());
    Ok(())
}

fn compact_field(text: &mut String, name: &str, value: &str) {
    if !text.is_empty() {
        text.push(' ');
    }
    write!(text, "{name}:{value}").expect("writing to String cannot fail");
}

fn write_pretty(family: &proto::MetricFamily, output: &mut Vec<u8>) -> Result<(), EncodeError> {
    let mut text = String::new();
    writeln!(text, "name: {}", quoted(family.name())).unwrap();
    writeln!(text, "help: {}", quoted(family.help())).unwrap();
    writeln!(
        text,
        "type: {:?}",
        family.type_.unwrap().enum_value().unwrap()
    )
    .unwrap();
    for metric in &family.metric {
        text.push_str("metric: {\n");
        for label in &metric.label {
            text.push_str("  label: {\n");
            writeln!(text, "    name: {}", quoted(label.name())).unwrap();
            writeln!(text, "    value: {}", quoted(label.value())).unwrap();
            text.push_str("  }\n");
        }
        if let Some(counter) = metric.counter.as_ref() {
            let timestamp = validate_created_timestamp(counter)?;
            text.push_str("  counter: {\n");
            writeln!(text, "    value: {}", canonical_float(counter.value())).unwrap();
            text.push_str("    created_timestamp: {\n");
            if timestamp.seconds != 0 {
                writeln!(text, "      seconds: {}", timestamp.seconds).unwrap();
            }
            if timestamp.nanos != 0 {
                writeln!(text, "      nanos: {}", timestamp.nanos).unwrap();
            }
            text.push_str("    }\n  }\n");
        } else if let Some(gauge) = metric.gauge.as_ref() {
            text.push_str("  gauge: {\n");
            writeln!(text, "    value: {}", canonical_float(gauge.value())).unwrap();
            text.push_str("  }\n");
        }
        text.push_str("}\n");
    }
    text.push('\n');
    output.extend_from_slice(text.as_bytes());
    Ok(())
}

fn canonical_float(value: f64) -> String {
    value.to_string()
}

fn write_compact_timestamp(output: &mut String, timestamp: &Timestamp) {
    if timestamp.seconds != 0 {
        write!(output, "seconds:{}", timestamp.seconds).unwrap();
    }
    if timestamp.nanos != 0 {
        if timestamp.seconds != 0 {
            output.push(' ');
        }
        write!(output, "nanos:{}", timestamp.nanos).unwrap();
    }
}

fn quoted(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character <= '\u{1f}' || character == '\u{7f}' => {
                write!(output, "\\x{:02x}", character as u32).unwrap();
            }
            character if ('\u{80}'..='\u{9f}').contains(&character) => {
                write!(output, "\\u{:04x}", character as u32).unwrap();
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

#[cfg(test)]
mod tests {
    use ployz_internal_metrics::CreatedIntCounterVec;
    use prometheus::{Gauge, Opts, Registry, core::Collector};

    use super::*;

    fn fixture() -> Vec<proto::MetricFamily> {
        let registry = Registry::new();
        let counter =
            CreatedIntCounterVec::new(Opts::new("probe_total", "Probe."), &["kind"]).unwrap();
        let gauge = Gauge::new("probe_gauge", "Gauge.").unwrap();
        registry.register(Box::new(counter.clone())).unwrap();
        registry.register(Box::new(gauge.clone())).unwrap();
        counter.with_label_values(&["a\\n\\\"b"]).inc_by(7);
        gauge.set(0.0);
        registry.gather()
    }

    fn utf8_name_fixture() -> proto::MetricFamily {
        let mut named_label = proto::LabelPair::new();
        named_label.name = Some("__name__".to_owned());
        named_label.value = Some("9other.metric_é".to_owned());
        let mut ordinary_label = proto::LabelPair::new();
        ordinary_label.name = Some("bad.label_é".to_owned());
        ordinary_label.value = Some("unchanged".to_owned());
        let mut gauge = proto::Gauge::new();
        gauge.value = Some(1.0);
        let mut metric = proto::Metric::new();
        metric.label.extend([named_label, ordinary_label]);
        metric.gauge = protobuf::MessageField::some(gauge);
        let mut family = proto::MetricFamily::new();
        family.name = Some("9bad.metric_é".to_owned());
        family.help = Some("UTF-8 names.".to_owned());
        family.type_ = Some(proto::MetricType::GAUGE.into());
        family.metric.push(metric);
        family
    }

    #[test]
    fn negotiates_legacy_formats_and_escaping() {
        assert_eq!(negotiate_format(""), (Format::Text, "underscores"));
        assert_eq!(
            negotiate_format(concat!(
                "text/plain;q=0.2, application/vnd.google.protobuf; ",
                "proto=io.prometheus.client.MetricFamily; encoding=delimited; q=0.9"
            )),
            (Format::ProtoDelimited, "underscores")
        );
        assert_eq!(
            negotiate_format("text/plain; version=0.0.4; escaping=dots"),
            (Format::Text, "dots")
        );
        assert_eq!(
            negotiate_format("application/openmetrics-text; version=1.0.0"),
            (Format::Text, "underscores")
        );
        assert_eq!(
            negotiate_format("application/openmetrics-text; version=1.0.0; escaping=dots"),
            (Format::Text, "dots")
        );
        assert_eq!(
            negotiate_format(concat!(
                "application/openmetrics-text; version=1.0.0; escaping=dots, ",
                "text/plain; q=0.5"
            )),
            (Format::Text, "dots")
        );
        assert_eq!(
            negotiate_format(concat!(
                "application/openmetrics-text; escaping=allow-utf-8, ",
                "text/plain; escaping=underscores"
            )),
            (Format::Text, "underscores")
        );
    }

    #[test]
    fn duplicate_accept_parameters_use_the_last_value() {
        assert_eq!(
            negotiate_format("text/plain; version=0.0.4; escaping=dots; escaping=allow-utf-8"),
            (Format::Text, "allow-utf-8")
        );
        assert_eq!(
            negotiate_format(concat!(
                "application/vnd.google.protobuf; proto=wrong; ",
                "proto=io.prometheus.client.MetricFamily; encoding=text; ",
                "encoding=delimited"
            )),
            (Format::ProtoDelimited, "underscores")
        );
        assert_eq!(
            negotiate_format(concat!(
                "text/plain; version=9; version=0.0.4; q=1, ",
                "application/vnd.google.protobuf; ",
                "proto=io.prometheus.client.MetricFamily; encoding=delimited; q=0.5"
            )),
            (Format::Text, "underscores")
        );
    }

    #[test]
    fn accept_parser_matches_goautoneg_edge_cases() {
        assert_eq!(
            negotiate_format(concat!(
                "application / vnd.google.protobuf ; ",
                "proto=io.prometheus.client.MetricFamily; encoding=delimited"
            )),
            (Format::ProtoDelimited, "underscores")
        );
        assert_eq!(
            negotiate_format(concat!(
                "text/plain; q=0.1=bad, application/vnd.google.protobuf; ",
                "proto=io.prometheus.client.MetricFamily; encoding=delimited; q=0.5"
            )),
            (Format::Text, "underscores")
        );
        assert_eq!(
            negotiate_format(concat!(
                "text/plain; q= 0.9, application/vnd.google.protobuf; ",
                "proto=io.prometheus.client.MetricFamily; encoding=delimited; q=0.5"
            )),
            (Format::ProtoDelimited, "underscores")
        );
        assert_eq!(
            negotiate_format(concat!(
                "text/plain; version=0.0.4=bad; q=1, ",
                "application/vnd.google.protobuf; ",
                "proto=io.prometheus.client.MetricFamily; encoding=delimited; q=0.5"
            )),
            (Format::Text, "underscores")
        );
        assert_eq!(
            negotiate_format(concat!(
                "application/vnd.google.protobuf; ",
                "proto=io.prometheus.client.MetricFamily; encoding=delimited; q=0.5, ",
                "text/plain; q=NaN; escaping=dots"
            )),
            (Format::ProtoDelimited, "underscores")
        );
        assert_eq!(
            negotiate_format(concat!(
                "text/*; q=1; escaping=dots, */plain; q=1; escaping=values, ",
                "text/plain; q=0.5"
            )),
            (Format::Text, "underscores")
        );
    }

    #[test]
    fn long_equal_quality_accept_order_matches_go_pdqsort() {
        let mut clauses = vec!["*/*".to_owned()];
        for index in 0..12 {
            let escaping = match index {
                0 => "dots",
                5 => "values",
                _ => "underscores",
            };
            clauses.push(format!("text/plain; escaping={escaping}"));
        }
        assert_eq!(
            negotiate_format(&clauses.join(",")),
            (Format::Text, "values")
        );
    }

    #[test]
    fn gzip_negotiation_matches_promhttp_identity_fallback() {
        for header in ["", "identity", "zstd", "*", "gzip;q=0, identity;q=0"] {
            assert!(!accepts_gzip(&[header]), "{header}");
        }
        for header in ["gzip", "zstd, gzip", "identity;q=0.5, gzip;q=1"] {
            assert!(accepts_gzip(&[header]), "{header}");
        }
        assert!(!accepts_gzip(&["gzip;q=.5", "identity;q=1"]));
        assert!(!accepts_gzip(&["identity;q=.5, gzip"]));
    }

    #[test]
    fn deterministic_legacy_text_locks_spacing_escaping_and_newlines() {
        let families = fixture();
        let compact =
            String::from_utf8(encode(&families, Format::ProtoCompact, "underscores").unwrap())
                .unwrap();
        let pretty =
            String::from_utf8(encode(&families, Format::ProtoText, "underscores").unwrap())
                .unwrap();

        assert!(compact.contains(
            "name:\"probe_total\" help:\"Probe.\" type:COUNTER metric:{label:{name:\"kind\" value:\"a\\\\n\\\\\\\"b\"} counter:{value:7 created_timestamp:{seconds:"
        ));
        assert!(compact.ends_with('\n'));
        assert!(!compact.ends_with("\n\n"));
        assert!(
            pretty.contains("name: \"probe_total\"\nhelp: \"Probe.\"\ntype: COUNTER\nmetric: {\n")
        );
        assert!(pretty.contains("    created_timestamp: {\n      seconds: "));
        assert!(pretty.ends_with("}\n\n"));
        assert_eq!(quoted("\0\u{7f}\u{80}é"), "\"\\x00\\x7f\\u0080é\"");
    }

    #[test]
    fn negotiated_name_escaping_matches_the_go_model_for_each_encoder() {
        let family = utf8_name_fixture();
        let families = [family.clone()];

        let underscores = escape_metric_families(&families, "underscores");
        assert_eq!(underscores[0].name(), "_bad_metric__");
        assert_eq!(underscores[0].metric[0].label[0].name(), "__name__");
        assert_eq!(underscores[0].metric[0].label[0].value(), "_other_metric__");
        assert_eq!(underscores[0].metric[0].label[1].name(), "bad_label__");
        assert_eq!(underscores[0].metric[0].label[1].value(), "unchanged");

        let dots = escape_metric_families(&families, "dots");
        assert_eq!(dots[0].name(), "__bad_dot_metric____");
        assert_eq!(dots[0].metric[0].label[0].value(), "__other_dot_metric____");
        assert_eq!(dots[0].metric[0].label[1].name(), "bad_dot_label____");

        let values = escape_metric_families(&families, "values");
        assert_eq!(values[0].name(), "U___39_bad_2e_metric___e9_");
        assert_eq!(
            values[0].metric[0].label[0].value(),
            "U___39_other_2e_metric___e9_"
        );
        assert_eq!(values[0].metric[0].label[1].name(), "U__bad_2e_label___e9_");

        let text = String::from_utf8(encode(&families, Format::Text, "dots").unwrap()).unwrap();
        let compact =
            String::from_utf8(encode(&families, Format::ProtoCompact, "dots").unwrap()).unwrap();
        let pretty =
            String::from_utf8(encode(&families, Format::ProtoText, "dots").unwrap()).unwrap();
        for rendered in [&text, &compact, &pretty] {
            assert!(rendered.contains("__bad_dot_metric____"));
            assert!(rendered.contains("__other_dot_metric____"));
            assert!(rendered.contains("bad_dot_label____"));
            assert!(!rendered.contains("9bad.metric_é"));
        }

        let utf8_text =
            String::from_utf8(encode(&families, Format::Text, "allow-utf-8").unwrap()).unwrap();
        assert_eq!(
            utf8_text,
            concat!(
                "# HELP \"9bad.metric_é\" UTF-8 names.\n",
                "# TYPE \"9bad.metric_é\" gauge\n",
                "{\"9bad.metric_é\",__name__=\"9other.metric_é\",",
                "\"bad.label_é\"=\"unchanged\"} 1\n"
            )
        );

        assert_eq!(
            encode(&families, Format::ProtoDelimited, "dots").unwrap(),
            encode(&families, Format::ProtoDelimited, "allow-utf-8").unwrap()
        );
        assert_eq!(
            family.name(),
            "9bad.metric_é",
            "encoding must not mutate input"
        );
        assert_eq!(escape_name("", "values"), "");
    }

    #[test]
    fn delimited_wire_retains_created_timestamp_field_three() {
        let families = fixture();
        let encoded = encode(&families, Format::ProtoDelimited, "underscores").unwrap();
        assert!(!encoded.is_empty());
        let counter = families
            .iter()
            .find(|family| family.name() == "probe_total")
            .unwrap()
            .metric[0]
            .counter
            .as_ref()
            .unwrap();
        assert!(counter.special_fields.unknown_fields().get(3).is_some());
    }

    #[test]
    fn delimited_wire_matches_the_approved_go_golden() {
        let mut created = Timestamp::new();
        created.seconds = 1_700_000_000;
        created.nanos = 123_456_789;
        let mut counter = proto::Counter::new();
        counter.value = Some(7.0);
        counter
            .special_fields
            .mut_unknown_fields()
            .add_length_delimited(3, created.write_to_bytes().unwrap());
        let mut label = proto::LabelPair::new();
        label.name = Some("kind".to_owned());
        label.value = Some("a\\n\\\"b".to_owned());
        let mut metric = proto::Metric::new();
        metric.label.push(label);
        metric.counter = protobuf::MessageField::some(counter);
        let mut family = proto::MetricFamily::new();
        family.name = Some("probe_total".to_owned());
        family.help = Some("Probe.".to_owned());
        family.type_ = Some(proto::MetricType::COUNTER.into());
        family.metric.push(metric);

        let encoded = encode(&[family], Format::ProtoDelimited, "underscores").unwrap();
        assert_eq!(
            encoded,
            [
                65, 10, 11, 112, 114, 111, 98, 101, 95, 116, 111, 116, 97, 108, 18, 6, 80, 114,
                111, 98, 101, 46, 24, 0, 34, 40, 10, 14, 10, 4, 107, 105, 110, 100, 18, 6, 97, 92,
                110, 92, 34, 98, 26, 22, 9, 0, 0, 0, 0, 0, 0, 28, 64, 26, 11, 8, 128, 226, 207,
                170, 6, 16, 149, 154, 239, 58,
            ]
        );
    }

    #[test]
    fn bounded_formatter_rejects_uncreated_counters_and_unsupported_families() {
        let plain = prometheus::IntCounter::new("plain_total", "Plain.").unwrap();
        let mut families = plain.collect();
        let error = encode(&families, Format::ProtoText, "underscores").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("missing length-delimited created_timestamp")
        );

        families[0].type_ = Some(proto::MetricType::SUMMARY.into());
        let error = encode(&families, Format::ProtoText, "underscores").unwrap_err();
        assert!(error.to_string().contains("unsupported type SUMMARY"));
    }

    #[test]
    fn wrapped_encoder_errors_preserve_their_sources() {
        let protobuf = EncodeError::from(Timestamp::parse_from_bytes(b"not protobuf").unwrap_err());
        assert!(std::error::Error::source(&protobuf).is_some());

        let prometheus = EncodeError::from(prometheus::Error::Msg("failed".to_owned()));
        assert!(std::error::Error::source(&prometheus).is_some());

        let compression = EncodeError::Compression(std::io::Error::other("failed"));
        assert!(std::error::Error::source(&compression).is_some());
        assert!(std::error::Error::source(&unsupported("bounded rejection")).is_none());
    }
}
