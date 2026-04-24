use std::ffi::OsString;
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

pub fn with_temp_home<T>(f: impl FnOnce(&TempDir) -> T) -> T {
    static HOME_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = HOME_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = TempDir::new().unwrap();
    let _home = HomeRestore::set(temp.path().as_os_str().to_os_string());

    f(&temp)
}

struct HomeRestore {
    old_home: Option<OsString>,
}

impl HomeRestore {
    fn set(home: OsString) -> Self {
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", home);
        Self { old_home }
    }
}

impl Drop for HomeRestore {
    fn drop(&mut self) {
        match &self.old_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }
}
