use std::collections::BTreeMap;
use std::fs::{create_dir_all, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::{env, fs};
use std::time::SystemTime;
use chrono::{DateTime, Local};
use git2::Repository;

use serde::{Deserialize, Serialize};

use crate::git_repo_iter::GitRepoIter;
use crate::database::RuntimeLock;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct WatchConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub max_depth: u8,
}

impl WatchConfig {
    pub fn new() -> Self {
        Self {
            include: vec![],
            exclude: vec![],
            max_depth: 255,
        }
    }
}

impl Default for WatchConfig {
    fn default() -> Self {
        WatchConfig::new()
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    // When commit_exclude_git_config is true,
    // never use any git configuration to sign dura's commits.
    // Defaults to false
    #[serde(default)]
    pub commit_exclude_git_config: bool,
    pub commit_author: Option<String>,
    pub commit_email: Option<String>,
    pub repos: BTreeMap<String, Rc<WatchConfig>>,
}

impl Config {
    pub fn empty() -> Self {
        Self {
            commit_exclude_git_config: false,
            commit_author: None,
            commit_email: None,
            repos: BTreeMap::new(),
        }
    }

    pub fn default_path() -> PathBuf {
        Self::get_dura_config_home().join("config.toml")
    }

    /// Location of all config. By default
    ///
    /// Linux   :   $XDG_CONFIG_HOME/dura or $HOME/.config/dura
    /// macOS   :   $HOME/Library/Application Support
    /// Windows :   %AppData%\Roaming\dura
    ///
    /// This can be overridden by setting DURA_CONFIG_HOME environment variable.
    fn get_dura_config_home() -> PathBuf {
        // The environment variable lets us run tests independently, but I'm sure someone will come
        // up with another reason to use it.
        if let Ok(env_var) = env::var("DURA_CONFIG_HOME") {
            if !env_var.is_empty() {
                return env_var.into();
            }
        }

        dirs::config_dir()
            .expect("Could not find your config directory. The default is ~/.config/dura but it can also \
                be controlled by setting the DURA_CONFIG_HOME environment variable.")
            .join("dura")
    }

    /// Load Config from default path
    pub fn load() -> Self {
        Self::load_file(Self::default_path().as_path()).unwrap_or_else(|_| Self::empty())
    }

    pub fn load_file(path: &Path) -> Result<Self> {
        let mut reader = BufReader::new(File::open(path)?);

        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer)?;

        let res = toml::from_slice(buffer.as_slice())?;
        Ok(res)
    }

    /// Save config to disk in ~/.config/dura/config.toml
    pub fn save(&self) {
        self.save_to_path(Self::default_path().as_path())
    }

    pub fn create_dir(path: &Path) {
        if let Some(dir) = path.parent() {
            create_dir_all(dir)
                .unwrap_or_else(|_| panic!("Failed to create directory at `{}`.\
                    Dura stores its configuration in `{}/config.toml`, \
                    where you can instruct dura to watch patterns of Git repositories, among other things. \
                    See https://github.com/tkellogg/dura for more information.", dir.display(), path.display()))
        }
    }

    /// Attempts to create parent dirs, serialize `self` as TOML and write to disk.
    pub fn save_to_path(&self, path: &Path) {
        Self::create_dir(path);

        let config_string = match toml::to_string(self) {
            Ok(v) => v,
            Err(e) => {
                println!("Unexpected error when deserializing config: {e}");
                return;
            }
        };

        match fs::write(path, config_string) {
            Ok(_) => (),
            Err(e) => println!("Unable to initialize dura config file: {e}"),
        }
    }

    pub fn set_watch(&mut self, path: String, cfg: WatchConfig) {
        let abs_path = fs::canonicalize(path).expect("The provided path is not a directory");
        let abs_path = abs_path
            .to_str()
            .expect("The provided path is not valid unicode");

        if self.repos.contains_key(abs_path) {
            println!("{abs_path} is already being watched")
        } else {
            self.repos.insert(abs_path.to_string(), Rc::new(cfg));
            println!("Started watching {abs_path}")
        }
    }

