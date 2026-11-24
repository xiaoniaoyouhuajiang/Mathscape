use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use crate::codex::approval::{MATHSCAPE_READ_ONLY_TOOLS, enabled_tools};
use crate::{ApprovalDecision, SessionSpec};

pub(crate) fn tool_names(allow: &[String]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    let mut collisions = HashSet::new();
    for name in MATHSCAPE_READ_ONLY_TOOLS
        .iter()
        .chain(
            [
                "artifact.stage",
                "propose_changes",
                "get_change_set",
                "list_change_sets",
                "user.ask",
            ]
            .iter(),
        )
        .map(|s| s.to_string())
        .chain(allow.iter().cloned())
    {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            continue;
        }
        let title = format!("mathscape_{}", name.replace('.', "_"));
        if names.get(&title).is_some_and(|existing| existing != &name) {
            collisions.insert(title.clone());
        }
        names.insert(title, name);
    }
    for title in collisions {
        names.remove(&title);
    }
    names
}

pub(crate) fn permitted(spec: &SessionSpec, params: &Value, decision: ApprovalDecision) -> bool {
    if decision == ApprovalDecision::Decline {
        return false;
    }
    let call = &params["toolCall"];
    let title = call["title"].as_str().unwrap_or("");
    let kind = call["kind"].as_str().unwrap_or("");
    if kind == "execute" || matches!(title, "bash" | "shell" | "webfetch" | "websearch") {
        return false;
    }
    if title.starts_with("mathscape_") {
        return tool_names(&spec.tools_allow)
            .get(title)
            .is_some_and(|tool| enabled_tools(&spec.tools_allow).contains(tool));
    }
    let input = &call["rawInput"];
    let write = matches!(kind, "edit" | "delete" | "move")
        || matches!(title, "write" | "edit" | "delete" | "move" | "apply_patch");
    let read = matches!(kind, "read" | "search")
        || matches!(title, "read" | "glob" | "grep" | "list")
        || (kind == "other" && input.get("parentDir").is_some() && input.get("filepath").is_some());
    if !read && !write {
        return false;
    }
    let mut paths = Vec::new();
    let mut valid = true;
    for key in ["filepath", "filePath", "path", "parentDir"] {
        if let Some(value) = input.get(key) {
            collect_paths(value, &mut paths, &mut valid);
        }
    }
    if let Some(locations) = call.get("locations") {
        if let Some(locations) = locations.as_array() {
            for location in locations {
                if let Some(path) = location.get("path") {
                    collect_paths(path, &mut paths, &mut valid);
                } else {
                    valid = false;
                }
            }
        } else {
            valid = false;
        }
    }
    if !valid || paths.is_empty() {
        return false;
    }
    let roots = if write {
        spec.harness.scratch_dir.iter().cloned().collect::<Vec<_>>()
    } else {
        std::iter::once(spec.workspace.clone())
            .chain(spec.harness.read_roots.iter().cloned())
            .chain(spec.harness.skill_roots.iter().cloned())
            .chain(spec.harness.scratch_dir.iter().cloned())
            .collect()
    };
    paths
        .iter()
        .all(|path| within(path, &spec.workspace, &roots))
}

fn collect_paths<'a>(value: &'a Value, paths: &mut Vec<&'a str>, valid: &mut bool) {
    match value {
        Value::String(path) if !path.is_empty() => paths.push(path),
        Value::Array(values) if !values.is_empty() => {
            for value in values {
                collect_paths(value, paths, valid);
            }
        }
        _ => *valid = false,
    }
}

pub(crate) fn resolve(path: &Path, cwd: &Path) -> std::io::Result<PathBuf> {
    if path.components().any(|p| p == Component::ParentDir) {
        return Err(std::io::Error::other("parent components are not accepted"));
    }
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let mut ancestor = path.as_path();
    let mut suffix = Vec::new();
    loop {
        match std::fs::canonicalize(ancestor) {
            Ok(mut real) => {
                for name in suffix.iter().rev() {
                    real.push(name);
                }
                return Ok(real);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A dangling symlink is not a nonexistent filename we may append.
                if std::fs::symlink_metadata(ancestor).is_ok() {
                    return Err(error);
                }
                let Some(name) = ancestor.file_name() else {
                    return Err(error);
                };
                suffix.push(name.to_os_string());
                ancestor = ancestor.parent().ok_or(error)?;
            }
            Err(error) => return Err(error),
        }
    }
}

fn within(path: &str, cwd: &Path, roots: &[PathBuf]) -> bool {
    let Ok(real) = resolve(Path::new(path), cwd) else {
        return false;
    };
    roots
        .iter()
        .filter_map(|root| resolve(root, cwd).ok())
        .any(|root| real.starts_with(root))
}

