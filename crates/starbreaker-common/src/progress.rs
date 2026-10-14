//! Shared progress reporting for long-running operations.
//!
//! [`Progress`] is a cheap, cloneable handle onto shared state: an
//! `Arc<Inner>` holding the reported fraction (an `AtomicU32` in 0.01%
//! units, only ever raised) and the current stage message, plus the
//! `base`/`range` slice of the root's 0..1 range this handle writes into.
//! [`Progress::new`] creates a root that the UI polls with [`Progress::get`];
//! [`Progress::silent`] creates a no-op handle for callers that don't want
//! reporting. [`Progress::sub`] returns a child that shares the parent's
//! `Arc` and maps its own 0..1 range onto a slice of the parent's, so a
//! child can outlive its parent handle or move to another thread without any
//! `unsafe`. Children of a silent handle are silent. Heavy functions take
//! `&Progress`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use parking_lot::Mutex;

/// Units stored in [`Inner::fraction`] for a fraction of 1.0 (0.01% steps).
const UNITS_PER_ONE: f32 = 10000.0;

struct Inner {
    /// 0–10000 for 0.00%–100.00% of the root progress.
    fraction: AtomicU32,
    stage: Mutex<String>,
}

/// Thread-safe progress handle. Clones and [`sub`](Progress::sub) children
/// share the same underlying state.
#[derive(Clone)]
pub struct Progress {
    /// `None` for a silent handle: every report is a no-op.
    inner: Option<Arc<Inner>>,
    /// Where this handle's 0.0 lands on the root's 0..1 range.
    base: f32,
    /// How much of the root's 0..1 range this handle's 0..1 spans.
    range: f32,
}

impl Progress {
    /// A root progress the UI can poll.
    pub fn new() -> Self {
        Self {
            inner: Some(Arc::new(Inner {
                fraction: AtomicU32::new(0),
                stage: Mutex::new(String::new()),
            })),
            base: 0.0,
            range: 1.0,
        }
    }

    /// A progress that discards every report.
    pub fn silent() -> Self {
        Self { inner: None, base: 0.0, range: 1.0 }
    }

    /// Update progress. `fraction` is 0.0–1.0 within this handle's slice.
    /// Reports never move the root backwards: a lower value is ignored,
    /// along with its message.
    pub fn report(&self, fraction: f32, message: &str) {
        let Some(inner) = &self.inner else { return };
        let absolute = self.base + fraction.clamp(0.0, 1.0) * self.range;
        let units = (absolute * UNITS_PER_ONE) as u32;
        let previous = inner.fraction.fetch_max(units, Ordering::Relaxed);
        if units >= previous {
            *inner.stage.lock() = message.to_string();
        }
    }

    /// Read the root progress: its fraction and current stage message.
    /// A silent progress always reads `(0.0, "")`.
    pub fn get(&self) -> (f32, String) {
        match &self.inner {
            Some(inner) => {
                let fraction = inner.fraction.load(Ordering::Relaxed) as f32 / UNITS_PER_ONE;
                (fraction, inner.stage.lock().clone())
            }
            None => (0.0, String::new()),
        }
    }

    /// Whether the root progress has reached 100%.
    pub fn is_done(&self) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|inner| inner.fraction.load(Ordering::Relaxed) >= UNITS_PER_ONE as u32)
    }

    /// Create a child that maps its 0.0–1.0 to `from..to` on this progress.
    /// The child shares this progress's state, so it stays valid however long
    /// it lives; children of a silent progress are silent.
    pub fn sub(&self, from: f32, to: f32) -> Progress {
        Progress {
            inner: self.inner.clone(),
            base: self.base + from * self.range,
            range: (to - from) * self.range,
        }
    }
}

impl Default for Progress {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::Progress;

    #[test]
    fn progress_reports_are_monotonic() {
        let progress = Progress::new();
        progress.report(0.8, "later phase");
        progress.report(0.4, "older phase");

        let (fraction, stage) = progress.get();
        assert_eq!(fraction, 0.8);
        assert_eq!(stage, "later phase");
    }

    #[test]
    fn subprogress_reports_are_monotonic_on_parent() {
        let progress = Progress::new();
        progress.report(0.6, "parent phase");
        let sub = progress.sub(0.1, 0.2);
        sub.report(0.5, "sub phase");

        let (fraction, stage) = progress.get();
        assert_eq!(fraction, 0.6);
        assert_eq!(stage, "parent phase");
    }

    #[test]
    fn subprogress_maps_fraction_between_endpoints() {
        let progress = Progress::new();
        let sub = progress.sub(0.2, 0.6);

        sub.report(0.5, "sub phase");

        let (fraction, stage) = progress.get();
        assert!((fraction - 0.4).abs() < 0.0001);
        assert_eq!(stage, "sub phase");
    }

    #[test]
    fn nested_subprogress_reports_reach_root_progress() {
        let progress = Progress::new();
        let assembly = progress.sub(0.06, 0.90);
        let decomposed = assembly.sub(0.0, 0.90);

        decomposed.report(0.5, "Writing child assets");

        let (fraction, stage) = progress.get();
        assert!((fraction - 0.438).abs() < 0.0001);
        assert_eq!(stage, "Writing child assets");
    }

    #[test]
    fn subprogress_outliving_its_parent_still_reaches_root() {
        let root = Progress::new();
        let child = {
            let parent = root.sub(0.5, 1.0);
            parent.sub(0.0, 0.5)
        };

        std::thread::spawn(move || child.report(1.0, "child on another thread"))
            .join()
            .unwrap();

        let (fraction, stage) = root.get();
        assert!((fraction - 0.75).abs() < 0.0001);
        assert_eq!(stage, "child on another thread");
    }

    #[test]
    fn silent_progress_and_its_children_report_nothing() {
        let silent = Progress::silent();
        let child = silent.sub(0.0, 0.5);

        silent.report(0.5, "ignored");
        child.report(1.0, "ignored");

        assert_eq!(silent.get(), (0.0, String::new()));
        assert_eq!(child.get(), (0.0, String::new()));
        assert!(!child.is_done());
    }
}
