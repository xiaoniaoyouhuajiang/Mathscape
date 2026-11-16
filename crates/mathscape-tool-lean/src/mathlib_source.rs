//! 在本地 profile 自带的 mathlib 源码里按全名查找声明。
//!
//! 只读：不运行 Lean、不改 profile、不联网。解析是逐行的轻量扫描（跟踪
//! `namespace`/`section`/`end`，识别常见声明关键字与修饰符），用来回答
//! “这个名字在当前 mathlib 版本里是否存在、在哪个模块”。它不判断数学内容
//! 是否与教材一致，那属于对齐线的审阅。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::{DiscoveredLeanProfile, LeanProfileState};

/// 找到的声明位置。`location` 形如 `Mathlib/Data/Nat/Fib/Basic.lean:42`。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MathlibDeclarationLocation {
    pub module: String,
    pub location: String,
}

/// 一个可查询的 mathlib 源码目录及其版本信息。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MathlibSource {
    pub profile_id: String,
    pub mathlib_version: String,
    /// 含 `Mathlib/` 子目录的包根目录。
    pub package_root: PathBuf,
}

/// 从已登记的 profile 中挑出第一个状态为 ready 且带 mathlib 源码的。
/// 找不到时返回人能读懂的原因（基线因此保持 unchecked）。
pub fn select_mathlib_source(profiles: &[DiscoveredLeanProfile]) -> Result<MathlibSource, String> {
    if profiles.is_empty() {
        return Err("没有已登记的 Lean profile".to_owned());
    }
    let mut reasons = Vec::new();
    for profile in profiles {
        let Some(meta) = profile.metadata.as_ref() else {
            reasons.push(format!("{}：缺少 profile.json", profile.id));
            continue;
        };
        if meta.state != LeanProfileState::Ready {
            reasons.push(format!("{}：profile 尚未就绪", profile.id));
            continue;
        }
        let Some(package_root) = mathlib_package_root(&meta.project_root) else {
            reasons.push(format!("{}：找不到 mathlib 源码目录", profile.id));
            continue;
        };
        let mathlib_version = meta
            .detected_mathlib_revision
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| meta.mathlib_reference.clone());
        return Ok(MathlibSource {
            profile_id: profile.id.clone(),
            mathlib_version,
            package_root,
        });
    }
    Err(reasons.join("；"))
}

/// `project_root` 本身是 mathlib，或依赖 mathlib（`.lake/packages/mathlib`）。
pub fn mathlib_package_root(project_root: &Path) -> Option<PathBuf> {
    [
        project_root.join(".lake").join("packages").join("mathlib"),
        project_root.to_path_buf(),
    ]
    .into_iter()
    .find(|candidate| candidate.join("Mathlib").is_dir())
}

impl MathlibSource {
    /// 一次扫描查多个全名。结果里每个查询名都有一项；`None` 表示没找到。
    pub fn locate_many(
        &self,
        names: &BTreeSet<String>,
    ) -> io::Result<BTreeMap<String, Option<MathlibDeclarationLocation>>> {
        locate_in_package(&self.package_root, names)
    }
}

/// 在 `package_root/Mathlib/**.lean` 里查找 `names`。
pub fn locate_in_package(
    package_root: &Path,
    names: &BTreeSet<String>,
) -> io::Result<BTreeMap<String, Option<MathlibDeclarationLocation>>> {
    let mut found: BTreeMap<String, Option<MathlibDeclarationLocation>> =
        names.iter().map(|name| (name.clone(), None)).collect();
    if names.is_empty() {
        return Ok(found);
    }
    let short_names: BTreeSet<&str> = names
        .iter()
        .map(|name| name.rsplit('.').next().unwrap_or(name))
        .collect();
    let mut files = Vec::new();
    collect_lean_files(&package_root.join("Mathlib"), &mut files)?;
    files.sort();
    for file in files {
        if found.values().all(Option::is_some) {
            break;
        }
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        if !short_names.iter().any(|short| text.contains(short)) {
            continue;
        }
        let relative = file
            .strip_prefix(package_root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        let module = relative
            .strip_suffix(".lean")
            .unwrap_or(&relative)
            .replace('/', ".");
        for (full, line) in declarations(&text) {
            if let Some(slot) = found.get_mut(&full)
                && slot.is_none()
            {
                *slot = Some(MathlibDeclarationLocation {
                    module: module.clone(),
                    location: format!("{relative}:{line}"),
                });
            }
        }
    }
    Ok(found)
}

fn collect_lean_files(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_lean_files(&path, out)?;
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "lean") {
            out.push(path);
        }
    }
    Ok(())
}

const DECL_KEYWORDS: &[&str] = &[
    "theorem",
    "lemma",
    "def",
    "abbrev",
    "instance",
    "structure",
    "class",
    "inductive",
    "axiom",
    "opaque",
    "irreducible_def",
];

