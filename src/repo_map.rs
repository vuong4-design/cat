use crate::command;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

const CATDESK_DIR: &str = ".catdesk";
const REPO_MAP_FILE: &str = "repo_map.md";
const MAX_FILES_SCANNED: usize = 10_000;
const MAX_IMPORTANT_FOLDERS: usize = 16;
const MAX_ENTRY_POINTS: usize = 24;

const IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".idea",
    ".vscode",
    ".catdesk",
    ".venv",
    "venv",
    "target",
    "node_modules",
    "dist",
    "build",
    "coverage",
    ".next",
    ".nuxt",
    ".svelte-kit",
    "vendor",
    "vendors",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    "review-bundles",
    "npm/bin",
];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoMapOutput {
    pub path: String,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub important_folders: Vec<String>,
    pub entry_points: Vec<String>,
    pub build_test_commands: Vec<String>,
    pub files_scanned: usize,
    pub truncated: bool,
    pub text: String,
}

impl RepoMapOutput {
    pub fn render_text(&self) -> String {
        format!(
            "wrote: {}\nfiles scanned: {}\ntruncated: {}\n\n{}",
            self.path, self.files_scanned, self.truncated, self.text
        )
    }
}

fn workspace_root_path(workspace_root: &str) -> Result<PathBuf, String> {
    Path::new(workspace_root)
        .canonicalize()
        .map(command::normalize_windows_verbatim_path)
        .map_err(|e| e.to_string())
}

fn tool_path_string(path: &Path) -> String {
    let path = path.display().to_string();
    #[cfg(windows)]
    {
        path.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        path
    }
}

fn to_workspace_relative(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".into(),
        Ok(rel) => tool_path_string(rel),
        Err(_) => tool_path_string(path),
    }
}

fn is_ignored_dir(rel: &str) -> bool {
    let rel = rel.replace('\\', "/");
    IGNORED_DIRS
        .iter()
        .any(|ignored| rel == *ignored || rel.starts_with(&format!("{ignored}/")))
}

fn file_language(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("rs") => Some("Rust"),
        Some("js") | Some("mjs") | Some("cjs") => Some("JavaScript"),
        Some("ts") | Some("tsx") => Some("TypeScript"),
        Some("py") => Some("Python"),
        Some("go") => Some("Go"),
        Some("java") => Some("Java"),
        Some("html") | Some("htm") => Some("HTML"),
        Some("css") => Some("CSS"),
        Some("json") => Some("JSON"),
        Some("toml") => Some("TOML"),
        Some("md") => Some("Markdown"),
        Some("yml") | Some("yaml") => Some("YAML"),
        Some("sh") | Some("bash") | Some("ps1") => Some("Shell"),
        _ => None,
    }
}

fn collect_files(root: &Path) -> Result<(Vec<PathBuf>, bool), String> {
    let mut files = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back(root.to_path_buf());
    let mut truncated = false;

    while let Some(dir) = queue.pop_front() {
        let mut entries = fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|entry| entry.path());

        for entry in entries {
            let path = entry.path();
            let rel = to_workspace_relative(root, &path);
            let file_type = entry.file_type().map_err(|e| e.to_string())?;
            if file_type.is_dir() {
                if !is_ignored_dir(&rel) {
                    queue.push_back(path);
                }
            } else if file_type.is_file() {
                files.push(path);
                if files.len() >= MAX_FILES_SCANNED {
                    truncated = true;
                    return Ok((files, truncated));
                }
            }
        }
    }

    Ok((files, truncated))
}

fn detect_languages(files: &[PathBuf]) -> Vec<String> {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for file in files {
        if let Some(language) = file_language(file) {
            *counts.entry(language).or_default() += 1;
        }
    }
    let mut entries = counts.into_iter().collect::<Vec<_>>();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    entries
        .into_iter()
        .map(|(language, count)| format!("{language} ({count})"))
        .collect()
}

fn detect_frameworks(root: &Path) -> Vec<String> {
    let mut frameworks = BTreeSet::new();
    let cargo = fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
    for (needle, label) in [
        ("axum", "Axum"),
        ("tokio", "Tokio"),
        ("ratatui", "Ratatui"),
        ("crossterm", "Crossterm"),
        ("reqwest", "Reqwest"),
        ("ngrok", "ngrok Rust SDK"),
        ("tree-sitter", "Tree-sitter"),
    ] {
        if cargo.contains(needle) {
            frameworks.insert(label.to_string());
        }
    }

    let package_json = fs::read_to_string(root.join("package.json")).unwrap_or_default();
    for (needle, label) in [
        ("vite", "Vite"),
        ("react", "React"),
        ("next", "Next.js"),
        ("typescript", "TypeScript"),
    ] {
        if package_json.contains(needle) {
            frameworks.insert(label.to_string());
        }
    }

    frameworks.into_iter().collect()
}

