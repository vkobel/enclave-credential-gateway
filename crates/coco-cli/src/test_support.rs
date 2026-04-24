use std::cell::RefCell;
use std::path::PathBuf;
use tempfile::TempDir;

thread_local! {
    static CONFIG_ROOT_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static HOME_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub fn with_temp_config_root<T>(f: impl FnOnce(&TempDir) -> T) -> T {
    let temp = TempDir::new().unwrap();
    let _config = OverrideRestore::set_config_root(temp.path().join(".config/coco"));
    let _home = OverrideRestore::set_home_dir(temp.path().to_path_buf());

    f(&temp)
}

pub(crate) fn config_root_override() -> Option<PathBuf> {
    CONFIG_ROOT_OVERRIDE.with(|value| value.borrow().clone())
}

pub(crate) fn home_dir_override() -> Option<PathBuf> {
    HOME_DIR_OVERRIDE.with(|value| value.borrow().clone())
}

enum OverrideKind {
    ConfigRoot,
    HomeDir,
}

struct OverrideRestore {
    kind: OverrideKind,
    old_value: Option<PathBuf>,
}

impl OverrideRestore {
    fn set_config_root(path: PathBuf) -> Self {
        let old_value = CONFIG_ROOT_OVERRIDE.with(|value| value.replace(Some(path)));
        Self {
            kind: OverrideKind::ConfigRoot,
            old_value,
        }
    }

    fn set_home_dir(path: PathBuf) -> Self {
        let old_value = HOME_DIR_OVERRIDE.with(|value| value.replace(Some(path)));
        Self {
            kind: OverrideKind::HomeDir,
            old_value,
        }
    }
}

impl Drop for OverrideRestore {
    fn drop(&mut self) {
        let old_value = self.old_value.clone();
        match self.kind {
            OverrideKind::ConfigRoot => CONFIG_ROOT_OVERRIDE.with(|value| value.replace(old_value)),
            OverrideKind::HomeDir => HOME_DIR_OVERRIDE.with(|value| value.replace(old_value)),
        };
    }
}
