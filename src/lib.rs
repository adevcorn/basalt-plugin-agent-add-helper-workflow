//! Add Helper workflow plugin.
//!
//! - `AGENT_LAUNCHER`: OpenCode-based workflow agent for the add-helper repo.
//! - `REVIEW_ACTIONS`: `gleam format --check`, `gleam test`, and `gleam build`
//!   gates.
//! - `DIAGNOSTICS`: warns when `models.gleam` and `Models.kt` (the JSON
//!   contract) drift apart.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use basalt_plugin_sdk::prelude::*;

basalt_plugin_meta! {
    name:              "add-helper-workflow",
    version:           "0.1.1",
    hook_flags:        CAP_AGENT_LAUNCHER | CAP_REVIEW_ACTIONS | CAP_DIAGNOSTICS | CAP_SHADOW_EXCLUDES,
    provides:          "agent:add-helper-workflow",
    requires:          "",
    optional_requires: "",
    file_globs:        "",
    activates_on:      "gleam.toml\n**/gleam.toml\nmodels.gleam\n**/models.gleam\nModels.kt\n**/Models.kt",
    activation_events: "",
}

/// Add-helper stores downloaded models, logs, and scratch data beside source.
/// Keep those out of every shadow; the marker check prevents this plugin from
/// claiming generic `data/`, `logs/`, or `models/` directories in unrelated
/// workspaces.
#[basalt_plugin]
fn shadow_excludes(root: &str) -> Vec<u8> {
    let add_helper = read_host_file(&join_path(root, "src/add_helper/models.gleam")).is_some()
        || read_host_file(&join_path(
            root,
            "android/app/src/main/java/com/adevcorn/addhelper/Models.kt",
        ))
        .is_some();
    if add_helper {
        br#"["**/data/**","**/logs/**","**/scratch/**","**/models/**"]"#.to_vec()
    } else {
        b"[]".to_vec()
    }
}

#[cfg(target_arch = "wasm32")]
extern "C" {
    fn basalt_read_file(path_ptr: i32, path_len: i32, out_ptr: i32, out_cap: i32) -> i32;
    fn basalt_list_files(root_ptr: i32, root_len: i32, out_ptr: i32, out_cap: i32) -> i32;
}

#[cfg(not(target_arch = "wasm32"))]
#[no_mangle]
unsafe extern "C" fn basalt_read_file(_path: i32, _len: i32, _out: i32, _cap: i32) -> i32 {
    -1
}

#[cfg(not(target_arch = "wasm32"))]
#[no_mangle]
unsafe extern "C" fn basalt_list_files(_root: i32, _len: i32, _out: i32, _cap: i32) -> i32 {
    -1
}

const FILE_BUF_SIZE: usize = 4 * 1024 * 1024;
const LIST_BUF_SIZE: usize = 4 * 1024 * 1024;

static mut FILE_BUF: [u8; FILE_BUF_SIZE] = [0u8; FILE_BUF_SIZE];
static mut LIST_BUF: [u8; LIST_BUF_SIZE] = [0u8; LIST_BUF_SIZE];

// ── Agent launcher ──────────────────────────────────────────────────────────

#[basalt_plugin]
fn agent_metadata() -> AgentMetadata {
    AgentMetadata {
        name: "Add Helper Workflow (OpenCode)".into(),
        executable: "opencode".into(),
        args: vec![
            "run".into(),
            "--format".into(),
            "json".into(),
            "--auto".into(),
            "{prompt}".into(),
        ],
        resume_new_args: vec![
            "run".into(),
            "--format".into(),
            "json".into(),
            "--auto".into(),
            "{prompt}".into(),
        ],
        resume_cont_args: vec![
            "run".into(),
            "--format".into(),
            "json".into(),
            "--auto".into(),
            "--session".into(),
            "{session_id}".into(),
            "{prompt}".into(),
        ],
        execution_tier: AgentExecutionTier::StructuredDirect,
        workspace_capabilities: vec!["mcp".into(), "shadow".into(), "config:opencode".into()],
        protocol: AgentProtocol::Cli,
    }
}

// ── Review gates ────────────────────────────────────────────────────────────

