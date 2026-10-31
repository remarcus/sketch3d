//! Rebuildable index of recently saved or opened native drawings.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_RECENT: usize = 40;
const MAX_INDEX_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    #[serde(default)]
    recent: Vec<PathBuf>,
}

pub struct DrawingLibrary {
    path: Option<PathBuf>,
    recent: Vec<PathBuf>,
    pub notice: Option<String>,
}

impl DrawingLibrary {
    pub fn new() -> Self {
        let path = match crate::presets::path() {
            Ok(path) => path.with_file_name("drawing-library.json"),
            Err(error) => {
                return Self {
                    path: None,
                    recent: Vec::new(),
                    notice: Some(format!("Drawing library: {error}")),
                };
            }
        };
        let mut library = Self {
            path: Some(path),
            recent: Vec::new(),
            notice: None,
        };
        match Self::load(library.path.as_deref().unwrap()) {
            Ok(recent) => library.recent = recent,
            Err(error) => library.notice = Some(format!("Drawing library: {error}")),
        }
        library
    }

    fn load(path: &Path) -> Result<Vec<PathBuf>, String> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        if file.metadata().map_err(|error| error.to_string())?.len() > MAX_INDEX_BYTES {
            return Err("index exceeds its size limit".into());
        }
        let index: Index = serde_json::from_reader(file).map_err(|error| error.to_string())?;
        if index.recent.len() > MAX_RECENT
            || index.recent.iter().any(|path| path.as_os_str().is_empty())
        {
            return Err("index contains invalid recent drawing paths".into());
        }
        let mut unique = Vec::new();
        for path in index.recent {
            if !unique.contains(&path) {
                unique.push(path);
            }
        }
        Ok(unique)
    }

    pub fn recent(&self) -> &[PathBuf] {
        &self.recent
    }

    pub fn record(&mut self, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.recent.retain(|recent| recent != &path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
        if let Some(index_path) = &self.path {
            let result = (|| -> Result<(), String> {
                let parent = index_path
                    .parent()
                    .ok_or("Invalid library index location")?;
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                let mut temp =
                    tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
                serde_json::to_writer_pretty(
                    &mut temp,
                    &Index {
                        recent: self.recent.clone(),
                    },
                )
                .map_err(|error| error.to_string())?;
                temp.as_file()
                    .sync_all()
                    .map_err(|error| error.to_string())?;
                temp.persist(index_path)
                    .map_err(|error| error.to_string())?;
                Ok(())
            })();
            self.notice = result
                .err()
                .map(|error| format!("Cannot save drawing library: {error}"));
        }
    }
}
