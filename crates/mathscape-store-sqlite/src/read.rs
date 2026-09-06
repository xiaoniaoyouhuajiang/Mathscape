use std::collections::{BTreeMap, HashMap};

use mathscape_domain::{
    ActorRef, ArtifactLocation, ArtifactUri, ContentHash, ConversionParameters, ConversionRun,
    EntityEnvelope, EntityHeader, EntityKind, MediaType, PageRange, RevisionId, RightsRecord,
    SchemaVersion, SourceArtifact, SourceRelation, StableId, Timestamp,
};
use rusqlite::{Connection, Row};

use crate::{
    SqliteStoreError, parse_command_record_status, parse_conversion_exit_status, parse_lifecycle,
    parse_location_role, parse_location_status, parse_page_index_base, parse_rights_status,
    parse_storage_kind, rust_u64,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactCatalogRecord {
    entity: EntityEnvelope<SourceArtifact>,
    locations: Vec<ArtifactLocation>,
}

impl ArtifactCatalogRecord {
    pub fn entity(&self) -> &EntityEnvelope<SourceArtifact> {
        &self.entity
    }

    pub fn locations(&self) -> &[ArtifactLocation] {
        &self.locations
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArtifactCatalogSnapshot {
    artifacts: Vec<ArtifactCatalogRecord>,
    conversions: Vec<SourceRelation>,
}

impl ArtifactCatalogSnapshot {
    pub fn artifacts(&self) -> &[ArtifactCatalogRecord] {
        &self.artifacts
    }

    pub fn conversions(&self) -> &[SourceRelation] {
        &self.conversions
    }
}

pub(super) fn catalog_snapshot(
    connection: &Connection,
) -> Result<ArtifactCatalogSnapshot, SqliteStoreError> {
    let transaction = connection.unchecked_transaction()?;
    let artifacts = read_artifacts(&transaction)?;
    let conversions = read_conversions(&transaction)?;
    transaction.commit()?;
    Ok(ArtifactCatalogSnapshot {
        artifacts,
        conversions,
    })
}

struct RawArtifact {
    artifact_id: String,
    content_hash: String,
    byte_size: i64,
    revision_id: String,
    edition_id: Option<String>,
    media_type: String,
    original_filename: Option<String>,
    page_count: Option<i64>,
    rights_status: String,
    permitted_uses: String,
    rights_note: Option<String>,
    schema_version: String,
    lifecycle: String,
    created_at: String,
    created_by: String,
    generation_run: Option<String>,
}

impl RawArtifact {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            artifact_id: row.get(0)?,
            content_hash: row.get(1)?,
            byte_size: row.get(2)?,
            revision_id: row.get(3)?,
            edition_id: row.get(4)?,
            media_type: row.get(5)?,
            original_filename: row.get(6)?,
            page_count: row.get(7)?,
            rights_status: row.get(8)?,
            permitted_uses: row.get(9)?,
            rights_note: row.get(10)?,
            schema_version: row.get(11)?,
            lifecycle: row.get(12)?,
            created_at: row.get(13)?,
            created_by: row.get(14)?,
            generation_run: row.get(15)?,
        })
    }

    fn into_entity(self) -> Result<EntityEnvelope<SourceArtifact>, SqliteStoreError> {
        let stable_id = StableId::parse(self.artifact_id)?;
        let byte_size = rust_u64(self.byte_size, "artifact.byte_size")?;
        let page_count = self
            .page_count
            .map(|value| sqlite_u32(value, "artifact_revision.page_count"))
            .transpose()?;
        let permitted_uses = serde_json::from_str::<Vec<String>>(&self.permitted_uses)?;
        let rights = RightsRecord::new(
            parse_rights_status(&self.rights_status)?,
            permitted_uses,
            self.rights_note,
        )?;
        let artifact = SourceArtifact::new(
            self.edition_id.map(StableId::parse).transpose()?,
            MediaType::parse(self.media_type)?,
            ContentHash::parse(self.content_hash)?,
            byte_size,
            self.original_filename,
            page_count,
            rights,
        )?;
        EntityEnvelope::new(
            EntityHeader::new(
                stable_id,
                RevisionId::parse(self.revision_id)?,
                EntityKind::SourceArtifact,
                SchemaVersion::parse(self.schema_version)?,
                parse_lifecycle(&self.lifecycle)?,
                Timestamp::parse(self.created_at)?,
                ActorRef::parse(self.created_by)?,
                self.generation_run.map(StableId::parse).transpose()?,
            ),
            artifact,
        )
        .map_err(Into::into)
    }
}

