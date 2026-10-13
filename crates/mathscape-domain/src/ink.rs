//! P0 experimental ink interchange, independent of map registration and devices.
use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const INK_LAB_VERSION: &str = "mathscape-ink-lab-1";
pub const INK_SNAPSHOT_VERSION: &str = "mathscape-ink-snapshot-2";
pub const INK_LEGACY_SNAPSHOT_VERSION: &str = "mathscape-ink-snapshot-1";
pub const INK_LAB_MAX_POINTS: usize = 500_000;
pub const INK_LAB_MAX_OPERATIONS: usize = 20_000;
const JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InkLabDocument {
    pub version: String,
    pub document_id: String,
    pub title: String,
    pub created_at: String,
    pub exported_at: String,
    pub pages: Vec<InkLabPage>,
    pub capture_segments: Vec<InkLabCaptureSegment>,
    pub operations: Vec<InkLabOperation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InkLabDocumentMetadata {
    pub version: String,
    pub document_id: String,
    pub title: String,
    pub created_at: String,
    pub exported_at: String,
    pub pages: Vec<InkLabPage>,
    pub capture_segments: Vec<InkLabCaptureSegment>,
}

impl InkLabDocument {
    pub fn metadata(&self) -> InkLabDocumentMetadata {
        InkLabDocumentMetadata {
            version: self.version.clone(),
            document_id: self.document_id.clone(),
            title: self.title.clone(),
            created_at: self.created_at.clone(),
            exported_at: self.exported_at.clone(),
            pages: self.pages.clone(),
            capture_segments: self.capture_segments.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InkSnapshotManifest {
    pub version: String,
    pub document_id: String,
    pub through_document_seq: u32,
    pub document: InkLabDocument,
    pub visible_stroke_ids: Vec<String>,
    pub point_count: usize,
}

impl InkSnapshotManifest {
    pub fn from_document(document: &InkLabDocument, through_document_seq: u64) -> Result<Self> {
        require(
            through_document_seq <= document.operations.len() as u64,
            "snapshot.head",
        )?;
        let mut prefix = document.clone();
        prefix.operations.truncate(through_document_seq as usize);
        let projection = prefix.validate()?;
        Ok(Self {
            version: INK_SNAPSHOT_VERSION.to_owned(),
            document_id: prefix.document_id.clone(),
            through_document_seq: through_document_seq as u32,
            document: prefix,
            visible_stroke_ids: projection.visible_stroke_ids,
            point_count: projection.point_count,
        })
    }

    pub fn validate(&self) -> Result<()> {
        require(
            self.version == INK_SNAPSHOT_VERSION || self.version == INK_LEGACY_SNAPSHOT_VERSION,
            "snapshot.version",
        )?;
        require(
            self.document_id == self.document.document_id,
            "snapshot.document_id",
        )?;
        require(
            self.through_document_seq as usize == self.document.operations.len(),
            "snapshot.head",
        )?;
        let projection = self.document.validate()?;
        // v1's writer omitted partial-erase fragments from its summary. Its
        // immutable bytes still contain the complete, valid operation history.
        let originals = self
            .document
            .operations
            .iter()
            .filter_map(|operation| match operation {
                InkLabOperation::Stroke { stroke, .. } => Some(stroke.id.as_str()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let legacy_visible = projection
            .visible_stroke_ids
            .iter()
            .filter(|id| originals.contains(id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let compatible_summary = self.version == INK_LEGACY_SNAPSHOT_VERSION
            && self.visible_stroke_ids == legacy_visible;
        require(
            (projection.visible_stroke_ids == self.visible_stroke_ids || compatible_summary)
                && projection.point_count == self.point_count,
            "snapshot.projection",
        )
    }

    /// Bind a whole-stroke selection to this fixed prefix, including fragments
    /// produced by partial erasing. Displayed page numbers are not identities.
    pub fn validate_selection(&self, page_id: &str, stroke_ids: &[String]) -> Result<()> {
        self.validate()?;
        require(
            self.document.pages.iter().any(|page| page.id == page_id),
            "selection.page_id",
        )?;
        require(
            !stroke_ids.is_empty() && stroke_ids.len() <= INK_LAB_MAX_POINTS,
            "selection.stroke_ids",
        )?;
        require(
            stroke_ids.iter().collect::<BTreeSet<_>>().len() == stroke_ids.len(),
            "selection.duplicate_stroke_id",
        )?;
        let projection = self.document.validate()?;
        let visible = projection
            .visible_stroke_ids
            .iter()
            .collect::<BTreeSet<_>>();
        let mut pages = BTreeMap::new();
        for operation in &self.document.operations {
            match operation {
                InkLabOperation::Stroke { stroke, .. } => {
                    pages.insert(&stroke.id, &stroke.page_id);
                }
                InkLabOperation::PartialErase { fragments, .. } => {
                    for stroke in fragments {
                        pages.insert(&stroke.id, &stroke.page_id);
                    }
                }
                _ => {}
            }
        }
        for id in stroke_ids {
            require(
                visible.contains(id) && pages.get(id).is_some_and(|page| page.as_str() == page_id),
                "selection.visible_stroke_on_page",
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterInkEvidenceRequest {
    pub request_id: String,
    pub snapshot_id: String,
    pub expected_artifact_id: crate::StableId,
    pub expected_content_hash: crate::ContentHash,
    pub page_id: String,
    pub stroke_ids: Vec<String>,
    pub selection_mode: crate::InkSelectionMode,
}

impl RegisterInkEvidenceRequest {
    pub fn canonicalized(&self) -> Result<Self> {
        text(&self.request_id, 200, "evidence.request_id")?;
        text(&self.snapshot_id, 200, "evidence.snapshot_id")?;
        text(&self.page_id, 200, "evidence.page_id")?;
        require(
            !self.stroke_ids.is_empty() && self.stroke_ids.len() <= INK_LAB_MAX_POINTS,
            "evidence.stroke_ids",
        )?;
        for id in &self.stroke_ids {
            text(id, 200, "evidence.stroke_id")?;
        }
        let mut canonical = self.clone();
        canonical.stroke_ids.sort();
        canonical.stroke_ids.dedup();
        Ok(canonical)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InkLabPage {
    pub id: String,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InkLabCaptureSegment {
    pub id: String,
    pub started_at: String,
    pub user_agent: String,
    pub secure_context: bool,
    pub coalesced_events: bool,
    pub pointer_raw_update: bool,
    pub device_pixel_ratio: f64,
    pub time_origin_ms: f64,
    pub pressure_capability: InkLabCapability,
    pub tilt_capability: InkLabCapability,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InkLabCapability {
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InkLabPoint {
    pub x: f64,
    pub y: f64,
    pub t_us: u64,
    #[serde(deserialize_with = "required_nullable")]
    #[schemars(required, with = "RequiredNullableNumber")]
    pub pressure: Option<f64>,
    #[serde(deserialize_with = "required_nullable")]
    #[schemars(required, with = "RequiredNullableNumber")]
    pub tilt_x_deg: Option<f64>,
    #[serde(deserialize_with = "required_nullable")]
    #[schemars(required, with = "RequiredNullableNumber")]
    pub tilt_y_deg: Option<f64>,
    pub buttons: u16,
}

fn required_nullable<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<f64>, D::Error> {
    Option::<f64>::deserialize(deserializer)
}

struct RequiredNullableNumber;
impl JsonSchema for RequiredNullableNumber {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "RequiredNullableNumber".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        <Option<f64>>::json_schema(generator)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InkLabBrush {
    pub renderer: String,
    pub color: String,
    pub width: f64,
    pub smoothing: f64,
    pub streamline: f64,
    pub thinning: f64,
    pub simulate_pressure: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InkLabPointerType {
    Pen,
    Mouse,
    Touch,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InkLabStrokeStatus {
    Completed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InkLabStroke {
    pub id: String,
    pub page_id: String,
    pub capture_segment_id: String,
    pub pointer_type: InkLabPointerType,
    pub status: InkLabStrokeStatus,
    pub brush: InkLabBrush,
    pub points: Vec<InkLabPoint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InkTransform {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InkLabOperation {
    Stroke {
        seq: u32,
        command_id: String,
        stroke: InkLabStroke,
    },
    Erase {
        seq: u32,
        command_id: String,
        stroke_ids: Vec<String>,
    },
    PartialErase {
        seq: u32,
        command_id: String,
        source_stroke_id: String,
        fragments: Vec<InkLabStroke>,
    },
    Transform {
        seq: u32,
        command_id: String,
        stroke_ids: Vec<String>,
        matrix: InkTransform,
    },
    PageAdd {
        seq: u32,
        command_id: String,
        page: InkLabPage,
    },
    Undo {
        seq: u32,
        command_id: String,
        target_command_id: String,
    },
    Redo {
        seq: u32,
        command_id: String,
        target_command_id: String,
    },
}

impl InkLabOperation {
    pub fn seq(&self) -> u32 {
        self.identity().0
    }

    pub fn command_id(&self) -> &str {
        self.identity().1
    }

    fn identity(&self) -> (u32, &str) {
        match self {
            Self::Stroke {
                seq, command_id, ..
            }
            | Self::Erase {
                seq, command_id, ..
            }
            | Self::PartialErase {
                seq, command_id, ..
            }
            | Self::Transform {
                seq, command_id, ..
            }
            | Self::PageAdd {
                seq, command_id, ..
            }
            | Self::Undo {
                seq, command_id, ..
            }
            | Self::Redo {
                seq, command_id, ..
            } => (*seq, command_id),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InkLabProjection {
    pub visible_stroke_ids: Vec<String>,
    pub undo: Vec<String>,
    pub redo: Vec<String>,
    pub point_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InkLabValidationError(pub String);

impl std::fmt::Display for InkLabValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for InkLabValidationError {}

type Result<T> = std::result::Result<T, InkLabValidationError>;
fn require(condition: bool, field: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(InkLabValidationError(field.to_owned()))
    }
}
fn text(value: &str, max: usize, field: &str) -> Result<()> {
    require(
        !value.trim().is_empty()
            && value.encode_utf16().count() <= max
            && !value.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}'),
        field,
    )
}
fn numeric(value: f64, min: f64, max: f64, field: &str) -> Result<()> {
    require(value.is_finite() && value >= min && value <= max, field)
}

fn validate_stroke_data(
    stroke: &InkLabStroke,
    pages: &BTreeSet<&str>,
    segments: &BTreeSet<&str>,
    point_count: &mut usize,
) -> Result<()> {
    require(pages.contains(stroke.page_id.as_str()), "stroke.page_id")?;
    require(
        segments.contains(stroke.capture_segment_id.as_str()),
        "stroke.capture_segment_id",
    )?;
    let brush = &stroke.brush;
    require(
        brush.renderer == "perfect-freehand@1.2.3" && !brush.simulate_pressure,
        "brush.renderer",
    )?;
    require(
        brush.color.len() == 7
            && brush.color.starts_with('#')
            && brush.color[1..].bytes().all(|b| b.is_ascii_hexdigit()),
        "brush.color",
    )?;
    numeric(brush.width, 0.1, 100.0, "brush.width")?;
    numeric(brush.smoothing, 0.0, 1.0, "brush.smoothing")?;
    numeric(brush.streamline, 0.0, 1.0, "brush.streamline")?;
    numeric(brush.thinning, -1.0, 1.0, "brush.thinning")?;
    require(
        !stroke.points.is_empty() && stroke.points.len() <= 100_000,
        "stroke.points",
    )?;
    *point_count += stroke.points.len();
    require(*point_count <= INK_LAB_MAX_POINTS, "point_count")?;
    let mut previous = 0;
    for point in &stroke.points {
        numeric(point.x, -1_000_000.0, 1_000_000.0, "point.x")?;
        numeric(point.y, -1_000_000.0, 1_000_000.0, "point.y")?;
        require(
            point.t_us >= previous && point.t_us <= JS_SAFE_INTEGER,
            "point.t_us",
        )?;
        previous = point.t_us;
        if let Some(value) = point.pressure {
            numeric(value, 0.0, 1.0, "point.pressure")?;
        }
        if let Some(value) = point.tilt_x_deg {
            numeric(value, -90.0, 90.0, "point.tilt_x_deg")?;
        }
        if let Some(value) = point.tilt_y_deg {
            numeric(value, -90.0, 90.0, "point.tilt_y_deg")?;
        }
    }
    Ok(())
}

impl InkLabDocument {
    /// Deserialization is shape checking; this method checks references and history.
    pub fn validate(&self) -> Result<InkLabProjection> {
        require(self.version == INK_LAB_VERSION, "version")?;
        for (field, value) in [
            ("document_id", &self.document_id),
            ("title", &self.title),
            ("created_at", &self.created_at),
            ("exported_at", &self.exported_at),
        ] {
            text(value, 200, field)?;
        }
        require(!self.pages.is_empty() && self.pages.len() <= 100, "pages")?;
        require(self.capture_segments.len() <= 1000, "capture_segments")?;
        require(
            self.operations.len() <= INK_LAB_MAX_OPERATIONS,
            "operations",
        )?;
        let mut pages = BTreeSet::new();
        for page in &self.pages {
            text(&page.id, 200, "page.id")?;
            require(pages.insert(page.id.as_str()), "duplicate page")?;
            numeric(page.width, 1.0, 20_000.0, "page.width")?;
            numeric(page.height, 1.0, 20_000.0, "page.height")?;
        }
        let mut segments = BTreeSet::new();
        for segment in &self.capture_segments {
            text(&segment.id, 200, "segment.id")?;
            require(segments.insert(segment.id.as_str()), "duplicate segment")?;
            text(&segment.started_at, 200, "segment.started_at")?;
            text(&segment.user_agent, 2000, "segment.user_agent")?;
            numeric(segment.device_pixel_ratio, 0.1, 20.0, "device_pixel_ratio")?;
            numeric(
                segment.time_origin_ms,
                0.0,
                JS_SAFE_INTEGER as f64,
                "time_origin_ms",
            )?;
        }
        let mut commands = BTreeSet::new();
        let mut strokes = BTreeSet::new();
        let mut active = BTreeMap::new();
        let mut undo = Vec::new();
        let mut redo = Vec::new();
        let mut point_count = 0;
        for (index, operation) in self.operations.iter().enumerate() {
            let (seq, command_id) = operation.identity();
            require(seq as usize == index + 1, "operation.seq")?;
            text(command_id, 200, "command_id")?;
            require(commands.insert(command_id), "duplicate command")?;
            match operation {
                InkLabOperation::Stroke { stroke, .. } => {
                    text(&stroke.id, 200, "stroke.id")?;
                    require(strokes.insert(stroke.id.as_str()), "duplicate stroke")?;
                    validate_stroke_data(stroke, &pages, &segments, &mut point_count)?;
                    active.insert(command_id, true);
                    undo.push(command_id.to_owned());
                    redo.clear();
                }
                InkLabOperation::Erase { stroke_ids, .. } => {
                    require(
                        !stroke_ids.is_empty() && stroke_ids.len() <= INK_LAB_MAX_OPERATIONS,
                        "erase.stroke_ids",
                    )?;
                    let mut seen = BTreeSet::new();
                    for id in stroke_ids {
                        text(id, 200, "erase.stroke_id")?;
                        require(
                            strokes.contains(id.as_str()) && seen.insert(id),
                            "erase.reference",
                        )?;
                    }
                    active.insert(command_id, true);
                    undo.push(command_id.to_owned());
                    redo.clear();
                }
                InkLabOperation::PartialErase {
                    source_stroke_id,
                    fragments,
                    ..
                } => {
                    text(source_stroke_id, 200, "partial_erase.source_stroke_id")?;
                    require(
                        strokes.contains(source_stroke_id.as_str()),
                        "partial_erase.reference",
                    )?;
                    require(
                        !fragments.is_empty() && fragments.len() <= 1000,
                        "partial_erase.fragments",
                    )?;
                    for fragment in fragments {
                        text(&fragment.id, 200, "partial_erase.fragment.id")?;
                        require(strokes.insert(fragment.id.as_str()), "duplicate fragment")?;
                        validate_stroke_data(fragment, &pages, &segments, &mut point_count)?;
                    }
                    active.insert(command_id, true);
                    undo.push(command_id.to_owned());
                    redo.clear();
                }
                InkLabOperation::Transform {
                    stroke_ids, matrix, ..
                } => {
                    require(
                        !stroke_ids.is_empty() && stroke_ids.len() <= INK_LAB_MAX_OPERATIONS,
                        "transform.stroke_ids",
                    )?;
                    for id in stroke_ids {
                        text(id, 200, "transform.stroke_id")?;
                        require(strokes.contains(id.as_str()), "transform.reference")?;
                    }
                    for value in [matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f] {
                        numeric(value, -1_000_000.0, 1_000_000.0, "transform.matrix")?;
                    }
                    active.insert(command_id, true);
                    undo.push(command_id.to_owned());
                    redo.clear();
                }
                InkLabOperation::PageAdd { page, .. } => {
                    text(&page.id, 200, "page_add.page.id")?;
                    require(pages.contains(page.id.as_str()), "page_add.page")?;
                    numeric(page.width, 1.0, 20_000.0, "page_add.page.width")?;
                    numeric(page.height, 1.0, 20_000.0, "page_add.page.height")?;
                    active.insert(command_id, true);
                    // Page creation is structural and is not removed by stroke undo.
                }
                InkLabOperation::Undo {
                    target_command_id, ..
                }
                | InkLabOperation::Redo {
                    target_command_id, ..
                } => {
                    text(target_command_id, 200, "history.target")?;
                    let redoing = matches!(operation, InkLabOperation::Redo { .. });
                    let (from, to) = if redoing {
                        (&mut redo, &mut undo)
                    } else {
                        (&mut undo, &mut redo)
                    };
                    require(from.last() == Some(target_command_id), "history.target")?;
                    from.pop();
                    to.push(target_command_id.clone());
                    active.insert(target_command_id.as_str(), redoing);
                }
            }
        }
        // Preserve creation order, rather than BTreeMap's lexicographic ordering.
        let mut visible = BTreeSet::new();
        for operation in &self.operations {
            let (_, command_id) = operation.identity();
            if active.get(command_id) != Some(&true) {
                continue;
            }
            match operation {
                InkLabOperation::Stroke { stroke, .. } => {
                    visible.insert(stroke.id.as_str());
                }
                InkLabOperation::Erase { stroke_ids, .. } => {
                    for id in stroke_ids {
                        visible.remove(id.as_str());
                    }
                }
                InkLabOperation::PartialErase {
                    source_stroke_id,
                    fragments,
                    ..
                } => {
                    visible.remove(source_stroke_id.as_str());
                    for fragment in fragments {
                        visible.insert(fragment.id.as_str());
                    }
                }
                _ => {}
            }
        }
        let mut visible_stroke_ids = Vec::new();
        for operation in &self.operations {
            match operation {
                InkLabOperation::Stroke { stroke, .. } if visible.contains(stroke.id.as_str()) => {
                    visible_stroke_ids.push(stroke.id.clone())
                }
                InkLabOperation::PartialErase { fragments, .. } => {
                    for fragment in fragments {
                        if visible.contains(fragment.id.as_str()) {
                            visible_stroke_ids.push(fragment.id.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(InkLabProjection {
            visible_stroke_ids,
            undo,
            redo,
            point_count,
        })
    }
}
