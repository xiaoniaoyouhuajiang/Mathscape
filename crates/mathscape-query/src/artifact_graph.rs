use mathscape_domain::{
    CanonicalMarkdownView, ConversionExitStatus, ConversionRun, EntityKind, LocationStatus,
    PageIndexBase, SourcePayload, StableId,
};
use mathscape_graph::{
    GraphProjection, GraphProjectionError, IncidenceDirection, ProjectionEndpoint, ProjectionLens,
    ProjectionNode, ProjectionRelation, ProjectionRelationKind, ViewLayer,
};
use mathscape_store_sqlite::{ArtifactCatalogSnapshot, CanonicalEnsureLease};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default)]
pub struct ArtifactGraphProjector;

impl ArtifactGraphProjector {
    pub fn project(
        &self,
        catalog: &ArtifactCatalogSnapshot,
        lens: ProjectionLens,
    ) -> Result<GraphProjection, ArtifactGraphProjectionError> {
        project_artifact_catalog(catalog, lens)
    }
}

pub fn project_artifact_catalog(
    catalog: &ArtifactCatalogSnapshot,
    lens: ProjectionLens,
) -> Result<GraphProjection, ArtifactGraphProjectionError> {
    project_artifact_catalog_labeled(catalog, lens, None, &[], &[])
}