#[basalt_plugin]
fn review_actions(workspace_root: &str, _session_workspace: &str) -> Vec<ReviewActionDescriptor> {
    if read_host_file(&join_path(workspace_root, "gleam.toml")).is_none() {
        return Vec::new();
    }
    vec![
        ReviewActionDescriptor {
            id: "add-helper-format-check".into(),
            title: "Gate: gleam format --check".into(),
            kind: ReviewActionKind::FormatCheck,
            ecosystem: "gleam".into(),
            command_preview: "gleam format --check src test".into(),
            mutates_workspace: false,
            priority: 200,
        },
        ReviewActionDescriptor {
            id: "add-helper-test".into(),
            title: "Gate: gleam test".into(),
            kind: ReviewActionKind::Test,
            ecosystem: "gleam".into(),
            command_preview: "gleam test".into(),
            mutates_workspace: false,
            priority: 200,
        },
        ReviewActionDescriptor {
            id: "add-helper-build".into(),
            title: "Gate: gleam build".into(),
            kind: ReviewActionKind::Build,
            ecosystem: "gleam".into(),
            command_preview: "gleam build".into(),
            mutates_workspace: false,
            priority: 200,
        },
    ]
}

#[basalt_plugin]
fn review_action_plan(
    action_id: &str,
    _workspace_root: &str,
    _session_workspace: &str,
) -> Option<ReviewActionExecutionPlan> {
    match action_id {
        "add-helper-format-check" => Some(ReviewActionExecutionPlan {
            executable: "gleam".into(),
            args: vec![
                "format".into(),
                "--check".into(),
                "src".into(),
                "test".into(),
            ],
            env: Vec::new(),
            cwd_mode: ReviewActionCwdMode::SessionWorkspace,
            output_category: "format".into(),
        }),
        "add-helper-test" => Some(ReviewActionExecutionPlan {
            executable: "gleam".into(),
            args: vec!["test".into()],
            env: Vec::new(),
            cwd_mode: ReviewActionCwdMode::SessionWorkspace,
            output_category: "test".into(),
        }),
        "add-helper-build" => Some(ReviewActionExecutionPlan {
            executable: "gleam".into(),
            args: vec!["build".into()],
            env: Vec::new(),
            cwd_mode: ReviewActionCwdMode::SessionWorkspace,
            output_category: "build".into(),
        }),
        _ => None,
    }
}

#[basalt_plugin]
fn review_action_parse_line(
    action_id: &str,
    line: &[u8],
    state: &[u8],
) -> (Vec<u8>, Vec<AgentEvent>) {
    let (mut counter, mut pending_severity, mut pending_message) = read_state(state);
    let Ok(line_str) = core::str::from_utf8(line) else {
        return (
            encode_state(counter, pending_severity, &pending_message),
            Vec::new(),
        );
    };
    let line_str = strip_ansi(line_str);
    let trimmed = line_str.trim();
    if trimmed.is_empty() {
        return (
            encode_state(counter, pending_severity, &pending_message),
            Vec::new(),
        );
    }

    let mut events = Vec::new();

    if action_id != "add-helper-format-check" {
        if trimmed.starts_with("error: ") || trimmed.starts_with("warning: ") {
            pending_severity = if trimmed.starts_with("error: ") { 1 } else { 2 };
            pending_message = trimmed.to_string();
            return (
                encode_state(counter, pending_severity, &pending_message),
                events,
            );
        }

        if trimmed.starts_with('┌') {
            if let Some((_, location)) = trimmed.split_once('─') {
                let location = location.trim();
                if let Some((path, line_no, col)) = split_location(location) {
                    let message = if pending_message.is_empty() {
                        location.to_string()
                    } else {
                        pending_message.clone()
                    };
                    events.push(AgentEvent::NewEntry {
                        vendor_id: format!("add-helper-{counter}"),
                        tool: "gleam".into(),
                        category: if pending_severity == 2 {
                            "diagnostic".into()
                        } else {
                            "build".into()
                        },
                        raw_cmd: diagnostic_json(
                            &path,
                            line_no,
                            col,
                            if pending_severity == 2 {
                                "warning"
                            } else {
                                "error"
                            },
                            &message,
                        ),
                        file_paths: Vec::new(),
                    });
                    counter += 1;
                    pending_severity = 0;
                    pending_message.clear();
                }
            }
        }
    }

    if action_id == "add-helper-test" {
        if let Some((passed, failures)) = parse_gleeunit_summary(trimmed) {
            events.push(AgentEvent::CloseEntry {
                vendor_id: format!("add-helper-test-{counter}"),
                exit_code: if failures > 0 { 1 } else { 0 },
                output_lines: vec![format!("{passed} passed, {failures} failures")],
            });
            counter += 1;
        }
    }

    if action_id == "add-helper-format-check" && trimmed.starts_with("- ") {
        events.push(AgentEvent::NewEntry {
            vendor_id: format!("add-helper-{counter}"),
            tool: "gleam format".into(),
            category: "format".into(),
            raw_cmd: trimmed.to_string(),
            file_paths: vec![trimmed[2..].trim().replace('\\', "/")],
        });
        counter += 1;
    }

    (
        encode_state(counter, pending_severity, &pending_message),
        events,
    )
}

