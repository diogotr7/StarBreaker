//! Shared progress reporting for long-running operations.
//!
//! The UI creates a root [`Progress`] and polls it with [`Progress::get`];
//! heavy functions take `Option<&Progress>` and report through it.
//! Opt-in checked reports call a borrowed observer and return `Cancelled` when
//! it requests a stop, including through nested sub-progress ranges.
//! [`Progress::sub`] returns a child that borrows its parent and maps its own
//! 0..1 range onto a slice of the parent's. The borrow is the `'a` lifetime
//! parameter, so the compiler rejects a child that would outlive its parent:
//!
//! ```compile_fail,E0597
//! use starbreaker_common::Progress;
//!
//! let child = {
//!     let parent = Progress::new();
//!     parent.sub(0.0, 1.0)
//! };
//! child.report(0.5, "parent already dropped");
//! ```

use std::sync::atomic::{AtomicU32, Ordering};
use parking_lot::Mutex;

/// Cooperative stop requested by a checked progress observer.
#[derive(Debug, thiserror::Error)]
#[error("operation cancelled")]
pub struct Cancelled;

/// Thread-safe progress state. Pass `Option<&Progress>` to heavy functions.
/// `None` means no reporting (zero cost). The UI creates one and polls it.
pub struct Progress<'a> {
    /// 0–10000 for 0.00%–100.00%
    value: AtomicU32,
    stage: Mutex<String>,
    /// If set, this progress writes scaled values into a parent.
    parent: Option<(&'a Progress<'a>, f32, f32)>,  // (parent, base, range)
    observer: Option<&'a (dyn Fn(f32, &str) -> bool + Sync)>,
}

impl<'a> Progress<'a> {
    pub fn new() -> Self {
        Self {
            value: AtomicU32::new(0),
            stage: Mutex::new(String::new()),
            parent: None,
            observer: None,
        }
    }

    /// Observe checked reports synchronously. False requests cancellation.
    /// The observer may run on the reporting worker; it must not block or panic.
    pub fn with_callback(observer: &'a (dyn Fn(f32, &str) -> bool + Sync)) -> Self {
        Self { observer: Some(observer), ..Self::new() }
    }

    /// Report a phase boundary and stop before starting its work when requested.
    pub fn report_checked(&self, fraction: f32, message: &str) -> Result<(), Cancelled> {
        let fraction = fraction.clamp(0.0, 1.0);
        if let Some((parent, base, range)) = self.parent {
            return parent.report_checked(base + fraction * range, message);
        }
        self.report(fraction, message);
        if self.observer.is_some_and(|callback| !callback(fraction, message)) {
            return Err(Cancelled);
        }
        Ok(())
    }

    /// Update progress. `fraction` is 0.0–1.0.
    pub fn report(&self, fraction: f32, message: &str) {
        let clamped = fraction.clamp(0.0, 1.0);

        if let Some((parent, base, range)) = self.parent {
            parent.report(base + clamped * range, message);
        } else {
            let units = (clamped * 10000.0) as u32;
            let previous = self.value.fetch_max(units, Ordering::Relaxed);
            if units >= previous {
                *self.stage.lock() = message.to_string();
            }
        }
    }

    /// Read current progress.
    pub fn get(&self) -> (f32, String) {
        let v = self.value.load(Ordering::Relaxed) as f32 / 10000.0;
        let s = self.stage.lock().clone();
        (v, s)
    }

    pub fn is_done(&self) -> bool {
        self.value.load(Ordering::Relaxed) >= 10000
    }

    /// Create a sub-progress that maps 0.0–1.0 to `from..to` on this progress.
    /// The returned `Progress` can be passed anywhere `&Progress` is accepted,
    /// but borrows `self`, so it cannot outlive this progress.
    pub fn sub(&self, from: f32, to: f32) -> Progress<'_> {
        Progress {
            value: AtomicU32::new(0),
            stage: Mutex::new(String::new()),
            parent: Some((self, from, to - from)),
            observer: None,
        }
    }
}

impl Default for Progress<'_> {
    fn default() -> Self { Self::new() }
}

/// Helper: report on an `Option<&Progress>`, noop if None.
pub fn report(progress: Option<&Progress>, fraction: f32, message: &str) {
    if let Some(p) = progress {
        p.report(fraction, message);
    }
}

/// Checked counterpart to `report`; callers propagate the stop with `?`.
pub fn report_checked(progress: Option<&Progress>, fraction: f32, message: &str) -> Result<(), Cancelled> {
    progress.map_or(Ok(()), |p| p.report_checked(fraction, message))
}

#[cfg(test)]
mod tests {
    use super::Progress;

    #[test]
    fn checked_subprogress_propagates_worker_cancellation() {
        let callback = |fraction: f32, phase: &str| {
            assert!((fraction - 0.4).abs() < 0.0001);
            assert_eq!(phase, "worker");
            false
        };
        let root = Progress::with_callback(&callback);
        let sub = root.sub(0.2, 0.6);
        std::thread::scope(|scope| {
            assert!(scope.spawn(|| sub.report_checked(0.5, "worker")).join().unwrap().is_err());
        });
        assert!(Progress::new().report_checked(0.5, "ordinary").is_ok());
        assert!(super::report_checked(None, 0.5, "absent").is_ok());
    }

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
    fn subprogress_reports_from_scoped_threads() {
        let progress = Progress::new();
        let sub = progress.sub(0.5, 1.0);

        std::thread::scope(|scope| {
            scope.spawn(|| sub.report(0.5, "worker thread"));
        });

        let (fraction, stage) = progress.get();
        assert!((fraction - 0.75).abs() < 0.0001);
        assert_eq!(stage, "worker thread");
    }
}
