use std::collections::BTreeMap;
use std::fs::{create_dir_all, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::{env, fs};
use std::time::{SystemTime, Duration};
use chrono::{DateTime, Local};
use git2::Repository;
use std::io::IsTerminal;

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
    const SYMBOLS_FANCY: [&'static str; 8] = ["✓", "📝", "❌", "⚠️", "ℹ️", "🕒", "📊", "📁"];
    const SYMBOLS_PLAIN: [&'static str; 8] = ["[OK]", "[M]", "[X]", "!", "i", "@", "#", "*"];

    fn get_symbols() -> &'static [&'static str; 8] {
        // Check environment variable first (explicit override)
        if std::env::var("DURA_PLAIN_TEXT").is_ok() {
            return &Self::SYMBOLS_PLAIN;
        }
        
        // Check if DURA_FANCY is set (explicit override)
        if std::env::var("DURA_FANCY").is_ok() {
            return &Self::SYMBOLS_FANCY;
        }

        // Auto-detect terminal capabilities
        if !std::io::stdout().is_terminal() {
            // Not a terminal (e.g., pipe or redirect)
            return &Self::SYMBOLS_PLAIN;
        }

        // Check for NO_COLOR (standard for disabling color/unicode)
        if std::env::var("NO_COLOR").is_ok() {
            return &Self::SYMBOLS_PLAIN;
        }

        // Check TERM environment variable
        if let Ok(term) = std::env::var("TERM") {
            let term = term.to_lowercase();
            if term == "dumb" || term == "vt100" || term.contains("linux") {
                return &Self::SYMBOLS_PLAIN;
            }
        }

        // Default to fancy if we couldn't determine otherwise
        // Most modern terminals support Unicode
        &Self::SYMBOLS_FANCY
    }

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

    fn count_backups(&self, repo: &Repository) -> (usize, Option<String>, i64) {
        let mut backup_count = 0;
        let mut latest_commit_id = None;
        let mut latest_time = 0;

        let mut cmd = std::process::Command::new("git");
        cmd.current_dir(repo.path().parent().unwrap_or(repo.path()));
        cmd.args(&["log", "--all", "--format=%H %s"]);
        
        if let Ok(output) = cmd.output() {
            if let Ok(output_str) = String::from_utf8(output.stdout) {
                for line in output_str.lines() {
                    if line.ends_with("dura auto-backup") {
                        backup_count += 1;
                        if let Some(hash) = line.split_whitespace().next() {
                            if let Ok(oid) = git2::Oid::from_str(hash) {
                                if let Ok(commit) = repo.find_commit(oid) {
                                    let commit_time = commit.time().seconds();
                                    if commit_time > latest_time {
                                        latest_time = commit_time;
                                        latest_commit_id = Some(oid.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        
        (backup_count, latest_commit_id, latest_time)
    }

    pub fn print_summary(&self) {
        let symbols = Self::get_symbols();
        let [ok, modified, error, _warning, _info, _time, _stats, _folder] = symbols;

        println!("Dura Status Summary");
        println!("-----------------");
        
        // Add server status at the top
        let runtime_lock = RuntimeLock::load();
        match runtime_lock.pid {
            Some(pid) => {
                let uptime = runtime_lock.start_time
                    .and_then(|start| SystemTime::now().duration_since(start).ok())
                    .map(|duration| {
                        let days = duration.as_secs() / 86400;
                        let hours = (duration.as_secs() % 86400) / 3600;
                        let minutes = (duration.as_secs() % 3600) / 60;
                        if days > 0 {
                            format!("{}d {}h", days, hours)
                        } else if hours > 0 {
                            format!("{}h {}m", hours, minutes)
                        } else {
                            format!("{}m", minutes)
                        }
                    })
                    .unwrap_or_else(|| "unknown time".to_string());
                println!("Server: Running (PID: {}, Uptime: {})", pid, uptime);
            },
            None => println!("Server: Not running"),
        }
        println!();

        let total_repos = self.repos.len();
        let mut total_backups = 0;
        let mut repos_with_changes = 0;
        let mut inaccessible_repos = 0;

        for (path, _config) in &self.repos {
            let path = PathBuf::from(path);
            if !path.exists() {
                inaccessible_repos += 1;
                println!("{} {}: Not found", error, path.display());
                continue;
            }

            match Repository::open(&path) {
                Ok(repo) => {
                    let has_changes = repo.statuses(Some(git2::StatusOptions::new()
                        .include_untracked(true)
                        .include_ignored(false)
                        .include_unmodified(false)))
                        .map(|statuses| !statuses.is_empty())
                        .unwrap_or(false);
                    
                    if has_changes {
                        repos_with_changes += 1;
                    }

                    let (backup_count, latest_commit_id, _) = self.count_backups(&repo);
                    total_backups += backup_count;
                    
                    let commit_info = latest_commit_id
                        .map(|id| format!(" [{}]", &id[..7]))
                        .unwrap_or_default();
                    
                    println!("{}{}: {} backups{}{}", 
                        if has_changes { modified } else { ok },
                        path.display(),
                        backup_count,
                        commit_info,
                        if has_changes { " (uncommitted changes)" } else { "" }
                    );
                }
                Err(_) => {
                    inaccessible_repos += 1;
                    println!("{} {}: Not a git repository", error, path.display());
                }
            }
        }

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

    pub fn print_detailed_info(&self) {
        let symbols = Self::get_symbols();
        let [ok, modified, error, warning, info, time, stats, folder] = symbols;

        for (path, config) in &self.repos {
            let path = PathBuf::from(path);
            println!("{} {}", folder, path.display());

            if !path.exists() {
                println!("  {} Path does not exist", error);
                continue;
            }

            match Repository::open(&path) {
                Ok(repo) => {
                    println!("  {} Valid Git repository", ok);
                    
                    match repo.statuses(Some(git2::StatusOptions::new()
                        .include_untracked(true)
                        .include_ignored(false)
                        .include_unmodified(false))) 
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
                                    if let Some(path) = entry.path() {
                                        println!("  {} Change detected: {} ({:?})", 
                                               modified, path, status);
                                    }
                                    has_changes = true;
                                }
                            }

                            if has_changes {
                                println!("  {} Has uncommitted changes", warning);
                            } else {
                                println!("  {} No uncommitted changes", ok);
                            }
                        }
                        Err(e) => println!("  {} Unable to check repository status: {}", 
                                         warning, e),
                    }

                    let (backup_count, latest_commit_id, latest_time) = self.count_backups(&repo);
                    if backup_count > 0 {
                        if let Some(id) = latest_commit_id {
                            let time_sys = SystemTime::UNIX_EPOCH + 
                                     Duration::from_secs(latest_time as u64);
                            let datetime: DateTime<Local> = time_sys.into();
                            println!("  {} Last backup: {} ({})", 
                                   time,
                                   datetime.format("%Y-%m-%d %H:%M:%S"),
                                   &id[..7]);
                        }
                        println!("  {} Total backups: {}", stats, backup_count);
                    } else {
                        println!("  {} No backups found", info);
                    }

                    // Print watch configuration
                    println!("  Watch Configuration:");
                    if config.include.is_empty() {
                        println!("    Include: All files");
                    } else {
                        println!("    Include: {:?}", config.include);
                    }
                    println!("    Max depth: {}\n", config.max_depth);
                }
                Err(e) => {
                    println!("  {} Not a valid git repository: {}\n", error, e);
                }
            }
        }
    }
}
