use crate::command;
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const CATDESK_DIR: &str = ".catdesk";
const TODO_FILE: &str = "todo.md";
const NEXT_TASK_ID_MARKER_PREFIX: &str = "<!-- catdesk-next-task-id: ";
const DEFAULT_TODO_TEXT: &str =
    "# Todo\n\n<!-- catdesk-next-task-id: 1 -->\n\nUse task_queue_add to record follow-up work.\n";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskItem {
    pub id: String,
    pub done: bool,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskQueueOutput {
    pub path: String,
    pub total: usize,
    pub open: usize,
    pub done: usize,
    pub tasks: Vec<TaskItem>,
    pub text: String,
}

impl TaskQueueOutput {
    pub fn render_text(&self) -> String {
        let mut out = format!(
            "path: {}\ntotal: {}\nopen: {}\ndone: {}\n",
            self.path, self.total, self.open, self.done
        );
        if self.tasks.is_empty() {
            out.push_str("\n_No tasks recorded._\n");
        } else {
            out.push_str("\n## Tasks\n");
            for task in &self.tasks {
                let marker = if task.done { "x" } else { " " };
                out.push_str(&format!("- [{}] {} {}\n", marker, task.id, task.text));
            }
        }
        out
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

fn todo_path(root: &Path) -> PathBuf {
    root.join(CATDESK_DIR).join(TODO_FILE)
}

fn normalize_markdown(text: &str) -> String {
    let mut text = text.replace("\r\n", "\n").replace('\r', "\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

fn ensure_todo_file(root: &Path) -> Result<PathBuf, String> {
    let path = todo_path(root);
    let parent = path
        .parent()
        .ok_or_else(|| "failed to resolve .catdesk directory".to_string())?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    if !path.exists() {
        fs::write(&path, DEFAULT_TODO_TEXT).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

fn parse_checkbox_line(line: &str) -> Option<(bool, &str)> {
    let trimmed = line.trim_start();
    for prefix in ["- [ ] ", "* [ ] ", "- [x] ", "- [X] ", "* [x] ", "* [X] "] {
        if let Some(task) = trimmed.strip_prefix(prefix) {
            return Some((prefix.contains('x') || prefix.contains('X'), task));
        }
    }
    None
}

fn parse_task_payload(payload: &str) -> Option<(String, String)> {
    let mut parts = payload.trim().splitn(2, char::is_whitespace);
    let id = parts.next()?.trim();
    if !valid_task_id(id) {
        return None;
    }
    let text = parts.next().unwrap_or("").trim();
    if text.is_empty() {
        return None;
    }
    Some((id.to_string(), text.to_string()))
}

fn valid_task_id(id: &str) -> bool {
    let Some(number) = id.strip_prefix("T-") else {
        return false;
    };
    number.len() == 4 && number.chars().all(|ch| ch.is_ascii_digit())
}

fn parse_tasks(text: &str) -> Result<Vec<TaskItem>, String> {
    let mut tasks = Vec::new();
    let mut seen = HashSet::new();
    for line in text.lines() {
        let Some((done, payload)) = parse_checkbox_line(line) else {
            continue;
        };
        let Some((id, task_text)) = parse_task_payload(payload) else {
            continue;
        };
        if !seen.insert(id.clone()) {
            return Err(format!("Duplicate task ID found in .catdesk/todo.md: {id}"));
        }
        tasks.push(TaskItem {
            id,
            done,
            text: task_text,
        });
    }
    Ok(tasks)
}

fn output_from_text(root: &Path, path: &Path, text: String) -> Result<TaskQueueOutput, String> {
    let tasks = parse_tasks(&text)?;
    let done = tasks.iter().filter(|task| task.done).count();
    Ok(TaskQueueOutput {
        path: to_workspace_relative(root, path),
        total: tasks.len(),
        open: tasks.len().saturating_sub(done),
        done,
        tasks,
        text,
    })
}

fn max_task_number(tasks: &[TaskItem]) -> u32 {
    tasks
        .iter()
        .filter_map(|task| task.id.strip_prefix("T-"))
        .filter_map(|number| number.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
}

fn next_task_number_from_marker(text: &str) -> Option<u32> {
    text.lines().find_map(|line| {
        let marker = line.trim().strip_prefix(NEXT_TASK_ID_MARKER_PREFIX)?;
        marker.strip_suffix("-->")?.trim().parse::<u32>().ok()
    })
}

fn next_task_id(tasks: &[TaskItem], text: &str) -> String {
    let marker_next = next_task_number_from_marker(text).unwrap_or(1);
    let next = marker_next.max(max_task_number(tasks).saturating_add(1));
    format!("T-{next:04}")
}

fn set_next_task_marker(text: &str, next_number: u32) -> String {
    let marker = format!("{NEXT_TASK_ID_MARKER_PREFIX}{next_number} -->");
    let mut found = false;
    let mut next = String::new();
    for line in normalize_markdown(text).lines() {
        if line.trim().starts_with(NEXT_TASK_ID_MARKER_PREFIX) {
            next.push_str(&marker);
            found = true;
        } else {
            next.push_str(line);
        }
        next.push('\n');
    }
    if !found {
        let mut text = normalize_markdown(&next);
        if !text.ends_with("\n\n") {
            text.push('\n');
        }
        text.push_str(&marker);
        text.push_str("\n\n");
        text
    } else {
        next
    }
}

pub fn read(workspace_root: &str) -> Result<TaskQueueOutput, String> {
    let root = workspace_root_path(workspace_root)?;
    let path = todo_path(&root);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DEFAULT_TODO_TEXT.to_string(),
        Err(e) => return Err(e.to_string()),
    };
    output_from_text(&root, &path, text)
}

pub fn add(workspace_root: &str, tasks: &[String]) -> Result<TaskQueueOutput, String> {
    let root = workspace_root_path(workspace_root)?;
    let path = ensure_todo_file(&root)?;
    let existing = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let marker_next = next_task_number_from_marker(&existing).unwrap_or(1);
    let mut text = set_next_task_marker(&existing, marker_next);
    let mut parsed = parse_tasks(&text)?;
    if !text.ends_with("\n\n") {
        text.push('\n');
    }
    for task in tasks
        .iter()
        .map(|task| task.trim())
        .filter(|task| !task.is_empty())
    {
        let id = next_task_id(&parsed, &text);
        text.push_str("- [ ] ");
        text.push_str(&id);
        text.push(' ');
        text.push_str(task);
        text.push('\n');
        parsed.push(TaskItem {
            id,
            done: false,
            text: task.to_string(),
        });
        text = set_next_task_marker(&text, max_task_number(&parsed).saturating_add(1));
    }
    fs::write(&path, &text).map_err(|e| e.to_string())?;
    output_from_text(&root, &path, text)
}

pub fn set_status(workspace_root: &str, id: &str, done: bool) -> Result<TaskQueueOutput, String> {
    let id = id.trim();
    if !valid_task_id(id) {
        return Err("task id must use the form T-0001".into());
    }
    let root = workspace_root_path(workspace_root)?;
    let path = ensure_todo_file(&root)?;
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut updated = false;
    let mut seen = HashSet::new();
    let mut next = String::new();
    for line in text.lines() {
        if let Some((_, payload)) = parse_checkbox_line(line) {
            if let Some((task_id, task_text)) = parse_task_payload(payload) {
                if !seen.insert(task_id.clone()) {
                    return Err(format!(
                        "Duplicate task ID found in .catdesk/todo.md: {task_id}"
                    ));
                }
                if task_id == id {
                    let indent_len = line.len().saturating_sub(line.trim_start().len());
                    next.push_str(&line[..indent_len]);
                    next.push_str(if done { "- [x] " } else { "- [ ] " });
                    next.push_str(&task_id);
                    next.push(' ');
                    next.push_str(&task_text);
                    next.push('\n');
                    updated = true;
                    continue;
                }
            }
        }
        next.push_str(line);
        next.push('\n');
    }
    if !updated {
        return Err(format!("No task found with id {id}"));
    }
    fs::write(&path, &next).map_err(|e| e.to_string())?;
    output_from_text(&root, &path, next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_workspace(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("catdesk-task-queue-{name}-{}", Uuid::new_v4()))
    }

    #[test]
    fn read_does_not_initialize_todo_file() {
        let workspace = test_workspace("read-no-init");
        fs::create_dir_all(&workspace).expect("create workspace");

        let output = read(&workspace.to_string_lossy()).expect("read tasks");

        assert_eq!(output.total, 0);
        assert!(!workspace.join(CATDESK_DIR).exists());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn add_and_complete_tasks_by_stable_id() {
        let workspace = test_workspace("todo");
        fs::create_dir_all(&workspace).expect("create workspace");

        let added = add(
            &workspace.to_string_lossy(),
            &[
                "Write task queue tests".to_string(),
                "Wire MCP tools".to_string(),
            ],
        )
        .expect("add tasks");
        assert_eq!(added.open, 2);
        assert!(added.text.contains("- [ ] T-0001 Write task queue tests"));

        let completed =
            set_status(&workspace.to_string_lossy(), "T-0001", true).expect("complete task");
        assert_eq!(completed.done, 1);
        assert!(
            completed
                .text
                .contains("- [x] T-0001 Write task queue tests")
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn add_uses_monotonic_task_marker_after_manual_deletion() {
        let workspace = test_workspace("monotonic");
        fs::create_dir_all(workspace.join(CATDESK_DIR)).expect("create memory dir");
        fs::write(
            workspace.join(CATDESK_DIR).join(TODO_FILE),
            "# Todo\n\n<!-- catdesk-next-task-id: 3 -->\n\n- [ ] T-0001 Remaining\n",
        )
        .expect("write todo");

        let added =
            add(&workspace.to_string_lossy(), &["Next task".to_string()]).expect("add task");
        assert!(added.text.contains("- [ ] T-0003 Next task"));
        assert!(added.text.contains("catdesk-next-task-id: 4"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn duplicate_task_ids_are_rejected() {
        let workspace = test_workspace("duplicate");
        fs::create_dir_all(workspace.join(CATDESK_DIR)).expect("create memory dir");
        fs::write(
            workspace.join(CATDESK_DIR).join(TODO_FILE),
            "# Todo\n\n- [ ] T-0001 First\n- [ ] T-0001 Second\n",
        )
        .expect("write todo");

        let error = read(&workspace.to_string_lossy()).expect_err("duplicate ID should fail");
        assert!(error.contains("Duplicate task ID"));

        let _ = fs::remove_dir_all(workspace);
    }
}
