use crate::command;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

const MEMORY_DIR: &str = ".catdesk";
const DEFAULT_SECTION: &str = "_default";

#[derive(Clone, Copy)]
struct MemoryDocumentDef {
    name: &'static str,
    file_name: &'static str,
    title: &'static str,
    body: &'static str,
}

const MEMORY_DOCUMENTS: [MemoryDocumentDef; 4] = [
    MemoryDocumentDef {
        name: "project",
        file_name: "project.md",
        title: "Project Memory",
        body: "Persistent notes about this project.\n",
    },
    MemoryDocumentDef {
        name: "decisions",
        file_name: "decisions.md",
        title: "Decisions",
        body: "Durable architectural and product decisions.\n",
    },
    MemoryDocumentDef {
        name: "todo",
        file_name: "todo.md",
        title: "Todo",
        body: "- [ ] Capture project follow-up work here.\n",
    },
    MemoryDocumentDef {
        name: "session",
        file_name: "session.md",
        title: "Session",
        body: "Current session state and handoff notes.\n",
    },
];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDocument {
    pub name: String,
    pub path: String,
    pub text: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMemoryOutput {
    pub root: String,
    pub documents: Vec<MemoryDocument>,
}

impl ProjectMemoryOutput {
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("root: {}\n", self.root));
        out.push_str(&format!("documents: {}\n", self.documents.len()));
        for document in &self.documents {
            out.push_str(&format!(
                "\n## {}\npath: {}\n\n",
                document.name, document.path
            ));
            out.push_str(&document.text);
            if !document.text.ends_with('\n') {
                out.push('\n');
            }
        }
        out
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMemoryUpdateOutput {
    pub document: MemoryDocument,
    pub mode: String,
    pub bytes: usize,
}

impl ProjectMemoryUpdateOutput {
    pub fn render_text(&self) -> String {
        format!(
            "updated: {}\npath: {}\nmode: {}\nbytes: {}",
            self.document.name, self.document.path, self.mode, self.bytes
        )
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResumeOutput {
    pub document: MemoryDocument,
    pub session_goal: String,
    pub files_changed: Vec<String>,
    pub verification_results: String,
    pub remaining_work: String,
    pub resume_prompt: String,
}

impl SessionResumeOutput {
    pub fn render_text(&self) -> String {
        format!(
            "updated: {}\npath: {}\nsections: session goal, files changed, verification results, remaining work, resume prompt",
            self.document.name, self.document.path
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

fn memory_root(root: &Path) -> PathBuf {
    root.join(MEMORY_DIR)
}

fn default_document_text(def: MemoryDocumentDef) -> String {
    format!("# {}\n\n{}", def.title, def.body)
}

fn document_def(name: &str) -> Option<MemoryDocumentDef> {
    MEMORY_DOCUMENTS
        .iter()
        .copied()
        .find(|def| def.name.eq_ignore_ascii_case(name) || def.file_name.eq_ignore_ascii_case(name))
}

fn ensure_memory_files(root: &Path) -> Result<(), String> {
    let dir = memory_root(root);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for def in MEMORY_DOCUMENTS {
        let path = dir.join(def.file_name);
        if !path.exists() {
            fs::write(&path, default_document_text(def)).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn read_document(root: &Path, def: MemoryDocumentDef) -> Result<MemoryDocument, String> {
    let path = memory_root(root).join(def.file_name);
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    Ok(MemoryDocument {
        name: def.name.to_string(),
        path: to_workspace_relative(root, &path),
        text,
    })
}

fn normalize_markdown(content: &str) -> String {
    let mut text = content.replace("\r\n", "\n").replace('\r', "\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

fn append_markdown(existing: &str, content: &str, section: Option<&str>) -> String {
    let mut text = normalize_markdown(existing);
    let content = normalize_markdown(content);
    let section = section.unwrap_or(DEFAULT_SECTION).trim();
    if !section.is_empty() && section != DEFAULT_SECTION {
        if !text.ends_with("\n\n") {
            text.push('\n');
        }
        text.push_str(&format!("## {section}\n\n"));
    } else if !text.ends_with("\n\n") {
        text.push('\n');
    }
    text.push_str(&content);
    text
}

fn markdown_block(text: &str) -> String {
    let text = normalize_markdown(text.trim());
    if text.trim().is_empty() {
        "_None recorded._\n".to_string()
    } else {
        text
    }
}

fn markdown_list(items: &[String]) -> String {
    let items = items
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if items.is_empty() {
        return "- None recorded.\n".to_string();
    }
    let mut out = String::new();
    for item in items {
        out.push_str("- ");
        out.push_str(item);
        out.push('\n');
    }
    out
}

fn session_resume_text(
    session_goal: &str,
    files_changed: &[String],
    verification_results: &str,
    remaining_work: &str,
    resume_prompt: &str,
) -> String {
    format!(
        "# Session\n\n\
## Session goal\n\n{}\
\n## Files changed\n\n{}\
\n## Verification results\n\n{}\
\n## Remaining work\n\n{}\
\n## Resume prompt\n\n{}",
        markdown_block(session_goal),
        markdown_list(files_changed),
        markdown_block(verification_results),
        markdown_block(remaining_work),
        markdown_block(resume_prompt),
    )
}

pub fn init(workspace_root: &str) -> Result<ProjectMemoryOutput, String> {
    let root = workspace_root_path(workspace_root)?;
    ensure_memory_files(&root)?;
    read_all_from_root(&root)
}

pub fn read(workspace_root: &str, document: Option<&str>) -> Result<ProjectMemoryOutput, String> {
    let root = workspace_root_path(workspace_root)?;
    ensure_memory_files(&root)?;
    if let Some(document) = document.filter(|value| !value.trim().is_empty()) {
        let def = document_def(document.trim()).ok_or_else(|| {
            format!(
                "Unknown memory document: {document}. Use one of: project, decisions, todo, session"
            )
        })?;
        return Ok(ProjectMemoryOutput {
            root: to_workspace_relative(&root, &memory_root(&root)),
            documents: vec![read_document(&root, def)?],
        });
    }
    read_all_from_root(&root)
}

fn read_all_from_root(root: &Path) -> Result<ProjectMemoryOutput, String> {
    let mut documents = Vec::new();
    for def in MEMORY_DOCUMENTS {
        documents.push(read_document(root, def)?);
    }
    Ok(ProjectMemoryOutput {
        root: to_workspace_relative(root, &memory_root(root)),
        documents,
    })
}

pub fn update(
    workspace_root: &str,
    document: &str,
    content: &str,
    mode: Option<&str>,
    section: Option<&str>,
) -> Result<ProjectMemoryUpdateOutput, String> {
    let root = workspace_root_path(workspace_root)?;
    ensure_memory_files(&root)?;
    let def = document_def(document.trim()).ok_or_else(|| {
        format!(
            "Unknown memory document: {document}. Use one of: project, decisions, todo, session"
        )
    })?;
    let path = memory_root(&root).join(def.file_name);
    let mode = mode.unwrap_or("append").trim();
    let next_text = match mode {
        "append" => {
            let existing = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            append_markdown(&existing, content, section)
        }
        "overwrite" => normalize_markdown(content),
        _ => return Err("mode must be either append or overwrite".into()),
    };
    fs::write(&path, &next_text).map_err(|e| e.to_string())?;
    Ok(ProjectMemoryUpdateOutput {
        document: read_document(&root, def)?,
        mode: mode.to_string(),
        bytes: next_text.len(),
    })
}

pub fn update_session_resume(
    workspace_root: &str,
    session_goal: &str,
    files_changed: Vec<String>,
    verification_results: &str,
    remaining_work: &str,
    resume_prompt: &str,
) -> Result<SessionResumeOutput, String> {
    let root = workspace_root_path(workspace_root)?;
    ensure_memory_files(&root)?;
    let def = document_def("session").expect("session memory document exists");
    let path = memory_root(&root).join(def.file_name);
    let content = session_resume_text(
        session_goal,
        &files_changed,
        verification_results,
        remaining_work,
        resume_prompt,
    );
    fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(SessionResumeOutput {
        document: read_document(&root, def)?,
        session_goal: session_goal.trim().to_string(),
        files_changed,
        verification_results: verification_results.trim().to_string(),
        remaining_work: remaining_work.trim().to_string(),
        resume_prompt: resume_prompt.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_workspace(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("catdesk-project-memory-{name}-{}", Uuid::new_v4()))
    }

    #[test]
    fn init_creates_markdown_memory_files() {
        let workspace = test_workspace("init");
        fs::create_dir_all(&workspace).expect("create workspace");
        let output = init(&workspace.to_string_lossy()).expect("init memory");

        assert_eq!(output.root, ".catdesk");
        assert_eq!(output.documents.len(), 4);
        for def in MEMORY_DOCUMENTS {
            let path = workspace.join(MEMORY_DIR).join(def.file_name);
            let text = fs::read_to_string(path).expect("read memory file");
            assert!(text.starts_with("# "));
        }

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn update_appends_markdown_section() {
        let workspace = test_workspace("append");
        fs::create_dir_all(&workspace).expect("create workspace");

        update(
            &workspace.to_string_lossy(),
            "decisions",
            "- Use markdown files for project memory.",
            Some("append"),
            Some("Architecture"),
        )
        .expect("update memory");

        let text = fs::read_to_string(workspace.join(MEMORY_DIR).join("decisions.md"))
            .expect("read decisions");
        assert!(text.contains("## Architecture"));
        assert!(text.contains("- Use markdown files for project memory."));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn update_session_resume_writes_required_sections() {
        let workspace = test_workspace("session-resume");
        fs::create_dir_all(&workspace).expect("create workspace");

        let output = update_session_resume(
            &workspace.to_string_lossy(),
            "Improve CatDesk",
            vec![
                "src/project_memory.rs".to_string(),
                "src/mcp.rs".to_string(),
            ],
            "cargo test project_memory passed",
            "Implement repository map",
            "Continue with Task 7",
        )
        .expect("update session resume");

        assert_eq!(output.document.name, "session");
        let text = fs::read_to_string(workspace.join(MEMORY_DIR).join("session.md"))
            .expect("read session memory");
        for heading in [
            "## Session goal",
            "## Files changed",
            "## Verification results",
            "## Remaining work",
            "## Resume prompt",
        ] {
            assert!(text.contains(heading), "missing heading {heading}");
        }
        assert!(text.contains("- src/project_memory.rs"));
        assert!(text.contains("Continue with Task 7"));

        let _ = fs::remove_dir_all(workspace);
    }
}
