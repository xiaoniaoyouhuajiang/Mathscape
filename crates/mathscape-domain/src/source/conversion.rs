use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    DomainError, EndpointRole, EntityHeader, EntityKind, EntityPayload, PayloadShape,
    RelationEndpoint, RelationRecord, StableId, Timestamp,
};

use super::{
    SourcePayload, SourceRelation, ensure_non_blank, ensure_non_empty_unique_refs,
    ensure_optional_non_blank, ensure_unique_refs,
};

/// Inclusive PDF page range with an explicit index convention.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct PageRange {
    start: u32,
    end: u32,
    index_base: PageIndexBase,
}

impl PageRange {
    pub fn new(start: u32, end: u32, index_base: PageIndexBase) -> Result<Self, DomainError> {
        if start > end {
            return Err(DomainError::InvalidRange {
                field: "conversion.page_range",
                start: u64::from(start),
                end: u64::from(end),
            });
        }
        Ok(Self {
            start,
            end,
            index_base,
        })
    }

    pub fn start(self) -> u32 {
        self.start
    }

    pub fn end(self) -> u32 {
        self.end
    }

    pub fn index_base(self) -> PageIndexBase {
        self.index_base
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageIndexBase {
    Zero,
    One,
}

/// Backend-neutral parameters that still preserve Marker reproducibility.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConversionParameters {
    page_range: Option<PageRange>,
    output_format: String,
    redo_inline_math: Option<bool>,
    additional_options: BTreeMap<String, String>,
}

impl ConversionParameters {
    pub fn new(
        page_range: Option<PageRange>,
        output_format: String,
        redo_inline_math: Option<bool>,
        additional_options: BTreeMap<String, String>,
    ) -> Result<Self, DomainError> {
        ensure_non_blank(&output_format, "conversion.output_format")?;
        for (key, value) in &additional_options {
            ensure_non_blank(key, "conversion.additional_options.key")?;
            ensure_non_blank(value, "conversion.additional_options.value")?;
        }
        Ok(Self {
            page_range,
            output_format,
            redo_inline_math,
            additional_options,
        })
    }

    pub fn page_range(&self) -> Option<PageRange> {
        self.page_range
    }

    pub fn output_format(&self) -> &str {
        &self.output_format
    }

    pub fn redo_inline_math(&self) -> Option<bool> {
        self.redo_inline_math
    }

    pub fn additional_options(&self) -> &BTreeMap<String, String> {
        &self.additional_options
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandRecordStatus {
    Recorded,
    ReconstructedFromArtifacts,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionExitStatus {
    Succeeded,
    Failed,
    Partial,
}

/// One immutable, auditable invocation that derives artifacts from artifacts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConversionRun {
    tool_name: String,
    tool_version: Option<String>,
    command_template: Option<String>,
    command_record_status: CommandRecordStatus,
    parameters: ConversionParameters,
    environment: Option<StableId>,
    started_at: Option<Timestamp>,
    finished_at: Option<Timestamp>,
    exit_status: ConversionExitStatus,
    warnings: Vec<StableId>,
    cost: Option<StableId>,
}

impl ConversionRun {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tool_name: String,
        tool_version: Option<String>,
        command_template: Option<String>,
        command_record_status: CommandRecordStatus,
        parameters: ConversionParameters,
        environment: Option<StableId>,
        started_at: Option<Timestamp>,
        finished_at: Option<Timestamp>,
        exit_status: ConversionExitStatus,
        warnings: Vec<StableId>,
        cost: Option<StableId>,
    ) -> Result<Self, DomainError> {
        ensure_unique_refs(&warnings, "conversion.warnings")?;
        ensure_non_blank(&tool_name, "conversion.tool_name")?;
        ensure_optional_non_blank(tool_version.as_deref(), "conversion.tool_version")?;
        ensure_optional_non_blank(command_template.as_deref(), "conversion.command_template")?;

        Ok(Self {
            tool_name,
            tool_version,
            command_template,
            command_record_status,
            parameters,
            environment,
            started_at,
            finished_at,
            exit_status,
            warnings,
            cost,
        })
    }

    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    pub fn tool_version(&self) -> Option<&str> {
        self.tool_version.as_deref()
    }

    pub fn command_template(&self) -> Option<&str> {
        self.command_template.as_deref()
    }

    pub fn command_record_status(&self) -> CommandRecordStatus {
        self.command_record_status
    }

    pub fn parameters(&self) -> &ConversionParameters {
        &self.parameters
    }

    pub fn environment(&self) -> Option<&StableId> {
        self.environment.as_ref()
    }

    pub fn started_at(&self) -> Option<&Timestamp> {
        self.started_at.as_ref()
    }

    pub fn finished_at(&self) -> Option<&Timestamp> {
        self.finished_at.as_ref()
    }

    pub fn exit_status(&self) -> ConversionExitStatus {
        self.exit_status
    }

    pub fn warnings(&self) -> &[StableId] {
        &self.warnings
    }

    pub fn cost(&self) -> Option<&StableId> {
        self.cost.as_ref()
    }

    /// Materializes the run as an input/output hyperedge.
    pub fn into_record(
        self,
        header: EntityHeader,
        input_artifacts: Vec<StableId>,
        output_artifacts: Vec<StableId>,
    ) -> Result<SourceRelation, DomainError> {
        ensure_non_empty_unique_refs(&input_artifacts, "conversion.input_artifacts")?;
        ensure_unique_refs(&output_artifacts, "conversion.output_artifacts")?;
        if self.exit_status == ConversionExitStatus::Succeeded && output_artifacts.is_empty() {
            return Err(DomainError::SuccessfulRunWithoutOutput);
        }

        let inputs: HashSet<_> = input_artifacts.iter().collect();
        if let Some(duplicate) = output_artifacts
            .iter()
            .find(|output| inputs.contains(output))
        {
            return Err(DomainError::DuplicateReference {
                field: "conversion.input_and_output_artifacts",
                stable_id: duplicate.to_string(),
            });
        }

        let input_role = EndpointRole::parse("input")?;
        let output_role = EndpointRole::parse("output")?;
        let mut endpoints = Vec::with_capacity(input_artifacts.len() + output_artifacts.len());
        for (ordinal, artifact) in input_artifacts.into_iter().enumerate() {
            endpoints.push(RelationEndpoint::new(
                input_role.clone(),
                artifact,
                Some(endpoint_ordinal(ordinal, "conversion.input_artifacts")?),
            ));
        }
        for (ordinal, artifact) in output_artifacts.into_iter().enumerate() {
            endpoints.push(RelationEndpoint::new(
                output_role.clone(),
                artifact,
                Some(endpoint_ordinal(ordinal, "conversion.output_artifacts")?),
            ));
        }

        RelationRecord::new(header, endpoints, SourcePayload::ConversionRun(self))
    }
}

fn endpoint_ordinal(value: usize, field: &'static str) -> Result<u32, DomainError> {
    u32::try_from(value).map_err(|_| DomainError::TooManyItems {
        field,
        maximum: u64::from(u32::MAX),
    })
}

impl EntityPayload for ConversionRun {
    fn entity_kind(&self) -> EntityKind {
        EntityKind::ConversionRun
    }