/// Like [`project_artifact_catalog`], with N3d snapshot names (snapshot
/// Artifact id → name) prefixed to the snapshot material labels.
pub(crate) fn project_artifact_catalog_labeled(
    catalog: &ArtifactCatalogSnapshot,
    lens: ProjectionLens,
    snapshot_labels: Option<&BTreeMap<String, String>>,
    canonical: &[CanonicalMarkdownView],
    parsing: &[CanonicalEnsureLease],
) -> Result<GraphProjection, ArtifactGraphProjectionError> {
    if !matches!(lens, ProjectionLens::Source | ProjectionLens::Provenance) {
        return Err(ArtifactGraphProjectionError::UnsupportedLens(lens));
    }

    let names: BTreeMap<_, _> = catalog
        .artifacts()
        .iter()
        .map(|record| {
            let artifact = record.entity().payload();
            (
                record.entity().header().stable_id().clone(),
                readable_filename(artifact.original_filename(), artifact.media_type().as_str()),
            )
        })
        .collect();
    let mut origins: BTreeMap<StableId, BTreeSet<String>> = BTreeMap::new();
    let mut conversion_names = BTreeMap::new();
    for relation in catalog.conversions() {
        let SourcePayload::ConversionRun(run) = relation.payload() else {
            continue;
        };
        let inputs: BTreeSet<_> = relation
            .endpoints()
            .iter()
            .filter(|endpoint| endpoint.role().as_str() == "input")
            .filter_map(|endpoint| names.get(endpoint.entity()).cloned())
            .collect();
        let context = format!(
            "{} · {}",
            inputs.into_iter().collect::<Vec<_>>().join("、"),
            page_label(run)
        );
        conversion_names.insert(relation.header().stable_id().clone(), context.clone());
        for endpoint in relation
            .endpoints()
            .iter()
            .filter(|endpoint| endpoint.role().as_str() == "output")
        {
            origins
                .entry(endpoint.entity().clone())
                .or_default()
                .insert(context.clone());
        }
    }
    let mut nodes = Vec::with_capacity(catalog.artifacts().len());
    for record in catalog.artifacts() {
        let header = record.entity().header();
        let artifact = record.entity().payload();
        let origin = origins.get(header.stable_id());
        let kind = product_label(artifact.media_type().as_str());
        let label = match origin {
            Some(contexts) if contexts.len() == 1 => {
                format!("{} · {kind}", contexts.first().unwrap())
            }
            Some(contexts) => format!("共享{kind} · {} 个来源", contexts.len()),
            None => names[header.stable_id()].clone(),
        };
        let label = match snapshot_labels.and_then(|labels| labels.get(header.stable_id().as_str()))
        {
            Some(name) => format!("「{name}」 · {label}"),
            None => label,
        };
        let location_status = record
            .locations()
            .first()
            .map(|location| status_label(location.status()))
            .unwrap_or("unlocated");
        let mut summary = format!(
            "{} · {} bytes · {location_status}",
            artifact.media_type().as_str(),
            artifact.byte_size()
        );
        if let Some(contexts) = origin {
            summary.push_str(&format!(
                " · 来源：{}",
                contexts.iter().cloned().collect::<Vec<_>>().join("；")
            ));
        }
        if let Some(signal) = parsing_signal(parsing, header.stable_id()) {
            summary.push_str(" · ");
            summary.push_str(&signal);
        }
        nodes.push(
            ProjectionNode::new(
                header.stable_id().clone(),
                EntityKind::SourceArtifact,
                ViewLayer::Source,
                label,
            )?
            .summary(summary)?
            .lifecycle(header.lifecycle()),
        );
    }

    let mut relations = Vec::with_capacity(catalog.conversions().len());
    for relation in catalog.conversions() {
        let SourcePayload::ConversionRun(run) = relation.payload() else {
            return Err(ArtifactGraphProjectionError::UnexpectedCatalogRelation(
                relation.header().stable_id().clone(),
            ));
        };
        let endpoints = relation
            .endpoints()
            .iter()
            .map(|endpoint| {
                let direction = match endpoint.role().as_str() {
                    "input" => IncidenceDirection::IntoRelation,
                    "output" => IncidenceDirection::OutOfRelation,
                    role => {
                        return Err(ArtifactGraphProjectionError::UnexpectedConversionRole {
                            relation: relation.header().stable_id().clone(),
                            role: role.to_owned(),
                        });
                    }
                };
                Ok(ProjectionEndpoint::new(
                    endpoint.entity().clone(),
                    endpoint.role().clone(),
                    endpoint.ordinal(),
                    direction,
                ))
            })
            .collect::<Result<Vec<_>, ArtifactGraphProjectionError>>()?;
        let tool = match run.tool_version() {
            Some(version) => format!("{} {version}", run.tool_name()),
            None => run.tool_name().to_owned(),
        };
        let label = format!("{} · 解析", conversion_names[relation.header().stable_id()]);
        let summary = format!(
            "{tool} · {}",
            conversion_summary(run.exit_status(), run.parameters().output_format())
        );
        relations.push(
            ProjectionRelation::new(
                relation.header().stable_id().clone(),
                EntityKind::ConversionRun,
                ViewLayer::Source,
                label,
                endpoints,
            )?
            .relation_kind(ProjectionRelationKind::Conversion)
            .summary(summary)?
            .lifecycle(relation.header().lifecycle()),
        );
    }

    let (nodes, relations) = collapse_canonical_markdown(nodes, relations, canonical, &names)?;
    GraphProjection::new(lens, vec![], nodes, relations).map_err(Into::into)
}

/// 「解析中 第a–b页」 for a PDF with a live canonical markdown ensure.
pub(crate) fn parsing_signal(parsing: &[CanonicalEnsureLease], pdf: &StableId) -> Option<String> {
    let ranges: Vec<_> = parsing
        .iter()
        .filter(|lease| &lease.source_pdf_artifact_id == pdf)
        .map(|lease| {
            if lease.pages.from == lease.pages.to {
                format!("第{}页", lease.pages.from)
            } else {
                format!("第{}–{}页", lease.pages.from, lease.pages.to)
            }
        })
        .collect();
    (!ranges.is_empty()).then(|| format!("解析中 {}", ranges.join("、")))
}

/// Markdown artifact id → canonical id, for fragments a registry entry replaces.
///
/// A physical markdown artifact that several canonical views claim (shared
/// across lineages) is ambiguous: folding it into whichever view came first
/// would rewire its edges to the wrong canonical node. Such artifacts are left
/// out of the map so they stay as their own uncollapsed node.
pub(crate) fn canonical_markdown_aliases(
    views: &[CanonicalMarkdownView],
) -> BTreeMap<StableId, StableId> {
    resolve_canonical_markdown_aliases(views).0
}

