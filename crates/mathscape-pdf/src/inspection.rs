use std::{collections::HashSet, fmt, path::Path};

use lopdf::{Document, Object, ObjectId};
use pdf_inspector::{DetectionConfig, PdfOptions, ProcessMode, ScanStrategy};

use crate::{PdfError, PdfPageGeometry, PdfPageIndex, PdfPointRect, PdfRotation, PdfSelection};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClassificationMode {
    /// Evenly sample up to eight pages. Suitable for interactive routing.
    Fast,
    /// Inspect every page. Required before persisting exact page-level routes.
    Exhaustive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanCoverage {
    Sampled,
    Exhaustive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfContentKind {
    TextBased,
    Scanned,
    ImageBased,
    Mixed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OcrReason {
    Scanned,
    NoText,
    VectorText,
    SuspectedGarbledText,
    Other(String),
}

impl OcrReason {
    fn from_backend(value: String) -> Self {
        match value.as_str() {
            pdf_inspector::OCR_REASON_SCANNED => Self::Scanned,
            pdf_inspector::OCR_REASON_NO_TEXT => Self::NoText,
            pdf_inspector::OCR_REASON_VECTOR_TEXT => Self::VectorText,
            pdf_inspector::OCR_REASON_SUSPECTED_GARBLED_TEXT => Self::SuspectedGarbledText,
            _ => Self::Other(value),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageOcrRecommendation {
    page: PdfPageIndex,
    reasons: Vec<OcrReason>,
}

impl PageOcrRecommendation {
    pub const fn page(&self) -> PdfPageIndex {
        self.page
    }

    pub fn reasons(&self) -> &[OcrReason] {
        &self.reasons
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfClassification {
    kind: PdfContentKind,
    page_count: u32,
    confidence: f32,
    title: Option<String>,
    coverage: ScanCoverage,
    ocr_recommendations: Vec<PageOcrRecommendation>,
}

impl PdfClassification {
    pub const fn kind(&self) -> PdfContentKind {
        self.kind
    }

    pub const fn page_count(&self) -> u32 {
        self.page_count
    }

    pub const fn confidence(&self) -> f32 {
        self.confidence
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub const fn coverage(&self) -> ScanCoverage {
        self.coverage
    }

    pub fn ocr_recommendations(&self) -> &[PageOcrRecommendation] {
        &self.ocr_recommendations
    }

    pub fn ocr_selection(&self) -> Result<Option<PdfSelection>, PdfError> {
        if self.ocr_recommendations.is_empty() {
            return Ok(None);
        }
        PdfSelection::from_pages(
            self.page_count,
            self.ocr_recommendations
                .iter()
                .map(PageOcrRecommendation::page)
                .collect(),
        )
        .map(Some)
    }
}

#[derive(Clone, Default)]
pub struct PdfOpenOptions {
    password: Option<String>,
}

impl PdfOpenOptions {
    pub const fn new() -> Self {
        Self { password: None }
    }

    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    pub(crate) fn password_ref(&self) -> Option<&str> {
        self.password.as_deref()
    }
}

impl fmt::Debug for PdfOpenOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PdfOpenOptions")
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfProbe {
    page_count: u32,
    title: Option<String>,
    pages: Vec<PdfPageGeometry>,
}

impl PdfProbe {
    pub const fn page_count(&self) -> u32 {
        self.page_count
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn pages(&self) -> &[PdfPageGeometry] {
        &self.pages
    }

    pub fn page(&self, index: PdfPageIndex) -> Option<&PdfPageGeometry> {
        self.pages.get(index.get() as usize)
    }
}

/// Stateless adapter around the embedded PDF engines.
#[derive(Clone, Copy, Debug, Default)]
pub struct PdfEngine;

impl PdfEngine {
    pub fn probe(
        &self,
        path: impl AsRef<Path>,
        open_options: &PdfOpenOptions,
    ) -> Result<PdfProbe, PdfError> {
        let path = path.as_ref();
        let classification = self.classify(path, open_options, ClassificationMode::Fast)?;
        let document = load_document(path, open_options)?;
        let document_page_count = u32::try_from(document.get_pages().len())
            .map_err(|_| PdfError::Parse("page count exceeds u32".to_owned()))?;
        if classification.page_count != document_page_count {
            return Err(PdfError::PageCountMismatch {
                inspector: classification.page_count,
                document: document_page_count,
            });
        }
        if document_page_count == 0 {
            return Err(PdfError::EmptyDocument);
        }

        let pages = page_geometries(&document)?;
        Ok(PdfProbe {
            page_count: document_page_count,
            title: classification.title,
            pages,
        })
    }

    pub fn classify(
        &self,
        path: impl AsRef<Path>,
        open_options: &PdfOpenOptions,
        mode: ClassificationMode,
    ) -> Result<PdfClassification, PdfError> {
        let (strategy, coverage) = match mode {
            ClassificationMode::Fast => (ScanStrategy::Sample(8), ScanCoverage::Sampled),
            ClassificationMode::Exhaustive => (ScanStrategy::Full, ScanCoverage::Exhaustive),
        };
        let detection = DetectionConfig {
            strategy,
            ..DetectionConfig::default()
        };
        let mut options = PdfOptions::new()
            .mode(ProcessMode::DetectOnly)
            .detection(detection);
        if let Some(password) = open_options.password_ref() {
            options = options.password(password);
        }
        let result = pdf_inspector::process_pdf_with_options(path, options)
            .map_err(PdfError::from_inspector)?;
        if result.page_count == 0 {
            return Err(PdfError::EmptyDocument);
        }

        let mut reasons_by_page = result
            .ocr_reasons_by_page
            .into_iter()
            .map(|entry| (entry.page, entry.reasons))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut ocr_recommendations = Vec::with_capacity(result.pages_needing_ocr.len());
        for one_based_page in result.pages_needing_ocr {
            let page = PdfPageIndex::from_one_based(one_based_page, result.page_count)?;
            let reasons = reasons_by_page
                .remove(&one_based_page)
                .unwrap_or_default()
                .into_iter()
                .map(OcrReason::from_backend)
                .collect();
            ocr_recommendations.push(PageOcrRecommendation { page, reasons });
        }
        ocr_recommendations.sort_unstable_by_key(|recommendation| recommendation.page);

        Ok(PdfClassification {
            kind: map_content_kind(result.pdf_type),
            page_count: result.page_count,
            confidence: result.confidence,
            title: result.title,
            coverage,
            ocr_recommendations,
        })
    }
}

fn map_content_kind(kind: pdf_inspector::PdfType) -> PdfContentKind {
    match kind {
        pdf_inspector::PdfType::TextBased => PdfContentKind::TextBased,
        pdf_inspector::PdfType::Scanned => PdfContentKind::Scanned,
        pdf_inspector::PdfType::ImageBased => PdfContentKind::ImageBased,
        pdf_inspector::PdfType::Mixed => PdfContentKind::Mixed,
    }
}

fn load_document(path: &Path, options: &PdfOpenOptions) -> Result<Document, PdfError> {
    let result = match options.password_ref() {
        Some(password) => Document::load_with_password(path, password),
        None => Document::load(path),
    };
    result.map_err(PdfError::from_lopdf)
}

fn page_geometries(document: &Document) -> Result<Vec<PdfPageGeometry>, PdfError> {
    let pages = document.get_pages();
    let page_count = u32::try_from(pages.len())
        .map_err(|_| PdfError::Parse("page count exceeds u32".to_owned()))?;
    pages
        .into_iter()
        .map(|(one_based_page, object_id)| {
            let page = PdfPageIndex::from_one_based(one_based_page, page_count)?;
            let media_box = inherited_page_value(document, object_id, b"MediaBox")?.ok_or(
                PdfError::MissingPageAttribute {
                    page: page.get(),
                    attribute: "MediaBox",
                },
            )?;
            let values = media_box.as_array().map_err(PdfError::from_lopdf)?;
            if values.len() < 4 {
                return Err(PdfError::InvalidRectangle(
                    "MediaBox must contain four numbers",
                ));
            }
            let media_box = PdfPointRect::new(
                values[0].as_float().map_err(PdfError::from_lopdf)?,
                values[1].as_float().map_err(PdfError::from_lopdf)?,
                values[2].as_float().map_err(PdfError::from_lopdf)?,
                values[3].as_float().map_err(PdfError::from_lopdf)?,
            )?;
            let rotation = inherited_page_value(document, object_id, b"Rotate")?
                .map(Object::as_i64)
                .transpose()
                .map_err(PdfError::from_lopdf)?
                .unwrap_or(0);
            PdfPageGeometry::new(page, media_box, PdfRotation::from_degrees(rotation)?)
        })
        .collect()
}

fn inherited_page_value<'a>(
    document: &'a Document,
    mut object_id: ObjectId,
    key: &[u8],
) -> Result<Option<&'a Object>, PdfError> {
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(object_id) {
            return Err(PdfError::InvalidStructure);
        }
        let dictionary = document
            .get_dictionary(object_id)
            .map_err(PdfError::from_lopdf)?;
        if let Ok(value) = dictionary.get(key) {
            return document
                .dereference(value)
                .map(|(_, value)| Some(value))
                .map_err(PdfError::from_lopdf);
        }
        match dictionary.get(b"Parent").and_then(Object::as_reference) {
            Ok(parent) => object_id = parent,
            Err(_) => return Ok(None),
        }
    }
}