struct RawLocation {
    location_id: String,
    artifact_id: String,
    storage_kind: String,
    uri: String,
    priority: i32,
    role: String,
    last_verified_at: Option<String>,
    status: String,
}

impl RawLocation {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            location_id: row.get(0)?,
            artifact_id: row.get(1)?,
            storage_kind: row.get(2)?,
            uri: row.get(3)?,
            priority: row.get(4)?,
            role: row.get(5)?,
            last_verified_at: row.get(6)?,
            status: row.get(7)?,
        })
    }

    fn into_location(self) -> Result<ArtifactLocation, SqliteStoreError> {
        Ok(ArtifactLocation::new(
            StableId::parse(self.location_id)?,
            StableId::parse(self.artifact_id)?,
            parse_storage_kind(&self.storage_kind)?,
            ArtifactUri::parse(self.uri)?,
            self.priority,
            parse_location_role(&self.role)?,
            self.last_verified_at.map(Timestamp::parse).transpose()?,
            parse_location_status(&self.status)?,
        ))
    }
}

fn read_artifacts(connection: &Connection) -> Result<Vec<ArtifactCatalogRecord>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT a.artifact_id, a.content_hash, a.byte_size, r.revision_id,
                r.edition_id, r.media_type, r.original_filename, r.page_count,
                r.rights_status, r.permitted_uses, r.rights_note, r.schema_version,
                r.lifecycle, r.created_at, r.created_by, r.generation_run
         FROM artifacts a
         JOIN artifact_revisions r
           ON r.artifact_id = a.artifact_id AND r.revision_id = a.current_revision
         ORDER BY a.artifact_id",
    )?;
    let rows = statement.query_map([], RawArtifact::from_row)?;
    let mut artifacts = BTreeMap::new();
    for row in rows {
        let entity = row?.into_entity()?;
        let id = entity.header().stable_id().clone();
        if artifacts
            .insert(
                id,
                ArtifactCatalogRecord {
                    entity,
                    locations: vec![],
                },
            )
            .is_some()
        {
            return Err(SqliteStoreError::InvalidSchema(
                "current Artifact query returned duplicate identities",
            ));
        }
    }
    let stored_artifact_count =
        connection.query_row("SELECT COUNT(*) FROM artifacts", [], |row| {
            row.get::<_, i64>(0)
        })?;
    if usize::try_from(stored_artifact_count).ok() != Some(artifacts.len()) {
        return Err(SqliteStoreError::InvalidSchema(
            "Artifact is missing its declared current revision",
        ));
    }
    drop(statement);

    let mut statement = connection.prepare(
        "SELECT location_id, artifact_id, storage_kind, uri, priority, role,
                last_verified_at, current_status
         FROM artifact_locations
         ORDER BY artifact_id, priority, location_id",
    )?;
    let rows = statement.query_map([], RawLocation::from_row)?;
    for row in rows {
        let location = row?.into_location()?;
        let Some(artifact) = artifacts.get_mut(location.artifact()) else {
            return Err(SqliteStoreError::InvalidSchema(
                "Artifact Location refers to an Artifact without a current revision",
            ));
        };
        artifact.locations.push(location);
    }
    Ok(artifacts.into_values().collect())
}

struct RawConversion {
    run_id: String,
    revision_id: String,
    schema_version: String,
    lifecycle: String,
    created_at: String,
    created_by: String,
    generation_run: Option<String>,
    tool_name: String,
    tool_version: Option<String>,
    command_template: Option<String>,
    command_record_status: String,
    page_start: Option<i64>,
    page_end: Option<i64>,
    page_index_base: Option<String>,
    output_format: String,
    redo_inline_math: Option<i64>,
    additional_options: String,
    environment: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    exit_status: String,
    warnings: String,
    cost: Option<String>,
}

