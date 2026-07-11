use crate::command;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const CATDESK_DIR: &str = ".catdesk";
const PROMPTS_DIR: &str = "prompts";

#[derive(Clone, Copy)]
struct DefaultTemplate {
    name: &'static str,
    title: &'static str,
    body: &'static str,
}

const DEFAULT_TEMPLATES: [DefaultTemplate; 6] = [
    DefaultTemplate {
        name: "start_session.md",
        title: "Start Session",
        body: "Goal:\n\nProject context to read first:\n\nConstraints:\n\nDefinition of done:\n",
    },
    DefaultTemplate {
        name: "plan_first.md",
        title: "Plan First",
        body: "Goal:\n\nRelevant files:\n\nRisks:\n\nPlan:\n1. Inspect context.\n2. Identify the smallest safe change.\n3. Implement.\n4. Verify.\n",
    },
    DefaultTemplate {
        name: "security_review.md",
        title: "Security Review",
        body: "Review scope:\n\nPrioritize:\n- Data loss risks\n- Path traversal\n- Shell execution risks\n- Secret exposure\n- Permission boundary mistakes\n",
    },
    DefaultTemplate {
        name: "code_review.md",
        title: "Code Review",
        body: "Review focus:\n\nPrioritize:\n- Bugs and regressions\n- Missing tests\n- Safety or data-loss risks\n- Unclear user-facing behavior\n",
    },
    DefaultTemplate {
        name: "verify_changes.md",
        title: "Verify Changes",
        body: "Changed files:\n\nExpected behavior:\n\nVerification commands:\n\nKnown residual risk:\n",
    },
    DefaultTemplate {
        name: "end_session.md",
        title: "End Session",
        body: "Session goal:\n\nFiles changed:\n\nVerification:\n\nRemaining work:\n\nResume prompt:\n",
    },
];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptTemplate {
    pub name: String,
    pub path: String,
    pub bytes: usize,
    pub description: String,
    pub modified_time: Option<String>,
    pub text: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptTemplatesOutput {
    pub root: String,
    pub templates: Vec<PromptTemplate>,
}

impl PromptTemplatesOutput {
    pub fn render_text(&self) -> String {
        let mut out = format!("root: {}\ntemplates: {}\n", self.root, self.templates.len());
        for template in &self.templates {
            out.push_str(&format!(
                "\n## {}\npath: {}\nbytes: {}\n",
                template.name, template.path, template.bytes
            ));
            if !template.text.is_empty() {
                out.push('\n');
                out.push_str(&template.text);
                if !template.text.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
        out
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptTemplateWriteOutput {
    pub template: PromptTemplate,
    pub created: bool,
}

impl PromptTemplateWriteOutput {
    pub fn render_text(&self) -> String {
        format!(
            "{}: {}\npath: {}\nbytes: {}",
            if self.created { "created" } else { "updated" },
            self.template.name,
            self.template.path,
            self.template.bytes
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

fn prompts_root(root: &Path) -> PathBuf {
    root.join(CATDESK_DIR).join(PROMPTS_DIR)
}

fn default_template_text(template: DefaultTemplate) -> String {
    format!("# {}\n\n{}", template.title, template.body)
}

fn normalize_markdown(text: &str) -> String {
    let mut text = text.replace("\r\n", "\n").replace('\r', "\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

fn ensure_default_templates(root: &Path) -> Result<(), String> {
    let dir = prompts_root(root);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for template in DEFAULT_TEMPLATES {
        let path = dir.join(template.name);
        if !path.exists() {
            fs::write(&path, default_template_text(template)).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn safe_template_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("template name must not be empty".into());
    }
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err("template name must be a file name, not a path".into());
    }
    let stem = name.strip_suffix(".md").unwrap_or(name);
    if stem.is_empty()
        || !stem
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(
            "template name may contain only letters, numbers, hyphen, and underscore".into(),
        );
    }
    Ok(format!("{stem}.md"))
}

fn modified_time_string(path: &Path) -> Option<String> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let seconds = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(format!("unix:{seconds}"))
}

fn template_description(text: &str) -> String {
    text.lines()
        .find(|line| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with('#')
        })
        .map(str::trim)
        .unwrap_or("")
        .chars()
        .take(120)
        .collect()
}

fn read_template(root: &Path, path: PathBuf, include_text: bool) -> Result<PromptTemplate, String> {
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown.md".to_string());
    Ok(PromptTemplate {
        name,
        path: to_workspace_relative(root, &path),
        bytes: text.len(),
        description: template_description(&text),
        modified_time: modified_time_string(&path),
        text: if include_text { text } else { String::new() },
    })
}

pub fn init(workspace_root: &str) -> Result<PromptTemplatesOutput, String> {
    let root = workspace_root_path(workspace_root)?;
    ensure_default_templates(&root)?;
    list_from_root(&root)
}

pub fn list(workspace_root: &str) -> Result<PromptTemplatesOutput, String> {
    let root = workspace_root_path(workspace_root)?;
    list_from_root(&root)
}

fn list_from_root(root: &Path) -> Result<PromptTemplatesOutput, String> {
    let dir = prompts_root(root);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(PromptTemplatesOutput {
            root: to_workspace_relative(root, &dir),
            templates: Vec::new(),
        });
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut templates = Vec::new();
    for path in paths {
        templates.push(read_template(root, path, false)?);
    }
    Ok(PromptTemplatesOutput {
        root: to_workspace_relative(root, &dir),
        templates,
    })
}

pub fn read(workspace_root: &str, name: &str) -> Result<PromptTemplatesOutput, String> {
    let root = workspace_root_path(workspace_root)?;
    let name = safe_template_name(name)?;
    let path = prompts_root(&root).join(name);
    if !path.is_file() {
        return Err(format!(
            "Prompt template not found: {}",
            path.file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default()
        ));
    }
    Ok(PromptTemplatesOutput {
        root: to_workspace_relative(&root, &prompts_root(&root)),
        templates: vec![read_template(&root, path, true)?],
    })
}

pub fn write(
    workspace_root: &str,
    name: &str,
    content: &str,
    overwrite: bool,
) -> Result<PromptTemplateWriteOutput, String> {
    let root = workspace_root_path(workspace_root)?;
    fs::create_dir_all(prompts_root(&root)).map_err(|e| e.to_string())?;
    let name = safe_template_name(name)?;
    let path = prompts_root(&root).join(name);
    let created = !path.exists();
    if !created && !overwrite {
        return Err("Prompt template already exists. Set overwrite=true to replace it.".into());
    }
    let text = normalize_markdown(content);
    fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(PromptTemplateWriteOutput {
        template: read_template(&root, path, true)?,
        created,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_workspace(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "catdesk-prompt-templates-{name}-{}",
            Uuid::new_v4()
        ))
    }

    #[test]
    fn init_read_and_write_markdown_templates() {
        let workspace = test_workspace("templates");
        fs::create_dir_all(&workspace).expect("create workspace");

        let initial = init(&workspace.to_string_lossy()).expect("init templates");
        assert_eq!(initial.root, ".catdesk/prompts");
        assert_eq!(initial.templates.len(), 6);
        assert!(
            initial
                .templates
                .iter()
                .all(|template| template.text.is_empty())
        );

        let written = write(
            &workspace.to_string_lossy(),
            "bug-report",
            "# Bug Report\n\nSteps:\n",
            false,
        )
        .expect("write template");
        assert!(written.created);
        assert_eq!(written.template.name, "bug-report.md");

        let read_output = read(&workspace.to_string_lossy(), "bug-report").expect("read template");
        assert_eq!(read_output.templates.len(), 1);
        assert!(read_output.templates[0].text.contains("Steps:"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn list_does_not_initialize_or_return_bodies() {
        let workspace = test_workspace("list-no-init");
        fs::create_dir_all(&workspace).expect("create workspace");

        let listed = list(&workspace.to_string_lossy()).expect("list templates");

        assert!(listed.templates.is_empty());
        assert!(!workspace.join(CATDESK_DIR).exists());

        let _ = fs::remove_dir_all(workspace);
    }
}
