//! The versioned Mathscape skill pack (`skills/` at the repository root).
//!
//! Sandbox rounds register the skills root with the Codex skill loader
//! (`skills/extraRoots/set`) and also name the stable path, version and
//! content hash in developer instructions, so the agent can read the files
//! with its own (read-only) file tools even if the loader is unavailable.
//! The "read SKILL.md first" instruction is turn context on a thread's first
//! turn only ([`SkillPack::first_turn_note`]); a resumed thread whose pack
//! changed gets [`SkillPack::update_note`]. Each round records the same
//! identity in the session's `harness.jsonl`.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

/// Override for the skills root (the directory that *contains* skill dirs).
pub const SKILLS_ROOT_ENV: &str = "MATHSCAPE_SKILLS_ROOT";
/// The skill the sandbox injects.
pub const WORKNET_SKILL: &str = "mathscape-worknet";

/// Repository `skills/` as seen at build time (development builds).
const BUILD_TIME_SKILLS_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../skills");

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillPack {
    /// Skills root registered with the harness.
    pub root: PathBuf,
    /// Directory of [`WORKNET_SKILL`].
    pub dir: PathBuf,
    pub name: String,
    pub version: Option<String>,
    pub tool_schema_version: Option<String>,
    /// sha256 over every file (sorted relative path, length, bytes).
    pub sha256: String,
    pub files: usize,
}

/// First existing of `$MATHSCAPE_SKILLS_ROOT`, the build-time repository
/// `skills/`, and `skills/` beside or above the executable (bundles).
pub fn locate_skills_root() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(root) = env::var_os(SKILLS_ROOT_ENV).filter(|v| !v.is_empty()) {
        candidates.push(PathBuf::from(root));
    }
    candidates.push(PathBuf::from(BUILD_TIME_SKILLS_ROOT));
    if let Some(dir) = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        candidates.push(dir.join("skills"));
        candidates.push(dir.join("../Resources/skills"));
    }
    candidates
        .into_iter()
        .find(|root| root.join(WORKNET_SKILL).join("SKILL.md").is_file())
        .map(|root| fs::canonicalize(&root).unwrap_or(root))
}