impl RawConversion {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            run_id: row.get(0)?,
            revision_id: row.get(1)?,
            schema_version: row.get(2)?,
            lifecycle: row.get(3)?,
            created_at: row.get(4)?,
            created_by: row.get(5)?,
            generation_run: row.get(6)?,
            tool_name: row.get(7)?,
            tool_version: row.get(8)?,
            command_template: row.get(9)?,
            command_record_status: row.get(10)?,
            page_start: row.get(11)?,
            page_end: row.get(12)?,
            page_index_base: row.get(13)?,
            output_format: row.get(14)?,
            redo_inline_math: row.get(15)?,
            additional_options: row.get(16)?,
            environment: row.get(17)?,
            started_at: row.get(18)?,
            finished_at: row.get(19)?,
            exit_status: row.get(20)?,
            warnings: row.get(21)?,
            cost: row.get(22)?,
        })
    }

    fn header(&self) -> Result<EntityHeader, SqliteStoreError> {
        Ok(EntityHeader::new(
            StableId::parse(self.run_id.clone())?,
            RevisionId::parse(self.revision_id.clone())?,
            EntityKind::ConversionRun,
            SchemaVersion::parse(self.schema_version.clone())?,
            parse_lifecycle(&self.lifecycle)?,
            Timestamp::parse(self.created_at.clone())?,
            ActorRef::parse(self.created_by.clone())?,
            self.generation_run
                .clone()
                .map(StableId::parse)
                .transpose()?,
        ))
    }

    fn into_run(self) -> Result<ConversionRun, SqliteStoreError> {
        let page_range = parse_page_range(
            self.page_start,
            self.page_end,
            self.page_index_base.as_deref(),
        )?;
        let redo_inline_math = self
            .redo_inline_math
            .map(|value| sqlite_bool(value, "conversion.redo_inline_math"))
            .transpose()?;
        let additional_options =
            serde_json::from_str::<BTreeMap<String, String>>(&self.additional_options)?;
        let warnings = serde_json::from_str::<Vec<String>>(&self.warnings)?
            .into_iter()
            .map(StableId::parse)
            .collect::<Result<Vec<_>, _>>()?;
        ConversionRun::new(
            self.tool_name,
            self.tool_version,
            self.command_template,
            parse_command_record_status(&self.command_record_status)?,
            ConversionParameters::new(
                page_range,
                self.output_format,
                redo_inline_math,
                additional_options,
            )?,
            self.environment.map(StableId::parse).transpose()?,
            self.started_at.map(Timestamp::parse).transpose()?,
            self.finished_at.map(Timestamp::parse).transpose()?,
            parse_conversion_exit_status(&self.exit_status)?,
            warnings,
            self.cost.map(StableId::parse).transpose()?,
        )
        .map_err(Into::into)
    }
}

struct RawEndpoint {
    role: String,
    ordinal: i64,
    artifact_id: String,
}

impl RawEndpoint {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<((String, String), Self)> {
        Ok((
            (row.get(0)?, row.get(1)?),
            Self {
                role: row.get(2)?,
                ordinal: row.get(3)?,
                artifact_id: row.get(4)?,
            },
        ))
    }
}

