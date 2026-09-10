//! Position-independent checks. M2 will compose these with task-scoped locators.
use crate::StageValidationReport;
use crate::discovery_validation::{relation_types_valid, valid_temp_id};
use mathscape_domain::{FactDisposition, SourceFactInput, SourceFactProposal};
use std::collections::{HashMap, HashSet};

pub fn preflight_source_facts(
    input: &SourceFactInput,
    proposal: &SourceFactProposal,
) -> StageValidationReport {
    let mut report = StageValidationReport::default();
    if input.request_ref != proposal.request_ref {
        report.push("request_mismatch", "request_ref", "wrong batch");
    }
    let units: HashMap<_, _> = input
        .units
        .iter()
        .map(|u| (u.unit_ref.as_str(), u))
        .collect();
    let tasks: HashMap<_, _> = input
        .tasks
        .iter()
        .map(|t| (t.task_ref.as_str(), t))
        .collect();
    if units.len() != input.units.len() || tasks.len() != input.tasks.len() {
        report.push(
            "invalid_input",
            "input",
            "duplicate directory or task entries",
        );
    }
    let mut seen = HashSet::new();
    for (i, result) in proposal.tasks.iter().enumerate() {
        let path = format!("tasks[{i}]");
        if !seen.insert(result.task_ref.as_str()) {
            report.push("duplicate_id", &path, "duplicate task result");
        }
        let Some(task) = tasks.get(result.task_ref.as_str()) else {
            report.push("unknown_task", &path, "task is not in batch");
            continue;
        };
        let Some(owner) = units.get(task.owner_unit_ref.as_str()) else {
            report.push("invalid_input", &path, "task owner is absent");
            continue;
        };
        let mut refs = HashSet::new();
        for id in result
            .source_labels
            .iter()
            .map(|f| &f.fact_ref)
            .chain(result.references.iter().map(|f| &f.fact_ref))
            .chain(result.relations.iter().map(|f| &f.fact_ref))
        {
            if !valid_temp_id(id) || !refs.insert(id) {
                report.push(
                    "invalid_fact_ref",
                    &path,
                    "fact IDs must be valid and unique within task",
                );
            }
        }
        if let FactDisposition::Unprocessed { reason } = &result.disposition
            && (reason.trim().is_empty()
                || !refs.is_empty()
                || result.source_title.is_some()
                || !result.issues.is_empty())
        {
            report.push(
                "invalid_disposition",
                &path,
                "unprocessed requires a reason and no facts",
            );
        }
        for (r, reference) in result.references.iter().enumerate() {
            if let Some(target) = &reference.target_hint {
                if target == &owner.unit_ref {
                    report.push(
                        "invalid_reference",
                        format!("{path}.references[{r}].target_hint"),
                        "reference must not target its owner",
                    );
                } else if !units.contains_key(target.as_str()) {
                    report.push(
                        "unknown_unit",
                        format!("{path}.references[{r}].target_hint"),
                        "target is not in directory",
                    );
                }
            }
        }
        for (r, relation) in result.relations.iter().enumerate() {
            let field = format!("{path}.relations[{r}]");
            match units.get(relation.target.as_str()) {
                Some(target)
                    if target.unit_ref != owner.unit_ref
                        && relation_types_valid(
                            relation.relation,
                            owner.unit_kind,
                            target.unit_kind,
                        ) => {}
                _ => report.push(
                    "invalid_relation",
                    &field,
                    "invalid target or endpoint types",
                ),
            }
            if relation.basis.trim().is_empty() || relation.evidence.is_empty() {
                report.push(
                    "invalid_evidence",
                    field,
                    "relation needs evidence and basis",
                );
            }
        }
    }
    for task in &input.tasks {
        if !seen.contains(task.task_ref.as_str()) {
            report.push("missing_task", &task.task_ref, "task result omitted");
        }
    }
    report
        .not_checked
        .push("fact positions, evidence ownership and target label resolution (M2)".into());
    report
}