pub fn load_worknet_skill(root: &Path) -> Result<SkillPack, String> {
    let dir = root.join(WORKNET_SKILL);
    let skill_md = dir.join("SKILL.md");
    let text = fs::read_to_string(&skill_md)
        .map_err(|error| format!("无法读取 {}：{error}", skill_md.display()))?;
    let front =
        frontmatter(&text).ok_or_else(|| format!("{} 缺少 frontmatter", skill_md.display()))?;
    let name = field(front, "name").ok_or_else(|| "skill frontmatter 缺少 name".to_owned())?;
    let mut files = Vec::new();
    collect_files(&dir, &dir, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for relative in &files {
        let bytes = fs::read(dir.join(relative))
            .map_err(|error| format!("无法读取 skill 文件 {relative}：{error}"))?;
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    let sha256 = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(SkillPack {
        root: root.to_path_buf(),
        dir,
        name,
        version: field(front, "version"),
        tool_schema_version: field(front, "tool_schema_version"),
        sha256,
        files: files.len(),
    })
}

impl SkillPack {
    pub fn short_hash(&self) -> &str {
        &self.sha256[..12.min(self.sha256.len())]
    }

    /// Developer-instruction paragraph: identity and stable path only. It is
    /// part of the thread's cached prefix, so it holds nothing per-turn.
    pub fn instructions(&self) -> String {
        let version = self.version.as_deref().unwrap_or("unversioned");
        format!(
            "Mathscape skill `{name}` {version} (sha256 {hash}) is registered with your skill loader and lives read-only at {dir}. Do not modify the skill directory.",
            name = self.name,
            hash = self.short_hash(),
            dir = self.dir.display(),
        )
    }

    /// Turn context for a thread's first turn: how to start reading.
    pub fn first_turn_note(&self) -> String {
        let dir = self.dir.display();
        format!(
            "Before building anything, read {dir}/SKILL.md and follow it; it points to the references you need (start with references/reader-primer.md and references/runtime-capabilities.md, whose sandbox section describes this session). \
Read large references such as references/rdtx_try.md only in the parts you need."
        )
    }

    /// Turn context after a resume when the pack (or the tool schema it
    /// targets) changed since the thread's last turn.
    pub fn update_note(
        &self,
        previous_version: Option<&str>,
        previous_tool_schema: Option<&str>,
    ) -> String {
        let version = self.version.as_deref().unwrap_or("unversioned");
        let tools = self.tool_schema_version.as_deref().unwrap_or("unknown");
        let dir = self.dir.display();
        format!(
            "工具/skill 已更新到 {name} {version}（tool_schema_version {tools}，sha256 {hash}；本对话上一轮用的是 {prev} / {prev_tools}）。请重读 {dir}/SKILL.md 与 references/runtime-capabilities.md 中变化的部分；之前轮次里关于工具的结论以新版本为准。",
            name = self.name,
            hash = self.short_hash(),
            prev = previous_version.unwrap_or("未知版本"),
            prev_tools = previous_tool_schema.unwrap_or("未知"),
        )
    }
}

fn frontmatter(text: &str) -> Option<&str> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

/// `key: value` anywhere in the frontmatter (top level or under `metadata:`).
fn field(front: &str, key: &str) -> Option<String> {
    front.lines().find_map(|line| {
        let (name, value) = line.trim().split_once(':')?;
        (name.trim() == key)
            .then(|| value.trim().trim_matches('"').trim_matches('\'').to_owned())
            .filter(|value| !value.is_empty())
    })
}

fn collect_files(base: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|error| format!("无法列出 {}：{error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // macOS metadata (`._*`, `.DS_Store`) is not part of the pack.
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_dir() {
            collect_files(base, &path, out)?;
        } else if kind.is_file() {
            let relative = path
                .strip_prefix(base)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            out.push(relative);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_pack() -> SkillPack {
        load_worknet_skill(Path::new(BUILD_TIME_SKILLS_ROOT)).expect("repository skill pack")
    }

    #[test]
    fn repository_skill_has_versioned_frontmatter() {
        let pack = repo_pack();
        assert_eq!(pack.name, WORKNET_SKILL);
        assert!(pack.version.is_some(), "metadata.version is required");
        assert_eq!(
            pack.tool_schema_version.as_deref(),
            Some(mathscape_mcp::tools::TOOL_SCHEMA_VERSION),
            "the skill must be written against the current MCP tool table"
        );
        assert!(pack.files > 1);
        assert_eq!(pack.sha256.len(), 64);
    }

    #[test]
    fn read_instruction_is_first_turn_context_not_prefix() {
        let pack = repo_pack();
        let prefix = pack.instructions();
        assert!(prefix.contains(pack.short_hash()));
        assert!(prefix.contains(&pack.dir.display().to_string()));
        assert!(!prefix.contains("Before building anything"));
        let first = pack.first_turn_note();
        assert!(first.contains("SKILL.md"));
        assert!(first.contains("runtime-capabilities.md"));
        let update = pack.update_note(Some("0.6.4"), Some("mathscape-mcp/16"));
        assert!(update.starts_with("工具/skill 已更新到"));
        assert!(update.contains("0.6.4"));
        assert!(update.contains(pack.version.as_deref().unwrap()));
    }

    #[test]
    fn skill_does_not_tell_the_agent_to_call_protocol_methods() {
        let text = fs::read_to_string(repo_pack().dir.join("SKILL.md")).unwrap();
        assert!(!text.contains("调用 `tools/list`"));
    }

    /// Every MCP tool is described, and the sandbox section lists exactly
    /// the tools the sandbox mounts.
    #[test]
    fn runtime_capabilities_match_the_tool_table() {
        let text =
            fs::read_to_string(repo_pack().dir.join("references/runtime-capabilities.md")).unwrap();
        for tool in mathscape_mcp::tools::TOOL_NAMES {
            assert!(
                text.contains(&format!("`{tool}`")),
                "{tool} missing from skill"
            );
        }
        let section = text
            .split("## Mathscape 沙箱运行时")
            .nth(1)
            .expect("sandbox runtime section");
        let mounted_line = section
            .lines()
            .find(|line| line.contains("**挂载的 mathscape 工具**"))
            .expect("mounted tools line");
        let mounted: Vec<String> = mounted_line
            .split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect();
        assert_eq!(mounted, crate::sandbox_dialogue::sandbox_tool_allowlist());
    }

    #[test]
    fn hash_ignores_mac_metadata_and_changes_with_content() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(WORKNET_SKILL);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: mathscape-worknet\nmetadata:\n  version: \"1\"\n---\nbody\n",
        )
        .unwrap();
        let first = load_worknet_skill(temp.path()).unwrap();
        fs::write(dir.join("._SKILL.md"), "junk").unwrap();
        assert_eq!(
            load_worknet_skill(temp.path()).unwrap().sha256,
            first.sha256
        );
        fs::write(dir.join("extra.md"), "x").unwrap();
        assert_ne!(
            load_worknet_skill(temp.path()).unwrap().sha256,
            first.sha256
        );
        assert_eq!(first.version.as_deref(), Some("1"));
    }
}
