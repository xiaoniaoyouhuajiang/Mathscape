use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{ContentHash, DomainError, EntityKind, EntityPayload, StableId};

use super::ensure_optional_non_blank;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanGranularity {
    Page,
    Block,
    Inline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutKind {
    Heading,
    Paragraph,
    Equation,
    ListItem,
    Table,
    Figure,
    Footnote,
    Other,
}

/// A coordinate normalized to millionths of a page dimension.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct NormalizedCoordinate(u32);

impl NormalizedCoordinate {
    pub const SCALE: u32 = 1_000_000;

    pub fn new(value: u32) -> Result<Self, DomainError> {
        if value > Self::SCALE {
            return Err(DomainError::InvalidBoundingBox);
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct BoundingBox {
    x_min: NormalizedCoordinate,
    y_min: NormalizedCoordinate,
    x_max: NormalizedCoordinate,
    y_max: NormalizedCoordinate,
}

impl BoundingBox {
    pub fn new(
        x_min: NormalizedCoordinate,
        y_min: NormalizedCoordinate,
        x_max: NormalizedCoordinate,
        y_max: NormalizedCoordinate,
    ) -> Result<Self, DomainError> {
        if x_min >= x_max || y_min >= y_max {
            return Err(DomainError::InvalidBoundingBox);
        }
        Ok(Self {
            x_min,
            y_min,
            x_max,
            y_max,
        })
    }

    pub fn x_min(self) -> NormalizedCoordinate {
        self.x_min
    }

    pub fn y_min(self) -> NormalizedCoordinate {
        self.y_min
    }

    pub fn x_max(self) -> NormalizedCoordinate {
        self.x_max
    }

    pub fn y_max(self) -> NormalizedCoordinate {
        self.y_max
    }
}

/// Three page-number systems that must never be implicitly conflated.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PageCoordinate {
    artifact: StableId,
    pdf_page_index: u32,
    marker_page_id: Option<u32>,
    printed_page_label: Option<String>,
}

impl PageCoordinate {
    pub fn new(
        artifact: StableId,
        pdf_page_index: u32,
        marker_page_id: Option<u32>,
        printed_page_label: Option<String>,
    ) -> Result<Self, DomainError> {
        ensure_optional_non_blank(
            printed_page_label.as_deref(),
            "page_coordinate.printed_page_label",
        )?;
        Ok(Self {
            artifact,
            pdf_page_index,
            marker_page_id,
            printed_page_label,
        })
    }

    pub fn artifact(&self) -> &StableId {
        &self.artifact
    }

    pub fn pdf_page_index(&self) -> u32 {
        self.pdf_page_index
    }

    pub fn marker_page_id(&self) -> Option<u32> {
        self.marker_page_id
    }

    pub fn printed_page_label(&self) -> Option<&str> {
        self.printed_page_label.as_deref()
    }
}

/// Half-open UTF-8 byte range `[start, end)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ByteRange {
    start: u64,
    end: u64,
}

impl ByteRange {
    pub fn new(start: u64, end: u64) -> Result<Self, DomainError> {
        if start >= end {
            return Err(DomainError::InvalidRange {
                field: "markdown.byte_range",
                start,
                end,
            });
        }
        Ok(Self { start, end })
    }

    pub fn start(self) -> u64 {
        self.start
    }

    pub fn end(self) -> u64 {
        self.end
    }
}

/// Inclusive, one-based displayed line range used only for human navigation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DisplayedLineRange {
    start: u32,
    end: u32,
}

impl DisplayedLineRange {
    pub fn new(start: u32, end: u32) -> Result<Self, DomainError> {
        if start == 0 || start > end {
            return Err(DomainError::InvalidRange {
                field: "markdown.displayed_line_range",
                start: u64::from(start),
                end: u64::from(end),
            });
        }
        Ok(Self { start, end })
    }

    pub fn start(self) -> u32 {
        self.start
    }

    pub fn end(self) -> u32 {
        self.end
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MarkdownLocator {
    artifact: StableId,
    byte_range: ByteRange,
    displayed_lines: Option<DisplayedLineRange>,
}

impl MarkdownLocator {
    pub fn new(
        artifact: StableId,
        byte_range: ByteRange,
        displayed_lines: Option<DisplayedLineRange>,
    ) -> Self {
        Self {
            artifact,
            byte_range,
            displayed_lines,
        }
    }

    pub fn artifact(&self) -> &StableId {
        &self.artifact
    }

    pub fn byte_range(&self) -> ByteRange {
        self.byte_range
    }

    pub fn displayed_lines(&self) -> Option<DisplayedLineRange> {
        self.displayed_lines
    }
}

/// Smallest persistent, physically locatable evidence region.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SourceSpan {
    granularity: SpanGranularity,
    layout: LayoutKind,
    page: Option<PageCoordinate>,
    bounding_boxes: Vec<BoundingBox>,
    marker_block_ids: Vec<String>,
    markdown: Option<MarkdownLocator>,
    exact_text_artifact: Option<StableId>,
    normalized_text_hash: Option<ContentHash>,
}

impl SourceSpan {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        granularity: SpanGranularity,
        layout: LayoutKind,
        page: Option<PageCoordinate>,
        bounding_boxes: Vec<BoundingBox>,
        marker_block_ids: Vec<String>,
        markdown: Option<MarkdownLocator>,
        exact_text_artifact: Option<StableId>,
        normalized_text_hash: Option<ContentHash>,
    ) -> Result<Self, DomainError> {
        if page.is_none() && markdown.is_none() {
            return Err(DomainError::EmptyCollection {
                field: "source_span.locators",
            });
        }
        if !bounding_boxes.is_empty() && page.is_none() {
            return Err(DomainError::InconsistentState {
                field: "source_span.bounding_boxes_without_page",
            });
        }
        if granularity == SpanGranularity::Page && page.is_none() {
            return Err(DomainError::InconsistentState {
                field: "source_span.page_granularity",
            });
        }

        let mut seen_block_ids = HashSet::new();
        for marker_block_id in &marker_block_ids {
            if marker_block_id.trim().is_empty() {
                return Err(DomainError::EmptyValue {
                    field: "source_span.marker_block_id",
                });
            }
            if !seen_block_ids.insert(marker_block_id) {
                return Err(DomainError::DuplicateValue {
                    field: "source_span.duplicate_marker_block_id",
                    value: marker_block_id.clone(),
                });
            }
        }

        if granularity == SpanGranularity::Block
            && bounding_boxes.is_empty()
            && marker_block_ids.is_empty()
        {
            return Err(DomainError::EmptyCollection {
                field: "source_span.block_locator",
            });
        }
        if granularity == SpanGranularity::Inline && bounding_boxes.is_empty() && markdown.is_none()
        {
            return Err(DomainError::EmptyCollection {
                field: "source_span.inline_locator",
            });
        }

        Ok(Self {
            granularity,
            layout,
            page,
            bounding_boxes,
            marker_block_ids,
            markdown,
            exact_text_artifact,
            normalized_text_hash,
        })
    }

    /// Preferred artifact for source navigation. Markdown is primary when
    /// present because it carries the exact UTF-8 range used by Discovery.
    pub fn primary_artifact(&self) -> &StableId {
        self.markdown
            .as_ref()
            .map(MarkdownLocator::artifact)
            .or_else(|| self.page.as_ref().map(PageCoordinate::artifact))
            .expect("SourceSpan requires at least one locator")
    }

    pub fn granularity(&self) -> SpanGranularity {
        self.granularity
    }

    pub fn layout(&self) -> LayoutKind {
        self.layout
    }

    pub fn page(&self) -> Option<&PageCoordinate> {
        self.page.as_ref()
    }

    pub fn bounding_boxes(&self) -> &[BoundingBox] {
        &self.bounding_boxes
    }

    pub fn marker_block_ids(&self) -> &[String] {
        &self.marker_block_ids
    }

    pub fn markdown(&self) -> Option<&MarkdownLocator> {
        self.markdown.as_ref()
    }

    pub fn exact_text_artifact(&self) -> Option<&StableId> {
        self.exact_text_artifact.as_ref()
    }

    pub fn normalized_text_hash(&self) -> Option<&ContentHash> {
        self.normalized_text_hash.as_ref()
    }
}

impl EntityPayload for SourceSpan {
    fn entity_kind(&self) -> EntityKind {
        EntityKind::SourceSpan
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_page_span_preserves_pdf_and_markdown_coordinates() {
        let pdf = StableId::parse("emir-fixture:artifact/source-a-pdf").unwrap();
        let markdown = StableId::parse("emir-fixture:artifact/source-a-markdown").unwrap();
        let span = SourceSpan::new(
            SpanGranularity::Page,
            LayoutKind::Paragraph,
            Some(PageCoordinate::new(pdf, 42, Some(42), Some("31".to_owned())).unwrap()),
            vec![],
            vec![],
            Some(MarkdownLocator::new(
                markdown,
                ByteRange::new(305, 708).unwrap(),
                Some(DisplayedLineRange::new(3, 9).unwrap()),
            )),
            None,
            Some(
                ContentHash::parse(
                    "sha256:1f9b963b9ea40f0a477087cea861a38763d6b1396791948a9bdcafbfff8f72df",
                )
                .unwrap(),
            ),
        )
        .unwrap();

        assert_eq!(span.page().unwrap().pdf_page_index(), 42);
        assert_eq!(span.markdown().unwrap().byte_range().start(), 305);
        assert_eq!(span.primary_artifact(), span.markdown().unwrap().artifact());
    }

    #[test]
    fn markdown_locator_can_independently_ground_a_span() {
        let markdown = StableId::parse("mathscape:artifact/markdown").unwrap();
        let span = SourceSpan::new(
            SpanGranularity::Inline,
            LayoutKind::Paragraph,
            None,
            vec![],
            vec!["block-1".into()],
            Some(MarkdownLocator::new(
                markdown.clone(),
                ByteRange::new(5, 12).unwrap(),
                None,
            )),
            None,
            None,
        )
        .unwrap();
        assert!(span.page().is_none());
        assert_eq!(span.primary_artifact(), &markdown);
    }

    #[test]
    fn source_span_requires_a_real_locator() {
        assert!(matches!(
            SourceSpan::new(
                SpanGranularity::Inline,
                LayoutKind::Paragraph,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
            ),
            Err(DomainError::EmptyCollection {
                field: "source_span.locators"
            })
        ));
    }
}