pub(crate) fn reply(params: &Value, allow: bool) -> Value {
    let kind = if allow { "allow_once" } else { "reject_once" };
    let option = params["options"]
        .as_array()
        .and_then(|options| options.iter().find(|o| o["kind"] == kind));
    let outcome = option
        .and_then(|o| o.get("optionId"))
        .map(|id| json!({"outcome":"selected","optionId":id}))
        .unwrap_or_else(|| json!({"outcome":"cancelled"}));
    json!({"outcome":outcome})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentBackend, HarnessContext};

    fn spec(root: &Path) -> SessionSpec {
        SessionSpec {
            backend: AgentBackend::OpenCode,
            session_key: "policy".into(),
            workspace: root.join("workspace"),
            tools_allow: vec![],
            harness: HarnessContext {
                scratch_dir: Some(root.join("workspace/scratch")),
                ..Default::default()
            },
            round_file: None,
            ephemeral: false,
        }
    }
    fn request(kind: &str, path: &Path) -> Value {
        json!({"toolCall":{"kind":kind,"rawInput":{"filepath":path},"locations":[{"path":path}]}})
    }
    #[test]
    fn paths_and_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("workspace/scratch")).unwrap();
        let spec = spec(root.path());
        let allow =
            |kind, path: &Path| permitted(&spec, &request(kind, path), ApprovalDecision::Policy);
        assert!(allow("read", Path::new("new.txt")));
        assert!(allow("edit", Path::new("scratch/new/deep.txt")));
        assert!(!allow("edit", Path::new("workspace.txt")));
        assert!(!allow("read", Path::new("scratch/../new.txt")));
        assert!(!allow("read", &root.path().join("outside")));
        assert!(!allow("execute", Path::new("scratch/a")));
        assert!(!allow("other", Path::new("scratch/a")));
        assert!(!permitted(
            &spec,
            &json!({"toolCall":{"kind":"read"}}),
            ApprovalDecision::Policy
        ));
        assert!(!permitted(
            &spec,
            &request("read", Path::new("a")),
            ApprovalDecision::Decline
        ));
        let mut multiple = request("read", Path::new("a"));
        multiple["toolCall"]["locations"] =
            json!([{"path":"a"},{"path":root.path().join("outside")}]);
        assert!(!permitted(&spec, &multiple, ApprovalDecision::Policy));
        assert!(!permitted(
            &spec,
            &json!({"toolCall":{"kind":"read","rawInput":{"path":[]}}}),
            ApprovalDecision::Policy
        ));
        assert_eq!(
            reply(
                &json!({"options":[{"kind":"allow_always","optionId":"always"}]}),
                true
            ),
            json!({"outcome":{"outcome":"cancelled"}})
        );
    }
    #[test]
    fn explicit_read_roots_and_mathscape() {
        let root = tempfile::tempdir().unwrap();
        for name in ["workspace/scratch", "reads", "skills"] {
            std::fs::create_dir_all(root.path().join(name)).unwrap();
        }
        let mut spec = spec(root.path());
        spec.harness.read_roots.push(root.path().join("reads"));
        spec.harness.skill_roots.push(root.path().join("skills"));
        for name in ["reads/a", "skills/a"] {
            assert!(permitted(
                &spec,
                &request("read", &root.path().join(name)),
                ApprovalDecision::Policy
            ));
            assert!(!permitted(
                &spec,
                &request("edit", &root.path().join(name)),
                ApprovalDecision::Policy
            ));
        }
        assert!(permitted(
            &spec,
            &json!({"toolCall":{"title":"mathscape_workspace_search"}}),
            ApprovalDecision::Policy
        ));
        assert!(!permitted(
            &spec,
            &json!({"toolCall":{"title":"mathscape_artifact_stage"}}),
            ApprovalDecision::Policy
        ));
        assert!(!tool_names(&["a_b.c".into(), "a.b_c".into()]).contains_key("mathscape_a_b_c"));
        assert_eq!(
            tool_names(&[])["mathscape_get_change_set"],
            "get_change_set"
        );
        assert!(!tool_names(&["*".into()]).contains_key("mathscape_*"));
    }
    #[cfg(unix)]
    #[test]
    fn canonical_symlinks_including_root_alias() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("workspace/scratch")).unwrap();
        std::fs::create_dir(root.path().join("outside")).unwrap();
        std::fs::write(root.path().join("outside/file"), "secret").unwrap();
        symlink(
            root.path().join("outside/file"),
            root.path().join("workspace/scratch/file"),
        )
        .unwrap();
        symlink(
            root.path().join("outside"),
            root.path().join("workspace/scratch/dir"),
        )
        .unwrap();
        symlink(
            root.path().join("missing"),
            root.path().join("workspace/scratch/dangling"),
        )
        .unwrap();
        let spec = spec(root.path());
        for path in [
            "scratch/file",
            "scratch/dir/file",
            "scratch/dir/nonexistent/deep",
            "scratch/dangling",
        ] {
            for kind in ["read", "edit"] {
                assert!(
                    !permitted(
                        &spec,
                        &request(kind, Path::new(path)),
                        ApprovalDecision::Policy
                    ),
                    "{kind} {path}"
                );
            }
        }
        symlink(root.path().join("workspace"), root.path().join("alias")).unwrap();
        let mut alias = spec.clone();
        alias.workspace = root.path().join("alias");
        alias.harness.scratch_dir = Some(root.path().join("alias/scratch"));
        assert!(permitted(
            &alias,
            &request("edit", &root.path().join("workspace/scratch/new")),
            ApprovalDecision::Policy
        ));
    }
}
