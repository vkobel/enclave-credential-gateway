use anyhow::{bail, Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path};

pub fn validate_path_component(component: &str, label: &str) -> Result<()> {
    let mut components = Path::new(component).components();
    let first = components.next();
    if component.is_empty()
        || component == "."
        || component == ".."
        || component.contains('/')
        || component.contains('\\')
        || components.next().is_some()
        || !matches!(first, Some(Component::Normal(path_component)) if path_component == component)
    {
        bail!("{label} must be a single safe path component");
    }
    Ok(())
}

pub fn write_secret_file(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
        set_owner_only_dir(parent)?;
    }

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    configure_secret_file_options(&mut options);
    let mut file = options
        .open(path)
        .with_context(|| format!("Failed to open {}", path.display()))?;
    set_owner_only_file(path)?;
    file.write_all(contents.as_ref())
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn configure_secret_file_options(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn configure_secret_file_options(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn set_owner_only_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("Failed to set private permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_owner_only_dir(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("Failed to set private permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_owner_only_file(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_path_component, write_secret_file};

    #[test]
    fn rejects_path_components_that_escape() {
        for value in ["", ".", "..", "../x", "x/y", r"x\y"] {
            assert!(validate_path_component(value, "token name").is_err());
        }
        validate_path_component("laptop token", "token name").unwrap();
        validate_path_component("laptop-1", "token name").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn writes_secret_file_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested/config.toml");
        write_secret_file(&path, "secret").unwrap();

        let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
