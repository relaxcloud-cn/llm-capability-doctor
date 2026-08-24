use std::fs::{File, OpenOptions};
use std::path::Path;

#[cfg(unix)]
pub(crate) fn open_private_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

#[cfg(not(unix))]
pub(crate) fn open_private_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
}

#[cfg(unix)]
pub(crate) fn create_new_private_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

#[cfg(not(unix))]
pub(crate) fn create_new_private_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create_new(true).write(true).open(path)
}
