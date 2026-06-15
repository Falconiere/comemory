//! Tests for the RAII restore primitive behind the terminal guard.
//!
//! The real `TerminalGuard` needs a tty, but its restore guarantee rests on
//! [`Restore`], which is tty-free: it must run its teardown closure exactly
//! once on every scope exit — including an early error return (criterion C9).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use comemory::tui::terminal::Restore;

#[test]
fn restore_runs_on_normal_scope_exit() {
    let flag = Arc::new(AtomicBool::new(false));
    {
        let f = flag.clone();
        let _g = Restore::new(move || f.store(true, Ordering::SeqCst));
    }
    assert!(
        flag.load(Ordering::SeqCst),
        "restore must run on scope exit"
    );
}

#[test]
fn restore_runs_on_early_error_return() {
    // A function that builds the guard then returns Err early: Rust runs Drop
    // on every scope exit, including the `?`/early-return path.
    fn faulty(flag: &Arc<AtomicBool>) -> Result<(), &'static str> {
        let f = flag.clone();
        let _g = Restore::new(move || f.store(true, Ordering::SeqCst));
        Err("boom")?;
        Ok(())
    }
    let flag = Arc::new(AtomicBool::new(false));
    let _ = faulty(&flag);
    assert!(
        flag.load(Ordering::SeqCst),
        "restore must run on the error path"
    );
}

#[test]
fn forget_defuses_restore() {
    // The staged rollback in `TerminalGuard::enter` `mem::forget`s its
    // per-step guards on success so the constructed guard owns restoration.
    // Forgetting a `Restore` must skip its teardown.
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    let g = Restore::new(move || f.store(true, Ordering::SeqCst));
    std::mem::forget(g);
    assert!(
        !flag.load(Ordering::SeqCst),
        "forget must skip the restore closure"
    );
}

#[test]
fn restore_runs_exactly_once() {
    let count = Arc::new(AtomicUsize::new(0));
    {
        let c = count.clone();
        let _g = Restore::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