/// `DiagnosticItem` JSON understood by `basalt_core::build_runner`.
fn diagnostic_json(file: &str, line: u32, col: u32, severity: &str, message: &str) -> String {
    format!(
        "{{\"file\":\"{}\",\"line\":{},\"col\":{},\"severity\":\"{}\",\"message\":\"{}\"}}",
        escape_json(file),
        line,
        col,
        severity,
        escape_json(message),
    )
}

/// Parser state: counter + pending severity (`0` none, `1` error, `2` warning)
/// + pending message waiting for its `┌─ path:line:col` line.
fn read_state(state: &[u8]) -> (u32, u8, String) {
    if state.len() < 7 {
        return (0, 0, String::new());
    }
    let counter = u32::from_le_bytes([state[0], state[1], state[2], state[3]]);
    let severity = state[4];
    let len = u16::from_le_bytes([state[5], state[6]]) as usize;
    let end = (7 + len).min(state.len());
    (
        counter,
        severity,
        String::from_utf8_lossy(&state[7..end]).into_owned(),
    )
}

fn encode_state(counter: u32, severity: u8, message: &str) -> Vec<u8> {
    let bytes = message.as_bytes();
    let len = bytes.len().min(u16::MAX as usize) as u16;
    let mut out = Vec::with_capacity(7 + len as usize);
    out.extend_from_slice(&counter.to_le_bytes());
    out.push(severity);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&bytes[..len as usize]);
    out
}

// ── JSON-contract drift diagnostics ─────────────────────────────────────────

#[basalt_plugin]
fn diagnose(src: &[u8], path: &str) -> Vec<Diagnostic> {
    let name = file_name(path);
    let Ok(text) = core::str::from_utf8(src) else {
        return Vec::new();
    };

    let warning = if name.eq_ignore_ascii_case("models.gleam") {
        find_counterpart(path, "Models.kt")
            .and_then(|path| read_host_file(&path))
            .and_then(|kotlin| contract_drift(text, &kotlin))
    } else if name.eq_ignore_ascii_case("Models.kt") {
        find_counterpart(path, "models.gleam")
            .and_then(|path| read_host_file(&path))
            .and_then(|gleam| contract_drift(&gleam, text))
    } else {
        None
    };

    match warning {
        Some(message) => vec![Diagnostic::new(0, 1, message, Severity::Warning)],
        None => Vec::new(),
    }
}

/// Compare Gleam and Kotlin model types field-by-field.
///
/// Type names and field names are normalised (lowercase, underscores removed)
/// so `eventId` / `event_id` compare equal.
fn contract_drift(gleam_src: &str, kotlin_src: &str) -> Option<String> {
    let gleam = parse_gleam_types(gleam_src);
    let kotlin = parse_kotlin_types(kotlin_src);
    if gleam.is_empty() || kotlin.is_empty() {
        return None;
    }

    let mut messages: Vec<String> = Vec::new();
    for (name, fields) in &gleam {
        let key = normalize_name(name);
        // Models.kt mirrors only the subset of the Gleam model that crosses
        // the JSON boundary. Missing Kotlin types are not drift by themselves.
        let Some(kotlin_fields) = kotlin.get(&key) else {
            continue;
        };
        let missing: Vec<&String> = fields
            .iter()
            .filter(|field| !kotlin_fields.contains(*field))
            .collect();
        let extra: Vec<&String> = kotlin_fields
            .iter()
            .filter(|field| !fields.contains(*field))
            .collect();
        if !missing.is_empty() || !extra.is_empty() {
            let mut detail = format!("type `{name}`");
            if !missing.is_empty() {
                detail.push_str(&format!(" missing in Models.kt: {missing:?}"));
            }
            if !extra.is_empty() {
                detail.push_str(&format!(" extra in Models.kt: {extra:?}"));
            }
            messages.push(detail);
        }
    }

    for (name, _) in &kotlin {
        if !gleam.keys().any(|key| normalize_name(key) == *name) {
            messages.push(format!("type `{name}` is missing from models.gleam"));
        }
    }

    if messages.is_empty() {
        None
    } else {
        Some(format!(
            "add-helper JSON contract drift — {}",
            messages.join("; ")
        ))
    }
}

