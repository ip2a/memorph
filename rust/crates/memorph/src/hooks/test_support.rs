//! Test-only support for hook provider tests.

use std::sync::{Mutex, MutexGuard, OnceLock};

pub fn test_runtime_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// Serializes tests that mutate the global hook home override and restores it on drop.
pub struct TestHookHomeGuard {
    _lock: MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
}

impl TestHookHomeGuard {
    pub fn new() -> Self {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let lock = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = tempfile::tempdir().expect("temp hook home");
        crate::hooks::shared::set_test_home_dir(Some(dir.path().to_path_buf()));
        Self {
            _lock: lock,
            _dir: dir,
        }
    }
}

impl Drop for TestHookHomeGuard {
    fn drop(&mut self) {
        crate::hooks::shared::set_test_home_dir(None);
    }
}
