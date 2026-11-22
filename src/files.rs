//! Private, atomic plugin state. No credential is written to a log.
use serde::Serialize;
use std::{io::Write, path::Path};

pub fn save(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid data directory")?;
    std::fs::create_dir_all(parent).map_err(|_| "Cannot create plugin data directory")?;
    let mut file =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| "Cannot create private state file")?;
    serde_json::to_writer(&mut file, value).map_err(|_| "Cannot encode plugin state")?;
    file.flush().map_err(|_| "Cannot write plugin state")?;
    file.as_file()
        .sync_all()
        .map_err(|_| "Cannot sync plugin state")?;
    file.persist(path).map_err(|_| "Cannot save plugin state")?;
    Ok(())
}

pub fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "Plugin state is invalid".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Cannot read plugin state".into()),
    }
}
