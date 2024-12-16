// src/repo_status.rs
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug)]
pub struct RepoStatus {
    pub path: PathBuf,
    pub exists: bool,
    pub is_git_repo: bool,
    pub last_backup: Option<SystemTime>,
    pub uncommitted_changes: bool,
}