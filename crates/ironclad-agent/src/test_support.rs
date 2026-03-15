use std::ffi::OsString;
use std::sync::Mutex;

static ENV_MUTEX: Mutex<()> = Mutex::new(());

/// RAII guard that sets an env var and restores the old value on drop.
pub struct EnvGuard {
    key: &'static str,
    old: Option<OsString>,
}

impl EnvGuard {
    pub fn set(key: &'static str, value: &str) -> Self {
        let _lock = ENV_MUTEX.lock().expect("env mutex poisoned");
        let old = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
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
