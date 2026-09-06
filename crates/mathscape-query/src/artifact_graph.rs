use mathscape_domain::{
    ConversionExitStatus, ConversionRun, EntityKind, LocationStatus, PageIndexBase, SourcePayload,
    StableId,
};
use mathscape_graph::{
    GraphProjection, GraphProjectionError, IncidenceDirection, ProjectionEndpoint, ProjectionLens,
    ProjectionNode, ProjectionRelation, ViewLayer,
};
use mathscape_store_sqlite::ArtifactCatalogSnapshot;
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
            .summary(summary)?
            .lifecycle(relation.header().lifecycle()),
        );
    }

    GraphProjection::new(lens, vec![], nodes, relations).map_err(Into::into)
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

fn product_label(media: &str) -> &'static str {
    match media {
        "text/markdown" => "解析正文",
        "application/json" => "解析元数据",
        "application/pdf" => "PDF",
        _ if media.starts_with("image/") => "解析图片",
        _ => "附件",
    }
}

fn readable_filename(filename: Option<&str>, media: &str) -> String {
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
