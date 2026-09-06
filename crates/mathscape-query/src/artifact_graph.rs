use mathscape_domain::{ConversionExitStatus, EntityKind, LocationStatus, SourcePayload, StableId};
use mathscape_graph::{
    GraphProjection, GraphProjectionError, IncidenceDirection, ProjectionEndpoint, ProjectionLens,
    ProjectionNode, ProjectionRelation, ViewLayer,
};
use mathscape_store_sqlite::ArtifactCatalogSnapshot;
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

    let mut nodes = Vec::with_capacity(catalog.artifacts().len());
    for record in catalog.artifacts() {
        let header = record.entity().header();
        let artifact = record.entity().payload();
        let label = artifact
            .original_filename()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Artifact {}", artifact.media_type().as_str()));
        let location_status = record
            .locations()
            .first()
            .map(|location| status_label(location.status()))
            .unwrap_or("unlocated");
        let summary = format!(
            "{} · {} bytes · {location_status}",
            artifact.media_type().as_str(),
            artifact.byte_size()
        );
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
        let label = match run.tool_version() {
            Some(version) => format!("{} {version}", run.tool_name()),
            None => run.tool_name().to_owned(),
        };
        let summary = conversion_summary(run.exit_status(), run.parameters().output_format());
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