    pub fn set_unwatch(&mut self, path: String) {
        let abs_path = fs::canonicalize(path).expect("The provided path is not a directory");
        let abs_path = abs_path
            .to_str()
            .expect("The provided path is not valid unicode")
            .to_string();

        match self.repos.remove(&abs_path) {
            Some(_) => {
                println!("Stopped watching {abs_path}");
            }
            None => println!("{abs_path} is not being watched"),
        }
    }

    pub fn git_repos(&self) -> GitRepoIter {
        GitRepoIter::new(self)
    }

    pub fn print_detailed_info(&self) {
        // Configuration File Info
        println!("Dura Configuration");
        println!("==================");
        println!("Config file: {}", Self::default_path().display());
        println!("Config directory: {}", Self::get_dura_config_home().display());
        
        // Server Status
        println!("\nServer Status");
        println!("-------------");
        let runtime_lock = RuntimeLock::load();
        match runtime_lock.pid {
            Some(pid) => {
                println!("Server is running (PID: {})", pid);
                if let Some(start_time) = runtime_lock.start_time {
                    if let Ok(duration) = SystemTime::now().duration_since(start_time) {
                        let days = duration.as_secs() / 86400;
                        let hours = (duration.as_secs() % 86400) / 3600;
                        let minutes = (duration.as_secs() % 3600) / 60;
                        let seconds = duration.as_secs() % 60;
                        println!("Running for: {}d {}h {}m {}s", days, hours, minutes, seconds);
                    }
                }
            },
            None => println!("Server is not running"),
        }
        
        // Commit Settings
        println!("\nCommit Settings");
        println!("--------------");
        println!("Exclude Git Config: {}", self.commit_exclude_git_config);
        println!("Author: {}", self.commit_author.as_deref().unwrap_or("Not set"));
        println!("Email: {}", self.commit_email.as_deref().unwrap_or("Not set"));
        
        // Repository Status
        println!("\nWatched Repositories");
        println!("-------------------");
        for (path, config) in &self.repos {
            self.print_repo_status(path, config);
        }
    }

