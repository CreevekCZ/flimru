use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedDevice {
    pub id: String,
    pub name: String,
    pub platform: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub favorites: Vec<SavedDevice>,
    #[serde(default)]
    pub last_device: Option<String>,
    #[serde(default)]
    pub last_config: Option<String>,
}

pub fn default_path() -> Result<PathBuf> {
    // Keep the storage identity stable across the rename to Flimru so existing
    // favorites and last selections continue to load without copying user data.
    let project_dirs = ProjectDirs::from("dev", "flutter-run-tui", "flutter-run-tui")
        .context("could not determine the platform config directory")?;
    Ok(project_dirs.config_dir().join("state.json"))
}

pub fn load(path: &Path) -> Result<State> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read state file {}", path.display()));
        }
    };
    serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse state file {}", path.display()))
}

pub fn save(path: &Path, state: &State) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create state directory {}", parent.display()))?;

    let mut temporary = NamedTempFile::new_in(parent).with_context(|| {
        format!(
            "failed to create temporary state file in {}",
            parent.display()
        )
    })?;
    serde_json::to_writer_pretty(&mut temporary, state).context("failed to serialize state")?;
    temporary
        .write_all(b"\n")
        .context("failed to finish serializing state")?;
    temporary.flush().context("failed to flush state file")?;
    temporary
        .as_file()
        .sync_all()
        .context("failed to sync temporary state file")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o600))
            .context("failed to set state file permissions")?;
    }

    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace state file {}", path.display()))?;
    Ok(())
}

impl State {
    pub fn is_favorite(&self, id: &str) -> bool {
        self.favorites.iter().any(|device| device.id == id)
    }

    pub fn toggle_favorite(&mut self, device: SavedDevice) -> bool {
        if let Some(index) = self
            .favorites
            .iter()
            .position(|saved_device| saved_device.id == device.id)
        {
            self.favorites.remove(index);
            false
        } else {
            self.favorites.push(device);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn device(id: &str) -> SavedDevice {
        SavedDevice {
            id: id.to_owned(),
            name: format!("Device {id}"),
            platform: "ios".to_owned(),
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("state.json");
        let expected = State {
            favorites: vec![device("simulator-1")],
            last_device: Some("simulator-1".to_owned()),
            last_config: Some("debug".to_owned()),
        };

        save(&path, &expected).unwrap();

        assert_eq!(load(&path).unwrap().favorites, expected.favorites);
        assert_eq!(load(&path).unwrap().last_device, expected.last_device);
        assert_eq!(load(&path).unwrap().last_config, expected.last_config);
    }

    #[test]
    fn missing_file_loads_default() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("missing.json");

        assert_eq!(load(&path).unwrap().favorites, Vec::new());
        assert!(load(&path).unwrap().last_device.is_none());
        assert!(load(&path).unwrap().last_config.is_none());
    }

    #[test]
    fn corrupt_file_returns_error_and_is_preserved() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("state.json");
        let corrupt = b"{ not valid json";
        fs::write(&path, corrupt).unwrap();

        let error = load(&path).unwrap_err();

        assert!(error.to_string().contains("failed to parse state file"));
        assert_eq!(fs::read(&path).unwrap(), corrupt);
    }

    #[test]
    fn favorite_toggling_is_keyed_by_id() {
        let mut state = State::default();
        let first = device("simulator-1");
        let replacement = SavedDevice {
            name: "Renamed".to_owned(),
            platform: "android".to_owned(),
            ..first.clone()
        };

        assert!(state.toggle_favorite(first));
        assert!(state.is_favorite("simulator-1"));
        assert!(!state.toggle_favorite(replacement));
        assert!(!state.is_favorite("simulator-1"));
        assert!(state.favorites.is_empty());
    }

    #[test]
    fn missing_fields_use_defaults() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("state.json");
        fs::write(&path, "{}").unwrap();

        let state = load(&path).unwrap();
        assert!(state.favorites.is_empty());
        assert!(state.last_device.is_none());
        assert!(state.last_config.is_none());
    }
}