fn read_conversions(connection: &Connection) -> Result<Vec<SourceRelation>, SqliteStoreError> {
    let mut endpoint_statement = connection.prepare(
        "SELECT run_id, revision_id, role, ordinal, artifact_id
         FROM conversion_endpoints
         ORDER BY run_id, revision_id, role, ordinal, artifact_id",
    )?;
    let endpoint_rows = endpoint_statement.query_map([], RawEndpoint::from_row)?;
    let mut endpoints: HashMap<(String, String), Vec<RawEndpoint>> = HashMap::new();
    for row in endpoint_rows {
        let (key, endpoint) = row?;
        endpoints.entry(key).or_default().push(endpoint);
    }
    drop(endpoint_statement);

    let mut statement = connection.prepare(
        "SELECT run_id, revision_id, schema_version, lifecycle, created_at,
                created_by, generation_run, tool_name, tool_version, command_template,
                command_record_status, page_start, page_end, page_index_base,
                output_format, redo_inline_math, additional_options, environment,
                started_at, finished_at, exit_status, warnings, cost
         FROM conversion_runs
         ORDER BY run_id, revision_id",
    )?;
    let rows = statement.query_map([], RawConversion::from_row)?;
    let mut conversions = Vec::new();
    let mut run_ids = HashMap::new();
    for row in rows {
        let raw = row?;
        if run_ids
            .insert(raw.run_id.clone(), raw.revision_id.clone())
            .is_some()
        {
            return Err(SqliteStoreError::InvalidSchema(
                "immutable ConversionRun has multiple revisions",
            ));
        }
        let key = (raw.run_id.clone(), raw.revision_id.clone());
        let header = raw.header()?;
        let run = raw.into_run()?;
        let raw_endpoints = endpoints
            .remove(&key)
            .ok_or(SqliteStoreError::InvalidSchema(
                "ConversionRun has no endpoints",
            ))?;
        let (inputs, outputs) = conversion_artifacts(raw_endpoints)?;
        conversions.push(run.into_record(header, inputs, outputs)?);
    }
    if !endpoints.is_empty() {
        return Err(SqliteStoreError::InvalidSchema(
            "conversion endpoint refers to an unknown run revision",
        ));
    }
    Ok(conversions)
}

fn conversion_artifacts(
    endpoints: Vec<RawEndpoint>,
) -> Result<(Vec<StableId>, Vec<StableId>), SqliteStoreError> {
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for endpoint in endpoints {
        let ordinal = sqlite_u32(endpoint.ordinal, "conversion_endpoint.ordinal")?;
        let target = match endpoint.role.as_str() {
            "input" => &mut inputs,
            "output" => &mut outputs,
            _ => return Err(crate::invalid("conversion_endpoint.role", &endpoint.role)),
        };
        target.push((ordinal, StableId::parse(endpoint.artifact_id)?));
    }
    Ok((
        ordered_endpoints(inputs, "conversion input ordinals are not contiguous")?,
        ordered_endpoints(outputs, "conversion output ordinals are not contiguous")?,
    ))
}

fn ordered_endpoints(
    mut endpoints: Vec<(u32, StableId)>,
    error: &'static str,
) -> Result<Vec<StableId>, SqliteStoreError> {
    endpoints.sort_unstable_by_key(|(ordinal, _)| *ordinal);
    for (expected, (actual, _)) in endpoints.iter().enumerate() {
        if usize::try_from(*actual).ok() != Some(expected) {
            return Err(SqliteStoreError::InvalidSchema(error));
        }
    }
    Ok(endpoints
        .into_iter()
        .map(|(_, artifact)| artifact)
        .collect())
}

fn parse_page_range(
    start: Option<i64>,
    end: Option<i64>,
    index_base: Option<&str>,
) -> Result<Option<PageRange>, SqliteStoreError> {
    match (start, end, index_base) {
        (None, None, None) => Ok(None),
        (Some(start), Some(end), Some(index_base)) => Ok(Some(PageRange::new(
            sqlite_u32(start, "conversion.page_start")?,
            sqlite_u32(end, "conversion.page_end")?,
            parse_page_index_base(index_base)?,
        )?)),
        _ => Err(SqliteStoreError::InvalidSchema(
            "conversion page range is only partially populated",
        )),
    }
}

fn sqlite_bool(value: i64, field: &'static str) -> Result<bool, SqliteStoreError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(SqliteStoreError::InvalidStoredValue {
            field,
            value: value.to_string(),
        }),
    }
}

fn sqlite_u32(value: i64, field: &'static str) -> Result<u32, SqliteStoreError> {
    u32::try_from(value).map_err(|_| SqliteStoreError::InvalidStoredValue {
        field,
        value: value.to_string(),
    })
}
