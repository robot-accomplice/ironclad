use std::ffi::OsString;
use std::sync::Mutex;

static ENV_MUTEX: Mutex<()> = Mutex::new(());

/// RAII guard that sets an env var and restores the old value on drop.
/// The global ENV_MUTEX is acquired during set() and drop() but NOT held
/// across the test body, avoiding await_holding_lock issues in async tests.
pub struct EnvGuard {
    key: &'static str,
    old: Option<OsString>,
}

impl EnvGuard {
    pub fn set(key: &'static str, value: &str) -> Self {
        let _lock = ENV_MUTEX.lock().expect("env mutex poisoned");
        let old = std::env::var_os(key);
        // SAFETY: serialized via ENV_MUTEX; only one thread mutates env at a time.
        unsafe { std::env::set_var(key, value) };
        // Lock is dropped here — not held across the test body.
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        let _lock = ENV_MUTEX.lock().expect("env mutex poisoned");
        if let Some(v) = &self.old {
            unsafe { std::env::set_var(self.key, v) };
        } else {
            unsafe { std::env::remove_var(self.key) };
        }
    }
}
