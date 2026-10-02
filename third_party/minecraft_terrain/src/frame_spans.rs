//! Named CPU spans within a frame, for the frame profile
//! (`MINECRAFTOSS_PROFILE_FRAMES`). Recording is a thread-local push when
//! profiling is on and nothing otherwise.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static SPANS: RefCell<Vec<(&'static str, f64)>> = const { RefCell::new(Vec::new()) };
}

pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Times the rest of the enclosing scope under `name`.
pub struct Span {
    name: &'static str,
    started: Option<Instant>,
}

pub fn span(name: &'static str) -> Span {
    Span { name, started: ENABLED.load(Ordering::Relaxed).then(Instant::now) }
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            SPANS.with(|s| s.borrow_mut().push((self.name, ms)));
        }
    }
}

/// Records a duration measured elsewhere.
pub fn record(name: &'static str, ms: f64) {
    if ENABLED.load(Ordering::Relaxed) {
        SPANS.with(|s| s.borrow_mut().push((name, ms)));
    }
}

/// This thread's spans since the last call, summed by name.
pub fn take() -> Vec<(&'static str, f64)> {
    SPANS.with(|s| {
        let mut out: Vec<(&'static str, f64)> = Vec::new();
        for (name, ms) in s.borrow_mut().drain(..) {
            match out.iter_mut().find(|(n, _)| *n == name) {
                Some(entry) => entry.1 += ms,
                None => out.push((name, ms)),
            }
        }
        out
    })
}
