//! Classification never invokes a host-installed global diagnostic callback.

use std::sync::atomic::{AtomicUsize, Ordering};
use tinymemory_api::safety::scan;

struct Counter;
static CALLS: AtomicUsize = AtomicUsize::new(0);
static LOGGER: Counter = Counter;

impl log::Log for Counter {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool { true }
    fn log(&self, _: &log::Record<'_>) { CALLS.fetch_add(1, Ordering::SeqCst); }
    fn flush(&self) {}
}

#[test]
fn classification_does_not_invoke_host_logger_callbacks() {
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Trace);
    let _ = scan("patient diagnosed; SSN 123-45-6789");
    let _ = scan("ordinary text");
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
}
