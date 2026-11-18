use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::AppError;

const APPLICATION_DIRECTORY: &str = "Presentationpoint";
const RECENT_LIMIT: usize = 20;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub ui_scale_percent: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ui_scale_percent: 100,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct RecentList {
    paths: Vec<PathBuf>,
}

/// Owns all local, non-roaming application files.
pub struct AppStorage {
    root: PathBuf,
    settings_path: PathBuf,
    recent_path: PathBuf,
}

impl AppStorage {
    pub fn local() -> Result<Self, AppError> {
        let local_app_data = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or(AppError::LocalAppDataUnavailable)?;
        Self::at(local_app_data.join(APPLICATION_DIRECTORY))
    }

    pub fn at(root: impl Into<PathBuf>) -> Result<Self, AppError> {
        let root = root.into();
        for directory in [
            root.clone(),
            root.join("autorecover"),
            root.join("snapshots"),
        ] {
            fs::create_dir_all(&directory).map_err(|source| AppError::io(&directory, source))?;
        }
        Ok(Self {
            settings_path: root.join("settings.json"),
            recent_path: root.join("recent.json"),
            root,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn settings(&self) -> Result<Settings, AppError> {
        if !self.settings_path.exists() {
            return Ok(Settings::default());
        }
        let bytes = fs::read(&self.settings_path)
            .map_err(|source| AppError::io(&self.settings_path, source))?;
        serde_json::from_slice(&bytes).map_err(|source| AppError::SettingsJson {
            path: self.settings_path.clone(),
            source,
        })
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), AppError> {
        let bytes =
            serde_json::to_vec_pretty(settings).map_err(|source| AppError::SerializeJson {
                path: self.settings_path.clone(),
                source,
            })?;
        write_bytes_atomically(&self.settings_path, &bytes)
    }

    /// Loads recent files, removes missing paths, and persists the pruned list.
    pub fn recent_files(&self) -> Result<Vec<PathBuf>, AppError> {
        if !self.recent_path.exists() {
            self.write_recent(&RecentList::default())?;
            return Ok(Vec::new());
        }
        let bytes = fs::read(&self.recent_path)
            .map_err(|source| AppError::io(&self.recent_path, source))?;
        let mut recent: RecentList =
            serde_json::from_slice(&bytes).map_err(|source| AppError::SettingsJson {
                path: self.recent_path.clone(),
                source,
            })?;
        let old_length = recent.paths.len();
        recent.paths.retain(|path| path.is_file());
        if recent.paths.len() != old_length {
            self.write_recent(&recent)?;
        }
        Ok(recent.paths)
    }

    pub fn add_recent(&self, path: &Path) -> Result<Vec<PathBuf>, AppError> {
        let absolute = path
            .canonicalize()
            .map_err(|source| AppError::io(path, source))?;
        let mut recent = if self.recent_path.exists() {
            let bytes = fs::read(&self.recent_path)
                .map_err(|source| AppError::io(&self.recent_path, source))?;
            serde_json::from_slice::<RecentList>(&bytes).map_err(|source| {
                AppError::SettingsJson {
                    path: self.recent_path.clone(),
                    source,
                }
            })?
        } else {
            RecentList::default()
        };
        recent
            .paths
            .retain(|existing| existing != &absolute && existing.is_file());
        recent.paths.insert(0, absolute);
        recent.paths.truncate(RECENT_LIMIT);
        self.write_recent(&recent)?;
        Ok(recent.paths)
    }

    fn write_recent(&self, recent: &RecentList) -> Result<(), AppError> {
        let bytes =
            serde_json::to_vec_pretty(recent).map_err(|source| AppError::SerializeJson {
                path: self.recent_path.clone(),
                source,
            })?;
        write_bytes_atomically(&self.recent_path, &bytes)
    }
}

fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| AppError::CommandLine("settings path has no filename".to_owned()))?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = name.to_os_string();
    temp_name.push(format!(".{}.{}.tmp", std::process::id(), sequence));
    let temp_path = parent.join(temp_name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|source| AppError::io(&temp_path, source))?;
    file.write_all(bytes)
        .map_err(|source| AppError::io(&temp_path, source))?;
    file.sync_all()
        .map_err(|source| AppError::io(&temp_path, source))?;
    drop(file);
    fs::rename(&temp_path, path).map_err(|source| AppError::io(path, source))?;
    Ok(())
}
