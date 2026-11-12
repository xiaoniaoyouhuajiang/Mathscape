//! mathlib 名称重新核对（S13 决定 4）。
//!
//! 在本地 Lean profile 自带的 mathlib 源码里查找共享对象写的 mathlib 名称，
//! 把结果以 `system:mathlib-baseline` 的身份写成 `confirmed` / `not_found`。
//! 没有可用 profile 时什么也不写，基线保持 `unchecked`。
//!
//! 只读 profile：不运行 Lean、不改依赖、不联网。核对只说明“这个名字在这个
//! mathlib 版本里存在”，不说明它和教材里的命题是一回事。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use mathscape_domain::{
    ActorRef, BaselineCheckMethod, ImpactPlanStatus, MATHLIB_BASELINE_ACTOR, Timestamp,
    UserChangeRequest,
};
use mathscape_orchestrator::map_edit::plan_map_change_over_head;
use mathscape_orchestrator::semantic_plan::{baseline_candidates, baseline_operations};
use mathscape_store_sqlite::SqliteArtifactStore;
use mathscape_tool_lean::{
    MathlibSource, discover_profile_root, list_profiles, select_mathlib_source,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// 一次核对的结果，给 Desktop 按钮和 CLI 展示。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct BaselineRecheckReport {
    /// `committed`、`unchanged`、`nothing_to_check` 或 `unavailable`。
    pub status: String,
    pub profile_id: Option<String>,
    pub mathlib_version: Option<String>,
    pub checked: usize,
    pub confirmed: Vec<String>,
    pub not_found: Vec<String>,
    pub commit_id: Option<String>,
    /// `unavailable` 时说明为什么没有核对。
    pub reason: Option<String>,
}

impl BaselineRecheckReport {
    fn empty(status: &str, reason: Option<String>) -> Self {
        Self {
            status: status.to_owned(),
            profile_id: None,
            mathlib_version: None,
            checked: 0,
            confirmed: Vec::new(),
            not_found: Vec::new(),
            commit_id: None,
            reason,
        }
    }
}

/// 找工作区可用的 mathlib 源码；找不到时返回原因。
pub fn workspace_mathlib_source(workspace_root: &Path) -> Result<MathlibSource, String> {
    let found = discover_profile_root(Some(workspace_root))
        .ok_or_else(|| "未找到 Lean profile 目录".to_owned())?;
    let profiles = list_profiles(&found.root).map_err(|e| format!("无法读取 profile：{e}"))?;
    select_mathlib_source(&profiles)
}

/// 用工作区的 Lean profile 重新核对所有 mathlib 名称。
pub fn recheck_mathlib_baselines(
    workspace_root: &Path,
    time: Timestamp,
) -> Result<BaselineRecheckReport, String> {
    let source = match workspace_mathlib_source(workspace_root) {
        Ok(source) => source,
        Err(reason) => return Ok(BaselineRecheckReport::empty("unavailable", Some(reason))),
    };
    recheck_with_source(workspace_root, &source, time)
}

/// 同上，但指定 mathlib 源码（测试和 CLI `--mathlib` 使用）。
pub fn recheck_with_source(
    workspace_root: &Path,
    source: &MathlibSource,
    time: Timestamp,
) -> Result<BaselineRecheckReport, String> {
    let mut store = SqliteArtifactStore::open(&workspace_root.join("mathscape.sqlite3"))
        .map_err(|e| e.to_string())?;
    let snapshot = store.map_edit_snapshot().map_err(|e| e.to_string())?;
    let candidates = baseline_candidates(&snapshot);
    if candidates.is_empty() {
        return Ok(BaselineRecheckReport::empty("nothing_to_check", None));
    }
    let names: BTreeSet<String> = candidates.iter().map(|c| c.decl.clone()).collect();
    let located = source
        .locate_many(&names)
        .map_err(|e| format!("读取 mathlib 源码失败：{e}"))?;
    let lookup: BTreeMap<String, Option<(String, String)>> = located
        .into_iter()
        .map(|(name, found)| (name, found.map(|loc| (loc.module, loc.location))))
        .collect();
    let mut report = BaselineRecheckReport {
        status: "unchanged".to_owned(),
        profile_id: Some(source.profile_id.clone()),
        mathlib_version: Some(source.mathlib_version.clone()),
        checked: names.len(),
        confirmed: lookup
            .iter()
            .filter(|(_, v)| v.is_some())
            .map(|(k, _)| k.clone())
            .collect(),
        not_found: lookup
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.clone())
            .collect(),
        commit_id: None,
        reason: None,
    };
    let operations = baseline_operations(
        &candidates,
        &lookup,
        &source.profile_id,
        &source.mathlib_version,
        BaselineCheckMethod::MathlibSource,
    );
    if operations.is_empty() {
        return Ok(report);
    }
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(&operations).map_err(|e| e.to_string())?);
    hasher.update(snapshot.base_commit.as_deref().unwrap_or("").as_bytes());
    let request = UserChangeRequest {
        request_id: format!("mathscape:mathlib-baseline/{:x}", hasher.finalize()),
        base_commit: snapshot.base_commit.clone(),
        reason: format!(
            "在 profile {} 的 mathlib（{}）源码中核对名称",
            source.profile_id, source.mathlib_version
        ),
        operations,
    };
    let actor = ActorRef::parse(MATHLIB_BASELINE_ACTOR).map_err(|e| e.to_string())?;
    let planned = plan_map_change_over_head(&snapshot, &request, &actor);
    if planned.impact.status != ImpactPlanStatus::Ready {
        let reasons: Vec<String> = planned
            .impact
            .diagnostics
            .iter()
            .map(|d| format!("{}：{}", d.code, d.message))
            .collect();
        return Err(format!("核对结果无法写入：{}", reasons.join("；")));
    }
    let planner_actor = actor.clone();
    let request_for_plan = request.clone();
    let receipt = store
        .commit_map_change(
            &request,
            &planned.impact.plan_fingerprint,
            &actor,
            time,
            move |snapshot| {
                let planned =
                    plan_map_change_over_head(snapshot, &request_for_plan, &planner_actor);
                (planned.impact, planned.patch)
            },
        )
        .map_err(|e| e.to_string())?;
    report.status = "committed".to_owned();
    report.commit_id = Some(receipt.commit_id);
    Ok(report)
}
