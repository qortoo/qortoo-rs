use std::{cmp::min, fs, path::PathBuf, process::Command};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    let git_hash = resolve_git_hash().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_HASH={}", git_hash);
    Ok(())
}

fn resolve_git_hash() -> Option<String> {
    resolve_git_hash_from_git_dir().or_else(resolve_git_hash_from_git_cmd)
}

fn resolve_git_hash_from_git_dir() -> Option<String> {
    let head_path = PathBuf::from(".git/HEAD");
    let head = fs::read_to_string(&head_path).ok()?;
    let head = head.trim();

    if let Some(ref_path) = head.strip_prefix("ref: ") {
        let full_ref_path = PathBuf::from(".git").join(ref_path);
        let commit = fs::read_to_string(full_ref_path).ok()?;
        let commit = commit.trim();
        if commit.is_empty() {
            None
        } else {
            Some(commit[..min(commit.len(), 7)].to_string())
        }
    } else if head.is_empty() {
        None
    } else {
        Some(head[..min(head.len(), 7)].to_string())
    }
}

fn resolve_git_hash_from_git_cmd() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let git_hash = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if git_hash.is_empty() {
        None
    } else {
        Some(git_hash)
    }
}