/// Returns `(aliases, conflicts)`: conflicts map each refused artifact to
/// every distinct canonical id that claimed it.
fn resolve_canonical_markdown_aliases(
    views: &[CanonicalMarkdownView],
) -> (
    BTreeMap<StableId, StableId>,
    BTreeMap<StableId, BTreeSet<StableId>>,
) {
    let mut claims: BTreeMap<StableId, BTreeSet<StableId>> = BTreeMap::new();
    for view in views {
        if view.head_artifact_id.is_none() {
            continue;
        }
        for id in view.collapsed_markdown_ids() {
            claims
                .entry(id)
                .or_default()
                .insert(view.canonical_id.clone());
        }
    }
    let mut aliases = BTreeMap::new();
    let mut conflicts = BTreeMap::new();
    for (artifact, canonical_ids) in claims {
        if canonical_ids.len() == 1 {
            let canonical = canonical_ids.into_iter().next().expect("one claim");
            aliases.insert(artifact, canonical);
        } else {
            conflicts.insert(artifact, canonical_ids);
        }
    }
    (aliases, conflicts)
}

fn collapse_canonical_markdown(
    nodes: Vec<ProjectionNode>,
    relations: Vec<ProjectionRelation>,
    views: &[CanonicalMarkdownView],
    names: &BTreeMap<StableId, String>,
) -> Result<(Vec<ProjectionNode>, Vec<ProjectionRelation>), ArtifactGraphProjectionError> {
    let (aliases, conflicts) = resolve_canonical_markdown_aliases(views);
    if aliases.is_empty() && conflicts.is_empty() {
        return Ok((nodes, relations));
    }
    let mut nodes: Vec<_> = nodes
        .into_iter()
        .filter(|node| !aliases.contains_key(node.entity_id()))
        .collect();
    let present: BTreeSet<_> = nodes.iter().map(|node| node.entity_id().clone()).collect();
    for view in views {
        if view.head_artifact_id.is_none() || present.contains(&view.canonical_id) {
            continue;
        }
        let pdf_label = names
            .get(&view.source_pdf_artifact_id)
            .map(String::as_str)
            .unwrap_or("PDF");
        let coverage = view.coverage(None);
        let Some(head_id) = view.head_artifact_id.clone() else {
            continue;
        };
        let head = head_id.to_string();
        let mut summary = format!(
            "text/markdown · canonical {} · head {head} · {}",
            view.canonical_id,
            if coverage.complete {
                "complete"
            } else {
                "partial"
            }
        );
        let shared = conflicts
            .values()
            .filter(|claimants| claimants.contains(&view.canonical_id))
            .count();
        if shared > 0 {
            // Diagnostic: these snapshots are claimed by several canonical
            // views, so they are not folded into this node.
            summary.push_str(&format!(
                " · canonical_alias_conflict: {shared} 个共享解析正文未折叠"
            ));
        }
        nodes.push(
            ProjectionNode::new(
                view.canonical_id.clone(),
                EntityKind::SourceArtifact,
                ViewLayer::Source,
                format!("{pdf_label} · 规范解析正文"),
            )?
            .summary(summary)?
            .lifecycle(mathscape_domain::LifecycleStatus::Active)
            // The canonical id is logical; readers open the head snapshot.
            .openable_artifact(head_id),
        );
    }
    let mut rewritten = Vec::with_capacity(relations.len());
    for relation in relations {
        let endpoints = relation
            .endpoints()
            .iter()
            .map(|endpoint| {
                let node = aliases
                    .get(endpoint.node())
                    .cloned()
                    .unwrap_or_else(|| endpoint.node().clone());
                ProjectionEndpoint::new(
                    node,
                    endpoint.role().clone(),
                    endpoint.ordinal(),
                    endpoint.direction(),
                )
            })
            .collect();
        let mut rebuilt = ProjectionRelation::new(
            relation.relation_id().clone(),
            relation.kind(),
            relation.layer(),
            relation.label(),
            endpoints,
        )?
        .relation_kind(relation.relation_kind_value());
        if let Some(summary) = relation.summary_ref() {
            rebuilt = rebuilt.summary(summary)?;
        }
        if let Some(lifecycle) = relation.lifecycle_status() {
            rebuilt = rebuilt.lifecycle(lifecycle);
        }
        if let Some(group) = relation.group_ref() {
            rebuilt = rebuilt.group(group.clone());
        }
        rewritten.push(rebuilt);
    }
    Ok((nodes, rewritten))
}