    fn payload_shape(&self) -> PayloadShape {
        PayloadShape::Relation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(name: &str) -> StableId {
        StableId::parse(format!("emir-fixture:artifact/{name}")).unwrap()
    }

    fn parameters() -> ConversionParameters {
        ConversionParameters::new(
            Some(PageRange::new(42, 48, PageIndexBase::Zero).unwrap()),
            "markdown".to_owned(),
            Some(true),
            BTreeMap::new(),
        )
        .unwrap()
    }

    fn header() -> EntityHeader {
        use crate::{ActorRef, LifecycleStatus, RevisionId, SchemaVersion};

        EntityHeader::new(
            StableId::parse("emir-fixture:conversion/source-a-marker").unwrap(),
            RevisionId::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
            EntityKind::ConversionRun,
            SchemaVersion::parse("emir-0.1.1").unwrap(),
            LifecycleStatus::Active,
            Timestamp::parse("2026-09-01T00:00:00+08:00").unwrap(),
            ActorRef::parse("historical-run-import").unwrap(),
            None,
        )
    }

    #[test]
    fn fixture_style_marker_run_is_valid() {
        let run = ConversionRun::new(
            "marker_single".to_owned(),
            None,
            Some("marker_single {source_pdf} --page_range 42-48".to_owned()),
            CommandRecordStatus::ReconstructedFromArtifacts,
            parameters(),
            None,
            None,
            None,
            ConversionExitStatus::Succeeded,
            vec![],
            None,
        )
        .unwrap()
        .into_record(
            header(),
            vec![artifact("source-a-pdf")],
            vec![artifact("source-a-markdown"), artifact("source-a-metadata")],
        )
        .unwrap();

        assert_eq!(run.endpoints().len(), 3);
        assert_eq!(run.endpoints()[0].role().as_str(), "input");
        assert_eq!(run.endpoints()[1].role().as_str(), "output");
    }

    #[test]
    fn successful_run_requires_new_output() {
        let error = ConversionRun::new(
            "marker_single".to_owned(),
            None,
            None,
            CommandRecordStatus::Unavailable,
            parameters(),
            None,
            None,
            None,
            ConversionExitStatus::Succeeded,
            vec![],
            None,
        )
        .unwrap()
        .into_record(header(), vec![artifact("source-a-pdf")], vec![])
        .unwrap_err();
        assert_eq!(error, DomainError::SuccessfulRunWithoutOutput);
    }
}