fn parse_gleam_types(src: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut out = std::collections::BTreeMap::new();
    let mut rest = src;
    while let Some(idx) = rest.find("pub type ") {
        let after = &rest[idx + "pub type ".len()..];
        let name: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            rest = after;
            continue;
        }
        let Some(body_start) = after.find('{') else {
            break;
        };
        let body = &after[body_start + 1..];
        let body_end = body.find("\n}").unwrap_or(body.len());
        let block = &body[..body_end];

        let mut fields = Vec::new();
        for chunk in block.split(|c: char| c == ',' || c == '\n') {
            let Some(colon) = chunk.find(':') else {
                continue;
            };
            let candidate = chunk[..colon]
                .rsplit(|c: char| c == '(' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .trim();
            if !candidate.is_empty()
                && candidate
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                fields.push(normalize_name(candidate));
            }
        }
        fields.sort();
        fields.dedup();
        out.insert(name, fields);
        rest = &body[body_end..];
    }
    out
}

fn parse_kotlin_types(src: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut out = std::collections::BTreeMap::new();
    let mut rest = src;
    while let Some(pos) = rest.find("data class ") {
        let after = &rest[pos + "data class ".len()..];
        let name: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            break;
        }
        // Only the primary constructor participates in the JSON contract.
        let Some(open) = after.find('(') else {
            break;
        };
        let Some(close) = matching_paren(after, open) else {
            break;
        };
        let params = &after[open + 1..close];

        let mut fields = Vec::new();
        for chunk in params.split(|c: char| c == ',' || c == '\n') {
            let Some(colon) = chunk.find(':') else {
                continue;
            };
            let left = &chunk[..colon];
            let Some(val_pos) = left.find("val ").or_else(|| left.find("var ")) else {
                continue;
            };
            let field: String = left[val_pos + 4..]
                .trim()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !field.is_empty() {
                fields.push(normalize_name(&field));
            }
        }
        fields.sort();
        fields.dedup();
        out.insert(normalize_name(&name), fields);
        rest = &after[close + 1..];
    }
    out
}

/// Index of the `)` matching the `(` at `open`.
fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (idx, ch) in text.char_indices().skip(open) {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '_')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn find_counterpart(path: &str, target: &str) -> Option<String> {
    let root = project_root(path);
    for candidate in [
        format!("{root}/android/app/src/main/java/com/adevcorn/addhelper/{target}"),
        format!("{root}/src/add_helper/{target}"),
        format!("{root}/android/app/src/main/kotlin/com/adevcorn/addhelper/{target}"),
    ] {
        if read_host_file(&candidate).is_some() {
            return Some(candidate);
        }
    }
    for rel in list_host_files(&root) {
        let rel = rel.replace('\\', "/");
        if file_name(&rel) == target {
            return Some(join_path(&root, &rel));
        }
    }
    None
}

fn project_root(path: &str) -> String {
    let mut dir = parent_dir(path);
    loop {
        if dir.is_empty() {
            break;
        }
        if read_host_file(&join_path(&dir, "gleam.toml")).is_some() {
            return dir;
        }
        let parent = parent_dir(&dir);
        if parent == dir {
            break;
        }
        dir = parent;
    }
    parent_dir(path)
}

