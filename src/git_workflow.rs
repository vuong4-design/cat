use crate::command;
use crate::verification;
use serde::Serialize;
use std::collections::{BTreeSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const GIT_TIMEOUT_MS: u64 = 120_000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusSummary {
    pub branch: String,
    pub clean: bool,
    pub warn_on_main: bool,
    pub raw: String,
    pub summary: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffSummary {
    pub staged: Vec<String>,
    pub unstaged: Vec<String>,
    pub untracked: Vec<String>,
    pub deleted: Vec<String>,
    pub renamed: Vec<String>,
    pub ignored: Vec<String>,
    pub stat: String,
    pub summary: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommandOutput {
    pub success: bool,
    pub summary: String,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitVerifiedOutput {
    pub success: bool,
    pub dry_run: bool,
    pub verification_status: String,
    pub verification_summary: String,
    pub staged_files: Vec<String>,
    pub confirmation_token: Option<String>,
    pub commit_preview: GitDiffSummary,
    pub commit: GitCommandOutput,
}

fn workspace_root_path(workspace_root: &str) -> Result<PathBuf, String> {
    Path::new(workspace_root)
        .canonicalize()
        .map(command::normalize_windows_verbatim_path)
        .map_err(|e| e.to_string())
}

fn validate_branch_name(branch: &str) -> Result<(), String> {
    let branch = branch.trim();
    if branch.is_empty() || branch.starts_with('-') || branch.contains("..") {
        return Err("Invalid branch name".into());
    }
    if !branch
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.'))
    {
        return Err(
            "Branch names may contain only ASCII letters, numbers, '/', '-', '_', and '.'".into(),
        );
    }
    Ok(())
}

fn current_branch_from_status(raw: &str) -> String {
    raw.lines()
        .next()
        .and_then(|line| line.strip_prefix("## "))
        .map(|line| {
            line.strip_prefix("No commits yet on ")
                .unwrap_or(line)
                .split(['.', ' '])
                .next()
                .unwrap_or(line)
                .to_string()
        })
        .filter(|branch| !branch.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

fn status_summary_text(branch: &str, clean: bool, warn_on_main: bool, raw: &str) -> String {
    let mut summary = format!(
        "branch: {branch}\nclean: {}\n",
        if clean { "yes" } else { "no" }
    );
    if warn_on_main {
        summary
            .push_str("warning: currently on main; create a feature branch before committing.\n");
    }
    if !raw.trim().is_empty() {
        summary.push_str("\n");
        summary.push_str(raw.trim());
    }
    summary
}

pub async fn status_summary(workspace_root: &str) -> Result<GitStatusSummary, String> {
    let root = workspace_root_path(workspace_root)?;
    let result = command::run_program(
        "git",
        &["status", "--short", "--branch"],
        &root,
        GIT_TIMEOUT_MS,
    )
    .await;
    if !result.success {
        return Err(command::format_result(&result));
    }
    let raw = result.stdout;
    let branch = current_branch_from_status(&raw);
    let clean = raw.lines().count() <= 1;
    let warn_on_main = matches!(branch.as_str(), "main" | "master");
    let summary = status_summary_text(&branch, clean, warn_on_main, &raw);
    Ok(GitStatusSummary {
        branch,
        clean,
        warn_on_main,
        raw,
        summary,
    })
}

pub async fn create_feature_branch(
    workspace_root: &str,
    branch: &str,
) -> Result<GitCommandOutput, String> {
    validate_branch_name(branch)?;
    let root = workspace_root_path(workspace_root)?;
    let result = command::run_program(
        "git",
        &["switch", "-c", branch.trim()],
        &root,
        GIT_TIMEOUT_MS,
    )
    .await;
    Ok(GitCommandOutput {
        success: result.success,
        summary: command::format_result(&result),
        stdout: result.stdout,
        stderr: result.stderr,
    })
}

fn parse_porcelain(
    raw: &str,
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
) {
    let mut staged = BTreeSet::new();
    let mut unstaged = BTreeSet::new();
    let mut untracked = BTreeSet::new();
    let mut deleted = BTreeSet::new();
    let mut renamed = BTreeSet::new();
    let mut ignored = BTreeSet::new();

    for line in raw.lines().filter(|line| line.len() >= 3) {
        let status = &line[..2];
        let path = line[3..].to_string();
        if status == "??" {
            untracked.insert(path);
            continue;
        }
        if status == "!!" {
            ignored.insert(path);
            continue;
        }
        let mut chars = status.chars();
        let index = chars.next().unwrap_or(' ');
        let worktree = chars.next().unwrap_or(' ');
        if index != ' ' {
            staged.insert(path.clone());
        }
        if worktree != ' ' {
            unstaged.insert(path.clone());
        }
        if index == 'D' || worktree == 'D' {
            deleted.insert(path.clone());
        }
        if index == 'R' || worktree == 'R' || path.contains(" -> ") {
            renamed.insert(path);
        }
    }

    (
        staged.into_iter().collect(),
        unstaged.into_iter().collect(),
        untracked.into_iter().collect(),
        deleted.into_iter().collect(),
        renamed.into_iter().collect(),
        ignored.into_iter().collect(),
    )
}

fn diff_summary_text(output: &GitDiffSummary) -> String {
    let mut summary = String::new();
    for (label, files) in [
        ("staged", &output.staged),
        ("unstaged", &output.unstaged),
        ("untracked", &output.untracked),
        ("deleted", &output.deleted),
        ("renamed", &output.renamed),
        ("ignored", &output.ignored),
    ] {
        summary.push_str(label);
        summary.push_str(":\n");
        if files.is_empty() {
            summary.push_str("- none\n");
        } else {
            for file in files {
                summary.push_str("- ");
                summary.push_str(file);
                summary.push('\n');
            }
        }
    }
    if !output.stat.trim().is_empty() {
        summary.push_str("\nstat:\n");
        summary.push_str(output.stat.trim());
        summary.push('\n');
    }
    summary
}

pub async fn diff_summary(
    workspace_root: &str,
    include_ignored: bool,
) -> Result<GitDiffSummary, String> {
    let root = workspace_root_path(workspace_root)?;
    let mut status_args = vec!["status", "--porcelain"];
    if include_ignored {
        status_args.push("--ignored");
    }
    let status = command::run_program("git", &status_args, &root, GIT_TIMEOUT_MS).await;
    let stat =
        command::run_program("git", &["diff", "--stat", "HEAD"], &root, GIT_TIMEOUT_MS).await;
    if !status.success {
        return Err(command::format_result(&status));
    }
    if !stat.success {
        return Err(command::format_result(&stat));
    }
    let (staged, unstaged, untracked, deleted, renamed, ignored) = parse_porcelain(&status.stdout);
    let mut output = GitDiffSummary {
        staged,
        unstaged,
        untracked,
        deleted,
        renamed,
        ignored,
        stat: stat.stdout,
        summary: String::new(),
    };
    output.summary = diff_summary_text(&output);
    Ok(output)
}

fn validate_stage_path(path: &str) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == "./"
        || trimmed == ".\\"
        || trimmed.ends_with('/')
        || trimmed.ends_with('\\')
        || trimmed.starts_with('-')
        || Path::new(trimmed).is_absolute()
    {
        return Err(format!("Invalid staged file path: {path}"));
    }
    Ok(())
}

fn validate_stage_files(root: &Path, files: &[String]) -> Result<(), String> {
    for file in files {
        validate_stage_path(file)?;
        let candidate = command::resolve_workspace_path(&root.to_string_lossy(), Some(file))?;
        if candidate.is_dir() {
            return Err(format!(
                "Invalid staged file path: {file}. git_commit_verified requires explicit files, not directories."
            ));
        }
    }
    Ok(())
}

fn literal_pathspec(path: &str) -> String {
    format!(":(literal){}", path.trim())
}

fn sorted_unique(files: Vec<String>) -> Vec<String> {
    files
        .into_iter()
        .map(|file| file.trim().replace('\\', "/"))
        .filter(|file| !file.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

async fn staged_files(root: &Path) -> Result<Vec<String>, String> {
    let result = command::run_program(
        "git",
        &["diff", "--cached", "--name-only"],
        root,
        GIT_TIMEOUT_MS,
    )
    .await;
    if !result.success {
        return Err(command::format_result(&result));
    }
    Ok(sorted_unique(
        result.stdout.lines().map(|line| line.to_string()).collect(),
    ))
}

fn unexpected_staged_files(staged: &[String], approved: &[String]) -> Vec<String> {
    let approved = approved.iter().cloned().collect::<BTreeSet<_>>();
    staged
        .iter()
        .filter(|file| !approved.contains(*file))
        .cloned()
        .collect()
}

async fn commit_preview_state(root: &Path) -> Result<String, String> {
    let result =
        command::run_program("git", &["status", "--porcelain"], root, GIT_TIMEOUT_MS).await;
    if !result.success {
        return Err(command::format_result(&result));
    }
    let diff = command::run_program(
        "git",
        &["diff", "--cached", "--binary"],
        root,
        GIT_TIMEOUT_MS,
    )
    .await;
    if !diff.success {
        return Err(command::format_result(&diff));
    }
    Ok(format!(
        "status:\n{}\n\ncached diff:\n{}",
        result.stdout, diff.stdout
    ))
}

fn commit_confirmation_fingerprint(
    branch: &str,
    message: &str,
    files: &[String],
    verification_status: &str,
    preview_state: &str,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    branch.hash(&mut hasher);
    message.trim().hash(&mut hasher);
    files.hash(&mut hasher);
    verification_status.hash(&mut hasher);
    preview_state.hash(&mut hasher);
    hasher.finish()
}

fn commit_confirmation_token(
    branch: &str,
    message: &str,
    files: &[String],
    verification_status: &str,
    preview_state: &str,
) -> Result<String, String> {
    let issued_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let fingerprint =
        commit_confirmation_fingerprint(branch, message, files, verification_status, preview_state);
    Ok(format!("commit:{issued_at}:{fingerprint:016x}"))
}

fn validate_commit_confirmation_token(
    token: Option<&str>,
    branch: &str,
    message: &str,
    files: &[String],
    verification_status: &str,
    preview_state: &str,
) -> Result<(), String> {
    let Some(token) = token.filter(|value| !value.trim().is_empty()) else {
        return Err("Run git_commit_verified with dry_run=true first and pass the returned commit_confirmation_token.".into());
    };
    let mut parts = token.split(':');
    if parts.next() != Some("commit") {
        return Err("Invalid commit confirmation token.".into());
    }
    let issued_at = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| "Invalid commit confirmation token.".to_string())?;
    let fingerprint = parts
        .next()
        .and_then(|value| u64::from_str_radix(value, 16).ok())
        .ok_or_else(|| "Invalid commit confirmation token.".to_string())?;
    if parts.next().is_some() {
        return Err("Invalid commit confirmation token.".into());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    if now.saturating_sub(issued_at) > 600 {
        return Err("Commit confirmation token expired; run dry_run=true again.".into());
    }
    let expected =
        commit_confirmation_fingerprint(branch, message, files, verification_status, preview_state);
    if fingerprint != expected {
        return Err("Commit confirmation token does not match the current staged preview.".into());
    }
    Ok(())
}

pub async fn commit_verified_changes(
    workspace_root: &str,
    message: &str,
    files: Vec<String>,
    allow_failed_verification: bool,
    allow_partial_verification: bool,
    allow_main: bool,
    dry_run: bool,
    provided_confirmation_token: Option<&str>,
) -> Result<GitCommitVerifiedOutput, String> {
    if message.trim().is_empty() {
        return Err("Commit message must not be empty".into());
    }
    if files.is_empty() {
        return Err("git_commit_verified requires an explicit non-empty files list".into());
    }
    let root = workspace_root_path(workspace_root)?;
    let files = sorted_unique(files);
    validate_stage_files(&root, &files)?;
    let status = status_summary(workspace_root).await?;
    if matches!(status.branch.as_str(), "main" | "master") && !allow_main {
        return Err("Refusing to commit on main/master. Use git_create_feature_branch first, or pass allow_main=true for an explicit override.".into());
    }
    let verification = verification::verify_project(workspace_root).await?;
    let verification_summary = verification.render_text();
    let verification_allowed = match verification.status.as_str() {
        "PASSED" => true,
        "PARTIAL" => allow_partial_verification || allow_failed_verification,
        "FAILED" | "NOT_CONFIGURED" => allow_failed_verification,
        _ => false,
    };
    if !verification_allowed {
        return Ok(GitCommitVerifiedOutput {
            success: false,
            dry_run,
            verification_status: verification.status,
            verification_summary,
            staged_files: files,
            confirmation_token: None,
            commit_preview: diff_summary(workspace_root, false).await?,
            commit: GitCommandOutput {
                success: false,
                summary: "verification did not pass; commit was not created".into(),
                stdout: String::new(),
                stderr: String::new(),
            },
        });
    }

    let already_staged = staged_files(&root).await?;
    let unexpected = unexpected_staged_files(&already_staged, &files);
    if !unexpected.is_empty() {
        return Err(format!(
            "Refusing verified commit because unrelated files are already staged: {}",
            unexpected.join(", ")
        ));
    }

    let mut add_args = vec!["add", "--"];
    let pathspecs = files
        .iter()
        .map(|file| literal_pathspec(file))
        .collect::<Vec<_>>();
    add_args.extend(pathspecs.iter().map(String::as_str));
    let add = command::run_program("git", &add_args, &root, GIT_TIMEOUT_MS).await;
    if !add.success {
        return Err(command::format_result(&add));
    }

    let staged_after_add = staged_files(&root).await?;
    if staged_after_add != files {
        return Err(format!(
            "Refusing verified commit because staged files do not exactly match requested files.\nrequested: {}\nstaged: {}",
            files.join(", "),
            staged_after_add.join(", ")
        ));
    }

    let preview = diff_summary(workspace_root, false).await?;
    let preview_state = commit_preview_state(&root).await?;
    if dry_run {
        let token = commit_confirmation_token(
            &status.branch,
            message,
            &files,
            &verification.status,
            &preview_state,
        )?;
        return Ok(GitCommitVerifiedOutput {
            success: true,
            dry_run: true,
            verification_status: verification.status,
            verification_summary,
            staged_files: files,
            confirmation_token: Some(token),
            commit_preview: preview,
            commit: GitCommandOutput {
                success: true,
                summary:
                    "dry run: verification passed and requested files would be staged/committed"
                        .into(),
                stdout: String::new(),
                stderr: String::new(),
            },
        });
    }

    validate_commit_confirmation_token(
        provided_confirmation_token,
        &status.branch,
        message,
        &files,
        &verification.status,
        &preview_state,
    )?;

    let commit = command::run_program(
        "git",
        &["commit", "-m", message.trim()],
        &root,
        GIT_TIMEOUT_MS,
    )
    .await;
    Ok(GitCommitVerifiedOutput {
        success: commit.success,
        dry_run: false,
        verification_status: verification.status,
        verification_summary,
        staged_files: files,
        confirmation_token: None,
        commit_preview: preview,
        commit: GitCommandOutput {
            success: commit.success,
            summary: command::format_result(&commit),
            stdout: commit.stdout,
            stderr: commit.stderr,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_and_warns_on_main() {
        let raw = "## main...origin/main\n M src/main.rs\n";
        let branch = current_branch_from_status(raw);
        assert_eq!(branch, "main");
        assert_eq!(
            current_branch_from_status("## No commits yet on main\n"),
            "main"
        );
        let summary = status_summary_text(&branch, false, true, raw);
        assert!(summary.contains("warning: currently on main"));
    }

    #[test]
    fn validates_feature_branch_names() {
        assert!(validate_branch_name("feature/git-workflow").is_ok());
        assert!(validate_branch_name("-bad").is_err());
        assert!(validate_branch_name("bad name").is_err());
        assert!(validate_branch_name("bad..name").is_err());
    }

    #[test]
    fn parses_porcelain_sections() {
        let raw =
            "M  staged.rs\n M unstaged.rs\n?? new file.rs\nD  deleted.rs\nR  old.rs -> new.rs\n";
        let (staged, unstaged, untracked, deleted, renamed, ignored) = parse_porcelain(raw);

        assert!(staged.contains(&"staged.rs".to_string()));
        assert!(unstaged.contains(&"unstaged.rs".to_string()));
        assert!(untracked.contains(&"new file.rs".to_string()));
        assert!(deleted.contains(&"deleted.rs".to_string()));
        assert!(renamed.contains(&"old.rs -> new.rs".to_string()));
        assert!(ignored.is_empty());
    }

    #[test]
    fn validates_explicit_stage_paths() {
        assert!(validate_stage_path("src/file with spaces.rs").is_ok());
        assert!(validate_stage_path("src/file[1].rs").is_ok());
        assert!(validate_stage_path(".").is_err());
        assert!(validate_stage_path("./").is_err());
        assert!(validate_stage_path("src/").is_err());
        assert!(validate_stage_path("-bad").is_err());
        assert!(validate_stage_path("C:/secret.txt").is_err());
    }

    #[test]
    fn commit_confirmation_token_tracks_preview_state() {
        let files = vec!["src/main.rs".to_string()];
        let porcelain = "M  src/main.rs\n";
        let token =
            commit_confirmation_token("feature/demo", "Update demo", &files, "PASSED", porcelain)
                .expect("token");
        assert!(
            validate_commit_confirmation_token(
                Some(&token),
                "feature/demo",
                "Update demo",
                &files,
                "PASSED",
                porcelain
            )
            .is_ok()
        );
        assert!(
            validate_commit_confirmation_token(
                Some(&token),
                "feature/demo",
                "Different message",
                &files,
                "PASSED",
                porcelain
            )
            .is_err()
        );
    }
}