const MODIFIERS: &[&str] = &[
    "private",
    "protected",
    "noncomputable",
    "nonrec",
    "partial",
    "unsafe",
    "public",
    "scoped",
];

/// 返回 (全名, 行号)。行号从 1 开始。
pub fn declarations(text: &str) -> Vec<(String, usize)> {
    // 每层记录 (是否 namespace, 名字的段数)。
    let mut scopes: Vec<(bool, Vec<String>)> = Vec::new();
    let mut out = Vec::new();
    let mut in_block_comment = 0usize;
    for (index, raw) in text.lines().enumerate() {
        let mut line = raw.trim();
        if in_block_comment > 0 {
            in_block_comment = update_block_depth(line, in_block_comment);
            continue;
        }
        if line.starts_with("/-") {
            in_block_comment = update_block_depth(line, 0);
            continue;
        }
        if line.starts_with("--") {
            continue;
        }
        let mut tokens = line.split_whitespace();
        match tokens.next() {
            Some("namespace") => {
                if let Some(name) = tokens.next() {
                    scopes.push((true, name.split('.').map(str::to_owned).collect()));
                }
                continue;
            }
            Some("section") | Some("noncomputable") if line.contains("section") => {
                let rest: Vec<&str> = line.split_whitespace().collect();
                if let Some(pos) = rest.iter().position(|token| *token == "section") {
                    let name = rest.get(pos + 1).copied().unwrap_or("");
                    scopes.push((false, vec![name.to_owned()]));
                    continue;
                }
            }
            Some("end") => {
                scopes.pop();
                continue;
            }
            _ => {}
        }
        // 去掉单行属性 @[...]
        while line.starts_with("@[") {
            match line.find(']') {
                Some(end) => line = line[end + 1..].trim_start(),
                None => break,
            }
        }
        let mut tokens = line.split_whitespace().peekable();
        while tokens.peek().is_some_and(|token| MODIFIERS.contains(token)) {
            tokens.next();
        }
        let Some(keyword) = tokens.next() else {
            continue;
        };
        if !DECL_KEYWORDS.contains(&keyword) {
            continue;
        }
        let Some(name_token) = tokens.next() else {
            continue;
        };
        let name: String = name_token
            .chars()
            .take_while(|c| !c.is_whitespace() && !matches!(c, '(' | '{' | '[' | ':' | '⦃'))
            .collect();
        if name.is_empty() {
            continue;
        }
        let full = if let Some(root) = name.strip_prefix("_root_.") {
            root.to_owned()
        } else {
            let mut parts: Vec<&str> = scopes
                .iter()
                .filter(|(is_namespace, _)| *is_namespace)
                .flat_map(|(_, segments)| segments.iter().map(String::as_str))
                .collect();
            parts.push(&name);
            parts.join(".")
        };
        out.push((full, index + 1));
    }
    out
}

fn update_block_depth(line: &str, mut depth: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'/' && bytes[i + 1] == b'-' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'-' && bytes[i + 1] == b'/' {
            depth = depth.saturating_sub(1);
            i += 2;
        } else {
            i += 1;
        }
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
/-! # 斐波那契
namespace Fake
theorem fake : True := trivial
-/
namespace Nat

/-- 文档 -/
@[simp] theorem fib_add_two {n : ℕ} : fib (n + 2) = fib n + fib (n + 1) := rfl

def fib (n : ℕ) : ℕ := 0

section Foo
protected theorem helper : True := trivial
end Foo

theorem _root_.Int.cassini_like (n : ℕ) : True := trivial
end Nat

namespace Matrix.Special
lemma x : True := trivial
end Matrix.Special
noncomputable def top : ℕ := 0
"#;

    #[test]
    fn tracks_namespaces_and_root_names() {
        let names: Vec<String> = declarations(SAMPLE).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            vec![
                "Nat.fib_add_two",
                "Nat.fib",
                "Nat.helper",
                "Int.cassini_like",
                "Matrix.Special.x",
                "top"
            ]
        );
    }

    #[test]
    fn locates_in_package_tree() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Mathlib/Data/Nat/Fib");
        fs::create_dir_all(&file).unwrap();
        fs::write(file.join("Basic.lean"), SAMPLE).unwrap();
        let names: BTreeSet<String> = ["Nat.fib", "Nat.fib_mul_matrix"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let found = locate_in_package(dir.path(), &names).unwrap();
        let fib = found["Nat.fib"].as_ref().unwrap();
        assert_eq!(fib.module, "Mathlib.Data.Nat.Fib.Basic");
        assert_eq!(fib.location, "Mathlib/Data/Nat/Fib/Basic.lean:11");
        assert!(found["Nat.fib_mul_matrix"].is_none());
        assert!(mathlib_package_root(dir.path()).is_some());
    }
}