// ── Host I/O ────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn read_host_file(path: &str) -> Option<String> {
    let buf: &mut [u8; FILE_BUF_SIZE] = unsafe { &mut *core::ptr::addr_of_mut!(FILE_BUF) };
    let n = unsafe {
        basalt_read_file(
            path.as_ptr() as i32,
            path.len() as i32,
            buf.as_mut_ptr() as i32,
            FILE_BUF_SIZE as i32,
        )
    };
    if n <= 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_host_file(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

#[cfg(target_arch = "wasm32")]
fn list_host_files(root: &str) -> Vec<String> {
    let buf: &mut [u8; LIST_BUF_SIZE] = unsafe { &mut *core::ptr::addr_of_mut!(LIST_BUF) };
    let n = unsafe {
        basalt_list_files(
            root.as_ptr() as i32,
            root.len() as i32,
            buf.as_mut_ptr() as i32,
            LIST_BUF_SIZE as i32,
        )
    };
    if n <= 0 {
        return Vec::new();
    }
    String::from_utf8_lossy(&buf[..n as usize])
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn list_host_files(root: &str) -> Vec<String> {
    fn walk(base: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>, depth: usize) {
        if depth > 8 || out.len() > 20_000 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if matches!(
                name.as_str(),
                ".git" | ".basalt" | "build" | ".gradle" | "node_modules" | "target"
            ) {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out, depth + 1);
            } else if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }

    let base = std::path::Path::new(root);
    let mut out = Vec::new();
    walk(base, base, &mut out, 0);
    out
}

// ── Small helpers ───────────────────────────────────────────────────────────

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn split_location(location: &str) -> Option<(String, u32, u32)> {
    let mut parts = location.rsplitn(3, ':');
    let col: u32 = parts.next()?.trim().parse().ok()?;
    let line: u32 = parts.next()?.trim().parse().ok()?;
    let path = parts.next()?.trim();
    if path.is_empty() {
        None
    } else {
        Some((path.to_string(), line, col))
    }
}

fn parse_gleeunit_summary(line: &str) -> Option<(u32, u32)> {
    let (passed_part, rest) = line.split_once(" passed, ")?;
    let passed: u32 = passed_part
        .rsplit(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    let failures = if rest.starts_with("no failures") {
        0
    } else {
        rest.split_whitespace().next()?.parse().ok()?
    };
    Some((passed, failures))
}

fn parent_dir(path: &str) -> String {
    path.rsplit_once(['/', '\\'])
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn join_path(root: &str, suffix: &str) -> String {
    if root.ends_with('/') || root.ends_with('\\') {
        format!("{root}{suffix}")
    } else {
        format!("{root}/{suffix}")
    }
}

fn escape_json(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLEAM_MODELS: &str = r#"
pub type CalendarEvent {
  CalendarEvent(
    id: String,
    title: String,
    start_time: Int,
    attendees: List(Attendee),
  )
}

pub type Attendee {
  Attendee(name: String, email: String)
}
"#;

    const KOTLIN_MODELS: &str = r#"
@Serializable
data class CalendarEvent(
    val id: String,
    val title: String,
    @SerialName("start_time") val startTime: Long,
    val attendees: List<Attendee>,
)

data class Attendee(
    val name: String,
    val email: String,
)
"#;

    #[test]
    fn matching_contracts_do_not_drift() {
        assert_eq!(contract_drift(GLEAM_MODELS, KOTLIN_MODELS), None);
    }

    #[test]
    fn drift_reports_missing_and_extra_fields() {
        let kotlin = r#"
data class CalendarEvent(
    val id: String,
    val title: String,
    val startTime: Long,
    val location: String,
)
"#;
        let message = contract_drift(GLEAM_MODELS, kotlin).expect("drift");
        assert!(message.contains("missing in Models.kt"));
        assert!(message.contains("attendees"));
        assert!(message.contains("extra in Models.kt"));
        assert!(message.contains("location"));
    }

    #[test]
    fn gleam_only_types_are_not_reported() {
        // Models.kt mirrors a subset of models.gleam; only overlapping types
        // participate in the contract check.
        let kotlin = "data class Attendee(val name: String, val email: String)\n";
        assert_eq!(contract_drift(GLEAM_MODELS, kotlin), None);
    }

    #[test]
    fn normalisation_ignores_case_and_underscores() {
        assert_eq!(normalize_name("start_time"), normalize_name("startTime"));
        assert_eq!(normalize_name("Event_ID"), "eventid");
    }

    #[test]
    fn agent_metadata_targets_the_workflow() {
        let meta = agent_metadata();
        assert_eq!(meta.executable, "opencode");
        assert!(meta.args.iter().any(|arg| arg == "{prompt}"));
        assert_eq!(meta.execution_tier, AgentExecutionTier::StructuredDirect);
    }

    #[test]
    fn review_gates_are_ordered_before_generic_actions() {
        let ws = temp_workspace(&[("gleam.toml", "name = \"add_helper\"\nversion = \"1.0.0\"\n")]);
        let actions = review_actions(ws.to_str().unwrap(), "/ws/.shadow");
        assert_eq!(actions.len(), 3);
        let ids: Vec<&str> = actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "add-helper-format-check",
                "add-helper-test",
                "add-helper-build"
            ]
        );
        assert!(actions.iter().all(|action| action.priority == 200));
        assert!(review_actions("/definitely/not/add-helper", "/tmp").is_empty());
        let _ = std::fs::remove_dir_all(&ws);
    }

    fn temp_workspace(files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "add-helper-workflow-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, contents) in files {
            std::fs::write(dir.join(name), contents).unwrap();
        }
        dir
    }

    #[test]
    fn review_plans_run_gleam_gates() {
        let plan = review_action_plan("add-helper-format-check", "/ws", "/ws").unwrap();
        assert_eq!(plan.executable, "gleam");
        assert_eq!(
            plan.args,
            vec!["format", "--check", "src", "test"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
        assert!(review_action_plan("unknown", "/ws", "/ws").is_none());
    }

    #[test]
    fn format_failures_are_listed() {
        let (_, events) =
            review_action_parse_line("add-helper-format-check", b"  - src\\add_helper.gleam", &[]);
        assert!(matches!(
            &events[0],
            AgentEvent::NewEntry { category, file_paths, .. }
                if category == "format" && file_paths[0] == "src/add_helper.gleam"
        ));
    }

    #[test]
    fn gleeunit_summary_sets_exit_code() {
        let (_, fail) = review_action_parse_line("add-helper-test", b"0 passed, 1 failures", &[]);
        assert!(matches!(
            &fail[0],
            AgentEvent::CloseEntry { exit_code: 1, .. }
        ));
        let (_, ok) = review_action_parse_line("add-helper-test", b"2 passed, no failures", &[]);
        assert!(matches!(
            &ok[0],
            AgentEvent::CloseEntry { exit_code: 0, .. }
        ));
    }

    #[test]
    fn build_errors_emit_json_diagnostics() {
        let (state, header) =
            review_action_parse_line("add-helper-build", b"error: Unknown variable", &[]);
        assert!(header.is_empty());
        let (_, events) = review_action_parse_line(
            "add-helper-build",
            b"  \xe2\x94\x8c\xe2\x94\x80 C:\\ws\\src\\add_helper.gleam:3:5",
            &state,
        );
        assert!(matches!(
            &events[0],
            AgentEvent::NewEntry { raw_cmd, .. }
                if raw_cmd.contains("\"line\":3")
                    && raw_cmd.contains("add_helper.gleam")
                    && raw_cmd.contains("Unknown variable")
        ));
    }

    #[test]
    #[ignore = "requires ../../add-helper"]
    fn real_add_helper_models_can_be_compared() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../add-helper");
        let gleam = std::fs::read_to_string(base.join("src/add_helper/models.gleam"))
            .expect("read models.gleam");
        let kotlin = std::fs::read_to_string(
            base.join("android/app/src/main/java/com/adevcorn/addhelper/Models.kt"),
        )
        .expect("read Models.kt");
        let gleam_types = parse_gleam_types(&gleam);
        let kotlin_types = parse_kotlin_types(&kotlin);
        assert!(!gleam_types.is_empty(), "no Gleam types parsed");
        assert!(!kotlin_types.is_empty(), "no Kotlin types parsed");
        println!(
            "gleam types: {:?}\nkotlin types: {:?}\ndrift: {:?}",
            gleam_types.keys().collect::<Vec<_>>(),
            kotlin_types.keys().collect::<Vec<_>>(),
            contract_drift(&gleam, &kotlin)
        );
    }
}