    fn print_repo_status(&self, path: &str, config: &WatchConfig) {
        println!("\n📁 {}", path);
        
        // Check if directory exists
        let path = PathBuf::from(path);
        if !path.exists() {
            println!("  ⚠️  Directory not found!");
            return;
        }

        // Check if it's a git repository
        match Repository::open(&path) {
            Ok(repo) => {
                println!("  ✓ Valid Git repository");
                
                // Check for uncommitted changes with more specific options
                match repo.statuses(Some(git2::StatusOptions::new()
                    .include_untracked(true)
                    .include_ignored(false)
                    .include_unmodified(false)
                    .renames_head_to_index(true)
                    .recurse_untracked_dirs(true))) 
                {
                    Ok(statuses) => {
                        let mut has_changes = false;
                        for entry in statuses.iter() {
                            let status = entry.status();
                            if status.is_wt_new() || 
                               status.is_wt_modified() || 
                               status.is_wt_deleted() ||
                               status.is_index_new() ||
                               status.is_index_modified() ||
                               status.is_index_deleted() {
                                // Print what kind of change was detected for debugging
                                if let Some(path) = entry.path() {
                                    println!("  📝 Change detected: {} ({:?})", path, status);
                                }
                                has_changes = true;
                            }
                        }

                        if has_changes {
                            println!("  ⚠️  Has uncommitted changes");
                        } else {
                            println!("  ✓ No uncommitted changes");
                        }
                    }
                    Err(e) => println!("  ⚠️  Unable to check repository status: {}", e),
                }

                // Check for dura backup branches
                let mut latest_backup = None;
                let mut backup_count = 0;

                if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
                    for branch_result in branches {
                        if let Ok((branch, _)) = branch_result {
                            if let Ok(Some(name)) = branch.name() {
                                if name.starts_with("dura/") {
                                    backup_count += 1;
                                    if let Ok(commit) = branch.get().peel_to_commit() {
                                        match latest_backup {
                                            None => latest_backup = Some(commit),
                                            Some(ref latest) => {
                                                if commit.time().seconds() > latest.time().seconds() {
                                                    latest_backup = Some(commit);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                match latest_backup {
                    Some(commit) => {
                        let time = SystemTime::UNIX_EPOCH + 
                                 std::time::Duration::from_secs(commit.time().seconds() as u64);
                        let datetime: DateTime<Local> = time.into();
                        println!("  🕒 Last backup: {} ({})", 
                               datetime.format("%Y-%m-%d %H:%M:%S"),
                               commit.id());
                        println!("  📊 Total backups: {}", backup_count);
                    }
                    None => println!("  ℹ️ No dura backups found"),
                }
            }
            Err(e) => {
                println!("  ⚠️  Not a Git repository: {}", e);
                return;
            }
        }

        // Print watch configuration
        println!("  Watch Configuration:");
        if config.include.is_empty() {
            println!("    Include: All files");
        } else {
            println!("    Include patterns: {:?}", config.include);
        }
        if !config.exclude.is_empty() {
            println!("    Exclude patterns: {:?}", config.exclude);
        }
        println!("    Max depth: {}", config.max_depth);
    }

    pub fn print_summary(&self) {
        // Print overall status
        let total_repos = self.repos.len();
        let mut total_backups = 0;
        let mut repos_with_changes = 0;
        let mut inaccessible_repos = 0;

        println!("Dura Status Summary");
        println!("-----------------");
        
        for (path, _) in &self.repos {
            let path = PathBuf::from(path);
            if !path.exists() {
                inaccessible_repos += 1;
                println!("❌ {}: Not found", path.display());
                continue;
            }

            match Repository::open(&path) {
                Ok(repo) => {
                    let mut status_str = String::new();
                    
                    // Check for changes
                    let has_changes = repo.statuses(Some(git2::StatusOptions::new()
                        .include_untracked(true)
                        .include_ignored(false)
                        .include_unmodified(false)))
                        .map(|statuses| !statuses.is_empty())
                        .unwrap_or(false);
                    
                    if has_changes {
                        repos_with_changes += 1;
                        status_str.push_str("📝 ");
                    }

                    // Get latest backup commit and count
                    let mut backup_count = 0;
                    let mut latest_commit_id = None;
                    if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
                        let mut latest_time = 0;
                        for branch_result in branches {
                            if let Ok((branch, _)) = branch_result {
                                if let Ok(Some(name)) = branch.name() {
                                    if name.starts_with("dura/") {
                                        backup_count += 1;
                                        if let Ok(commit) = branch.get().peel_to_commit() {
                                            let commit_time = commit.time().seconds();
                                            if commit_time > latest_time {
                                                latest_time = commit_time;
                                                latest_commit_id = Some(commit.id().to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    total_backups += backup_count;
                    
                    // Print repo summary with short commit id if available
                    let commit_info = latest_commit_id
                        .map(|id| format!(" [{}]", &id[..7]))
                        .unwrap_or_default();
                    
                    println!("{}{}: {} backups{}{}", 
                        if has_changes { "📝" } else { "✓" },
                        path.display(),
                        backup_count,
                        commit_info,
                        if has_changes { " (uncommitted changes)" } else { "" }
                    );
                }
                Err(_) => {
                    inaccessible_repos += 1;
                    println!("❌ {}: Not a git repository", path.display());
                }
            }
        }

        // Print overall summary
        println!("\nOverall Status:");
        println!("Watching {} repositories ({} accessible)", 
                total_repos, 
                total_repos - inaccessible_repos);
        println!("Total backups: {}", total_backups);
        if repos_with_changes > 0 {
            println!("Repositories with uncommitted changes: {}", repos_with_changes);
        }
        if inaccessible_repos > 0 {
            println!("Inaccessible repositories: {}", inaccessible_repos);
        }
    }
}
