use std::fs::{create_dir_all, File};
use std::io::Result;
use std::path::{Path, PathBuf};
use std::{env, fs, io};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeLock {
    pub pid: Option<u32>,
    pub start_time: Option<SystemTime>,
}

impl RuntimeLock {
    pub fn empty() -> Self {
        Self { pid: None, start_time: None }
    }

    pub fn default_path() -> PathBuf {
        Self::get_dura_cache_home().join("runtime.db")
    }

    /// Location of all database files. By default
    ///
    /// Linux   :   $XDG_CACHE_HOME/dura or $HOME/.cache/dura
    /// macOS   :   $HOME/Library/Caches
    /// Windows :   %AppData%\Local\dura
    ///
    /// This can be overridden by setting DURA_CACHE_HOME environment variable.
    fn get_dura_cache_home() -> PathBuf {
        // The environment variable lets us run tests independently, but I'm sure someone will come
        // up with another reason to use it.
        if let Ok(env_var) = env::var("DURA_CACHE_HOME") {
            if !env_var.is_empty() {
                return env_var.into();
            }
        }

        dirs::cache_dir()
            .expect("Could not find your cache directory. The default is ~/.cache/dura but it can also \
                be controlled by setting the DURA_CACHE_HOME environment variable.")
            .join("dura")
    }

    /// Load Config from default path
    pub fn load() -> Self {
        Self::load_file(Self::default_path().as_path()).unwrap_or_else(|_| Self::empty())
    }

    pub fn load_file(path: &Path) -> Result<Self> {
        let reader = io::BufReader::new(File::open(path)?);
        let res = serde_json::from_reader(reader)?;
        Ok(res)
    }

    /// Save config to disk in ~/.cache/dura/runtime.db
    pub fn save(&self) {
        self.save_to_path(Self::default_path().as_path())
    }

    pub fn create_dir(path: &Path) {
        if let Some(dir) = path.parent() {
            create_dir_all(dir).unwrap_or_else(|_| {
                panic!(
                    "Failed to create directory at `{}`.\
                    Dura stores its runtime cache in `{}/runtime.db`. \
                    See https://github.com/tkellogg/dura for more information.",
                    dir.display(),
                    path.display()
                )
            })
        }
    }

    /// Attempts to create parent dirs, serialize `self` as JSON and write to disk.
    pub fn save_to_path(&self, path: &Path) {
        Self::create_dir(path);

        let json = serde_json::to_string(self).unwrap();
        fs::write(path, json).unwrap()
    }
}