fn detect_important_folders(root: &Path, files: &[PathBuf]) -> Vec<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for file in files {
        let rel = to_workspace_relative(root, file);
        let top = rel.split('/').next().unwrap_or(".");
        if top != "." && !top.contains('.') {
            *counts.entry(top.to_string()).or_default() += 1;
        }
    }
    let mut entries = counts.into_iter().collect::<Vec<_>>();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    entries
        .into_iter()
        .take(MAX_IMPORTANT_FOLDERS)
        .map(|(folder, count)| format!("{folder}/ ({count} files)"))
        .collect()
}

fn detect_entry_points(root: &Path, files: &[PathBuf]) -> Vec<String> {
    let existing = files
        .iter()
        .map(|file| to_workspace_relative(root, file))
        .collect::<BTreeSet<_>>();
    let mut entries = BTreeSet::new();

    for candidate in ["src/main.rs", "main.py", "app.py", "wsgi.py", "asgi.py"] {
        if existing.contains(candidate) {
            entries.insert(candidate.to_string());
        }
    }
    for file in &existing {
        if file.starts_with("src/bin/") && file.ends_with(".rs") {
            entries.insert(file.clone());
        }
        if file.ends_with("/__main__.py") {
            entries.insert(file.clone());
        }
    }
    if let Ok(package_text) = fs::read_to_string(root.join("package.json")) {
        if let Ok(package) = serde_json::from_str::<Value>(&package_text) {
            for field in ["main", "module", "browser"] {
                if let Some(path) = package.get(field).and_then(Value::as_str) {
                    if existing.contains(path) {
                        entries.insert(format!("package.json {field}: {path}"));
                    }
                }
            }
            if let Some(bin) = package.get("bin") {
                match bin {
                    Value::String(path) if existing.contains(path.as_str()) => {
                        entries.insert(format!("package.json bin: {path}"));
                    }
                    Value::Object(map) => {
                        for (name, path) in map {
                            if let Some(path) =
                                path.as_str().filter(|path| existing.contains(*path))
                            {
                                entries.insert(format!("package.json bin.{name}: {path}"));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for file in files {
        let rel = to_workspace_relative(root, file);
        if rel.ends_with(".go")
            && fs::read_to_string(file)
                .unwrap_or_default()
                .lines()
                .any(|line| line.trim() == "package main")
        {
            entries.insert(rel.clone());
        }
        if rel.ends_with(".java")
            && fs::read_to_string(file)
                .unwrap_or_default()
                .contains("public static void main")
        {
            entries.insert(rel);
        }
    }
    entries.into_iter().take(MAX_ENTRY_POINTS).collect()
}

fn detect_build_test_commands(root: &Path) -> Vec<String> {
    let mut commands = Vec::new();
    if root.join("Cargo.toml").is_file() {
        commands.extend([
            "cargo fmt --check".to_string(),
            "cargo test".to_string(),
            "cargo build".to_string(),
        ]);
    }
    if root.join("package.json").is_file() {
        let node_runner = node_package_manager(root);
        let package = fs::read_to_string(root.join("package.json")).unwrap_or_default();
        if package.contains("\"test\"") {
            commands.push(format!("{node_runner} test"));
        }
        if package.contains("\"build\"") {
            commands.push(format!("{node_runner} run build"));
        }
        if package.contains("\"lint\"") {
            commands.push(format!("{node_runner} run lint"));
        }
    }
    commands
}

fn node_package_manager(root: &Path) -> &'static str {
    if root.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if root.join("yarn.lock").is_file() {
        "yarn"
    } else if root.join("bun.lockb").is_file() || root.join("bun.lock").is_file() {
        "bun"
    } else {
        "npm"
    }
}

fn markdown_list(items: &[String]) -> String {
    if items.is_empty() {
        return "- None detected.\n".to_string();
    }
    let mut out = String::new();
    for item in items {
        out.push_str("- ");
        out.push_str(item);
        out.push('\n');
    }
    out
}

fn repo_map_text(output: &RepoMapOutput) -> String {
    format!(
        "# Repository Map\n\n\
## Languages\n\n{}\
\n## Frameworks\n\n{}\
\n## Important folders\n\n{}\
\n## Entry points\n\n{}\
\n## Build/test commands\n\n{}\
\n## Scan notes\n\n- Files scanned: {}\n- Truncated: {}\n- Ignored generated/vendor directories: {}\n",
        markdown_list(&output.languages),
        markdown_list(&output.frameworks),
        markdown_list(&output.important_folders),
        markdown_list(&output.entry_points),
        markdown_list(&output.build_test_commands),
        output.files_scanned,
        output.truncated,
        IGNORED_DIRS.join(", "),
    )
}

pub fn generate(workspace_root: &str) -> Result<RepoMapOutput, String> {
    let root = workspace_root_path(workspace_root)?;
    let (files, truncated) = collect_files(&root)?;
    let map_path = root.join(CATDESK_DIR).join(REPO_MAP_FILE);
    fs::create_dir_all(root.join(CATDESK_DIR)).map_err(|e| e.to_string())?;

    let mut output = RepoMapOutput {
        path: to_workspace_relative(&root, &map_path),
        languages: detect_languages(&files),
        frameworks: detect_frameworks(&root),
        important_folders: detect_important_folders(&root, &files),
        entry_points: detect_entry_points(&root, &files),
        build_test_commands: detect_build_test_commands(&root),
        files_scanned: files.len(),
        truncated,
        text: String::new(),
    };
    output.text = repo_map_text(&output);
    fs::write(&map_path, &output.text).map_err(|e| e.to_string())?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_workspace(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("catdesk-repo-map-{name}-{}", Uuid::new_v4()))
    }

    #[test]
    fn generate_writes_repo_map_and_ignores_generated_dirs() {
        let workspace = test_workspace("generate");
        fs::create_dir_all(workspace.join("src")).expect("create src");
        fs::create_dir_all(workspace.join("target").join("debug")).expect("create target");
        fs::create_dir_all(workspace.join("node_modules").join("pkg"))
            .expect("create node_modules");
        fs::write(
            workspace.join("Cargo.toml"),
            "[package]\nname = \"demo\"\n[dependencies]\naxum = \"0.8\"\ntokio = \"1\"\n",
        )
        .expect("write cargo");
        fs::write(workspace.join("src").join("main.rs"), "fn main() {}\n").expect("write main");
        fs::create_dir_all(workspace.join("cmd").join("demo")).expect("create go dir");
        fs::write(
            workspace.join("cmd").join("demo").join("main.go"),
            "package main\nfunc main() {}\n",
        )
        .expect("write go main");
        fs::write(
            workspace.join("target").join("debug").join("generated.rs"),
            "ignored\n",
        )
        .expect("write ignored target");
        fs::write(
            workspace.join("node_modules").join("pkg").join("index.js"),
            "ignored\n",
        )
        .expect("write ignored node_modules");

        let output = generate(&workspace.to_string_lossy()).expect("generate repo map");
        assert_eq!(output.path, ".catdesk/repo_map.md");
        assert!(
            output
                .languages
                .iter()
                .any(|entry| entry.starts_with("Rust"))
        );
        assert!(output.frameworks.contains(&"Axum".to_string()));
        assert!(output.frameworks.contains(&"Tokio".to_string()));
        assert!(output.entry_points.contains(&"src/main.rs".to_string()));
        assert!(
            output
                .entry_points
                .contains(&"cmd/demo/main.go".to_string())
        );
        assert!(
            output
                .build_test_commands
                .contains(&"cargo test".to_string())
        );
        assert!(!output.text.contains("node_modules/pkg"));
        assert!(workspace.join(".catdesk").join("repo_map.md").is_file());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn detect_build_test_commands_uses_detected_node_package_manager() {
        let workspace = test_workspace("pnpm");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::write(
            workspace.join("package.json"),
            r#"{"scripts":{"test":"vitest","build":"vite build","lint":"eslint ."}}"#,
        )
        .expect("write package");
        fs::write(workspace.join("pnpm-lock.yaml"), "").expect("write lockfile");

        let commands = detect_build_test_commands(&workspace);
        assert!(commands.contains(&"pnpm test".to_string()));
        assert!(commands.contains(&"pnpm run build".to_string()));
        assert!(commands.contains(&"pnpm run lint".to_string()));

        let _ = fs::remove_dir_all(workspace);
    }
}
