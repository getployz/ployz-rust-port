use std::io::{self, Write};
use std::sync::{Arc, Barrier, Mutex};

use ployz_internal_log::{Attribute, TextHandler, TextLayer, reloadable_subscriber};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("buffer lock").clone()).expect("UTF-8 log")
    }
}

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("buffer lock").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn default_info_threshold_and_all_level_labels() {
    let output = Buffer::default();
    let subscriber =
        tracing_subscriber::registry().with(TextLayer::with_default_level(output.clone()));
    tracing::subscriber::with_default(subscriber, || {
        tracing::debug!("hidden");
        tracing::info!("info");
        tracing::warn!("warn");
        tracing::error!("error");
    });
    assert_eq!(output.text(), "INFO  info \nWARN  warn \nERROR error \n");
}

#[test]
fn global_event_path_discards_writer_errors() {
    let subscriber = tracing_subscriber::registry()
        .with(TextLayer::new(FailingWriter::new(1), LevelFilter::DEBUG));
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(key = "value", "fire and forget");
    });
}

#[test]
fn reload_replaces_filter_and_handler_without_reinstalling_dispatch() {
    let output = Buffer::default();
    let (subscriber, handle) =
        reloadable_subscriber(TextLayer::new(output.clone(), LevelFilter::WARN));
    tracing::subscriber::with_default(subscriber, || {
        emit_reloaded_info();
        handle
            .reload(TextLayer::new(output.clone(), LevelFilter::DEBUG))
            .expect("reload layer");
        emit_reloaded_info();
        tracing::debug!("visible after replacement");
    });
    assert_eq!(
        output.text(),
        "INFO  visible after replacement \nDEBUG visible after replacement \n"
    );
}

fn emit_reloaded_info() {
    tracing::info!("visible after replacement");
}

#[test]
fn inherited_attributes_and_nested_empty_groups_keep_order() {
    let output = Buffer::default();
    let subscriber =
        tracing_subscriber::registry().with(TextLayer::new(output.clone(), LevelFilter::DEBUG));
    tracing::subscriber::with_default(subscriber, || {
        let root = tracing::debug_span!("context", component = "dns");
        let _root = root.enter();
        let group = tracing::trace_span!("request", ployz.group = "request", name = "example.org");
        let _group = group.enter();
        let empty = tracing::trace_span!("empty", ployz.group = "details");
        let _empty = empty.enter();
        tracing::info!(kind = "A", "received");
    });
    assert_eq!(
        output.text(),
        "INFO  received component=dns request.name=example.org request.details.kind=A\n"
    );
}

#[test]
fn empty_group_names_are_no_ops_at_root_and_nested_depths() {
    let output = Buffer::default();
    let subscriber =
        tracing_subscriber::registry().with(TextLayer::new(output.clone(), LevelFilter::DEBUG));
    tracing::subscriber::with_default(subscriber, || {
        let empty_root = tracing::info_span!("empty-root", ployz.group = "", root = "value");
        let _empty_root = empty_root.enter();
        tracing::info!(event = "root", "root empty");

        let parent = tracing::info_span!("parent", ployz.group = "parent", parent = "value");
        let _parent = parent.enter();
        let empty_nested = tracing::info_span!("empty-nested", ployz.group = "", nested = "value");
        let _empty_nested = empty_nested.enter();
        tracing::info!(event = "nested", "nested empty");
    });
    assert_eq!(
        output.text(),
        concat!(
            "INFO  root empty root=value event=root\n",
            "INFO  nested empty root=value parent.parent=value parent.nested=value ",
            "parent.event=nested\n"
        )
    );
}

#[test]
fn direct_handler_filters_and_returns_first_and_second_write_errors() {
    let filtered = TextHandler::new(FailingWriter::new(1), LevelFilter::INFO);
    filtered
        .handle(&tracing::Level::DEBUG, "hidden", &[])
        .expect("filtered records do not write");

    let first = TextHandler::new(FailingWriter::new(1), LevelFilter::DEBUG)
        .handle(&tracing::Level::INFO, "message", &[])
        .expect_err("first write fails");
    assert_eq!(first.to_string(), "write failure 1");

    let second = TextHandler::new(FailingWriter::new(2), LevelFilter::DEBUG)
        .handle(
            &tracing::Level::INFO,
            "message",
            &[Attribute::string("key", "value")],
        )
        .expect_err("second write fails");
    assert_eq!(second.to_string(), "write failure 2");

    for fail_on in [1, 2] {
        let interrupted = TextHandler::new(InterruptedOnceWriter::new(fail_on), LevelFilter::DEBUG)
            .handle(&tracing::Level::INFO, "message", &[])
            .expect_err("Interrupted is returned without retrying the write");
        assert_eq!(interrupted.kind(), io::ErrorKind::Interrupted);
        assert_eq!(
            interrupted.to_string(),
            format!("interrupted write {fail_on}")
        );
    }

    let output = Buffer::default();
    TextHandler::with_default_level(output.clone())
        .handle(
            &tracing::Level::INFO,
            "reserved",
            &[
                Attribute::string("time", "removed"),
                Attribute::string("level", "removed"),
                Attribute::string("msg", "removed"),
                Attribute::string("request.time", "kept"),
            ],
        )
        .expect("write direct record");
    assert_eq!(output.text(), "INFO  reserved request.time=kept\n");
}

struct FailingWriter {
    fail_on: usize,
    writes: usize,
}

impl FailingWriter {
    fn new(fail_on: usize) -> Self {
        Self { fail_on, writes: 0 }
    }
}

impl Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        if self.writes == self.fail_on {
            Err(io::Error::other(format!("write failure {}", self.writes)))
        } else {
            Ok(bytes.len())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct InterruptedOnceWriter {
    fail_on: usize,
    writes: usize,
}

impl InterruptedOnceWriter {
    fn new(fail_on: usize) -> Self {
        Self { fail_on, writes: 0 }
    }
}

impl Write for InterruptedOnceWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        if self.writes == self.fail_on {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("interrupted write {}", self.writes),
            ))
        } else {
            Ok(bytes.len())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn cloned_layers_serialize_complete_records_across_threads() {
    let output = Buffer::default();
    let layer = TextLayer::new(output.clone(), LevelFilter::DEBUG);
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(layer));
    let barrier = Arc::new(Barrier::new(9));
    let mut threads = Vec::new();

    for thread_id in 0..8 {
        let dispatch = dispatch.clone();
        let barrier = Arc::clone(&barrier);
        threads.push(std::thread::spawn(move || {
            tracing::dispatcher::with_default(&dispatch, || {
                let span = tracing::info_span!("worker", worker = thread_id);
                let _span = span.enter();
                barrier.wait();
                for sequence in 0..100 {
                    tracing::info!(sequence, "record");
                }
            });
        }));
    }
    barrier.wait();
    for thread in threads {
        thread.join().expect("logging thread");
    }

    let lines = output.text();
    assert_eq!(lines.lines().count(), 800);
    assert!(lines.lines().all(|line| {
        line.starts_with("INFO  record worker=")
            && line.contains(" sequence=")
            && line.matches("INFO  record").count() == 1
    }));
}