fn page_label(run: &ConversionRun) -> String {
    match run.parameters().page_range() {
        Some(range) => {
            let offset = if range.index_base() == PageIndexBase::Zero {
                1_u64
            } else {
                0
            };
            format!(
                "PDF {}–{} 页",
                u64::from(range.start()) + offset,
                u64::from(range.end()) + offset
            )
        }
        None => "页码未记录".to_owned(),
    }
}

pub(crate) fn product_label(media: &str) -> &'static str {
    match media {
        "text/markdown" => "解析正文",
        "application/json" => "解析元数据",
        "application/vnd.mathscape.ink-snapshot+json" => "手稿快照",
        "application/pdf" => "PDF",
        _ if media.starts_with("image/") => "解析图片",
        _ => "附件",
    }
}

pub(crate) fn readable_filename(filename: Option<&str>, media: &str) -> String {
    let Some(filename) = filename else {
        return product_label(media).to_owned();
    };
    let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
    if stem.len() == 64 && stem.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return product_label(media).to_owned();
    }
    if media == "application/pdf" {
        stem.to_owned()
    } else {
        filename.to_owned()
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ArtifactGraphProjectionError {
    #[error("Artifact Catalog cannot be projected through the {0:?} lens")]
    UnsupportedLens(ProjectionLens),
    #[error("Artifact Catalog contains non-ConversionRun relation {0}")]
    UnexpectedCatalogRelation(StableId),
    #[error("ConversionRun {relation} contains unsupported endpoint role {role:?}")]
    UnexpectedConversionRole { relation: StableId, role: String },
    #[error("graph projection is invalid: {0}")]
    Graph(#[from] GraphProjectionError),
}

fn status_label(status: LocationStatus) -> &'static str {
    match status {
        LocationStatus::Available => "available",
        LocationStatus::Missing => "missing",
        LocationStatus::Corrupted => "corrupted",
        LocationStatus::AccessDenied => "access_denied",
        LocationStatus::Unavailable => "unavailable",
    }
}

fn conversion_summary(status: ConversionExitStatus, output_format: &str) -> String {
    let status = match status {
        ConversionExitStatus::Succeeded => "succeeded",
        ConversionExitStatus::Failed => "failed",
        ConversionExitStatus::Partial => "partial",
        ConversionExitStatus::TimedOut => "timed_out",
        ConversionExitStatus::Cancelled => "cancelled",
    };
    format!("{status} · output {output_format}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mathscape_domain::{CanonicalSegment, EndpointRole, InclusivePageRange};

    fn id(value: &str) -> StableId {
        StableId::parse(value).unwrap()
    }

    fn view(canonical: &str, pdf: &str, markdown: &[&str]) -> CanonicalMarkdownView {
        CanonicalMarkdownView {
            canonical_id: id(canonical),
            source_pdf_artifact_id: id(pdf),
            parser_profile: "profile".to_owned(),
            parser_policy_version: "marker-2.0.0".to_owned(),
            head_artifact_id: Some(id(markdown[0])),
            head_revision: None,
            total_pages: Some(2),
            segments: markdown
                .iter()
                .enumerate()
                .map(|(index, artifact)| CanonicalSegment {
                    conversion_id: id(&format!("mathscape:conversion/marker/{canonical}-{index}")
                        .replace("mathscape:canonical-markdown/", "")),
                    markdown_artifact_id: id(artifact),
                    pages: Some(InclusivePageRange::new(1, 1).unwrap()),
                    finished_at: None,
                })
                .collect(),
        }
    }

    fn node(entity: &str) -> ProjectionNode {
        ProjectionNode::new(
            id(entity),
            EntityKind::SourceArtifact,
            ViewLayer::Source,
            entity.to_owned(),
        )
        .unwrap()
    }

    fn conversion(relation: &str, input: &str, output: &str) -> ProjectionRelation {
        ProjectionRelation::new(
            id(relation),
            EntityKind::ConversionRun,
            ViewLayer::Source,
            relation.to_owned(),
            vec![
                ProjectionEndpoint::new(
                    id(input),
                    EndpointRole::parse("input").unwrap(),
                    None,
                    IncidenceDirection::IntoRelation,
                ),
                ProjectionEndpoint::new(
                    id(output),
                    EndpointRole::parse("output").unwrap(),
                    None,
                    IncidenceDirection::OutOfRelation,
                ),
            ],
        )
        .unwrap()
        .relation_kind(ProjectionRelationKind::Conversion)
    }

    #[test]
    fn shared_markdown_claimed_by_two_canonicals_is_not_collapsed_first_wins() {
        let canonical_a = "mathscape:canonical-markdown/a";
        let canonical_b = "mathscape:canonical-markdown/b";
        let pdf_a = "mathscape:artifact/test/pdf-a";
        let pdf_b = "mathscape:artifact/test/pdf-b";
        let own_a = "mathscape:artifact/test/md-a";
        let shared = "mathscape:artifact/test/md-shared";
        let views = vec![
            view(canonical_a, pdf_a, &[own_a, shared]),
            view(canonical_b, pdf_b, &[shared]),
        ];

        let aliases = canonical_markdown_aliases(&views);
        assert_eq!(aliases.get(&id(own_a)), Some(&id(canonical_a)));
        assert!(
            !aliases.contains_key(&id(shared)),
            "shared artifact must not silently map to the first canonical"
        );
        let (_, conflicts) = resolve_canonical_markdown_aliases(&views);
        assert_eq!(
            conflicts.get(&id(shared)),
            Some(&BTreeSet::from([id(canonical_a), id(canonical_b)]))
        );

        let nodes = vec![node(pdf_a), node(pdf_b), node(own_a), node(shared)];
        let relations = vec![
            conversion("mathscape:conversion/marker/run-a", pdf_a, own_a),
            conversion("mathscape:conversion/marker/run-b", pdf_b, shared),
        ];
        let (nodes, relations) =
            collapse_canonical_markdown(nodes, relations, &views, &BTreeMap::new()).unwrap();
        let ids: BTreeSet<_> = nodes.iter().map(|node| node.entity_id().clone()).collect();
        assert!(
            ids.contains(&id(shared)),
            "conflicting artifact stays uncollapsed"
        );
        assert!(
            !ids.contains(&id(own_a)),
            "unambiguous artifact still collapses"
        );
        assert!(ids.contains(&id(canonical_a)));
        assert!(ids.contains(&id(canonical_b)));

        let run_b = relations
            .iter()
            .find(|relation| relation.relation_id() == &id("mathscape:conversion/marker/run-b"))
            .unwrap();
        let outputs: Vec<_> = run_b
            .endpoints()
            .iter()
            .filter(|endpoint| endpoint.role().as_str() == "output")
            .map(|endpoint| endpoint.node().clone())
            .collect();
        assert_eq!(
            outputs,
            vec![id(shared)],
            "edge is not folded into canonical a"
        );

        let summary_a = nodes
            .iter()
            .find(|node| node.entity_id() == &id(canonical_a))
            .and_then(|node| node.summary_ref().map(str::to_owned))
            .unwrap();
        assert!(
            summary_a.contains("canonical_alias_conflict"),
            "{summary_a}"
        );
    }
}
