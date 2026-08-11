use ployz_internal_log::{configure_daemon, process_subscriber};
use tracing_subscriber::filter::LevelFilter;

#[test]
fn application_owns_one_shot_global_install_and_can_reload_it() {
    let (subscriber, handle) = process_subscriber(LevelFilter::INFO);
    tracing::subscriber::set_global_default(subscriber)
        .expect("first application install succeeds");
    configure_daemon(&handle).expect("daemon replaces the internal layer");

    let (second, _second_handle) = process_subscriber(LevelFilter::INFO);
    assert!(
        tracing::subscriber::set_global_default(second).is_err(),
        "an unrelated second global installation must remain an application error"
    );
}
