use std::{collections::HashSet, path::Path};

use crate::{
    NormalizedRect, PdfError, PdfOpenOptions, PdfPageGeometry, PdfPageIndex, PdfPointRect,
    PdfSelection,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfTextItemKind {
    Text,
    Image,
    Link(String),
    FormField,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfTextItem {
    page: PdfPageIndex,
    text: String,
    bounds: PdfPointRect,
    font: String,
    font_tag: String,
    font_size: f32,
    bold: bool,
    italic: bool,
    underline: bool,
    strikeout: bool,
    kind: PdfTextItemKind,
    marked_content_id: Option<i64>,
}

impl PdfTextItem {
    pub const fn page(&self) -> PdfPageIndex {
        self.page
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn bounds(&self) -> PdfPointRect {
        self.bounds
    }

    pub fn font(&self) -> &str {
        &self.font
    }

    pub fn font_tag(&self) -> &str {
        &self.font_tag
    }

    pub const fn font_size(&self) -> f32 {
        self.font_size
    }

    pub const fn is_bold(&self) -> bool {
        self.bold
    }

    pub const fn is_italic(&self) -> bool {
        self.italic
    }

    pub const fn is_underline(&self) -> bool {
        self.underline
    }

    pub const fn is_strikeout(&self) -> bool {
        self.strikeout
    }

    pub const fn kind(&self) -> &PdfTextItemKind {
        &self.kind
    }

    pub const fn marked_content_id(&self) -> Option<i64> {
        self.marked_content_id
    }

    pub fn normalized_bounds(
        &self,
        geometry: PdfPageGeometry,
    ) -> Result<Option<NormalizedRect>, PdfError> {
        if self.page != geometry.page() {
            return Err(PdfError::GeometryPageMismatch {
                item_page: self.page.get(),
                geometry_page: geometry.page().get(),
            });
        }
        Ok(geometry.normalize(self.bounds))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextExtraction {
    selection: PdfSelection,
    items: Vec<PdfTextItem>,
}

impl TextExtraction {
    pub const fn selection(&self) -> &PdfSelection {
        &self.selection
    }

    pub fn items(&self) -> &[PdfTextItem] {
        &self.items
    }

    /// Plain text for every selected page, in selection order.
    ///
    /// Only [`PdfTextItemKind::Text`] items contribute. Items keep extraction
    /// order; a new line starts when the baseline moves by more than half a
    /// line height, and a single space is inserted between runs on the same
    /// line when there is a visible horizontal gap. Pages without any text
    /// item yield an empty string, so callers can detect a missing text layer.
    pub fn page_texts(&self) -> Vec<(PdfPageIndex, String)> {
        self.selection
            .pages()
            .iter()
            .map(|&page| {
                let mut text = String::new();
                let mut previous: Option<&PdfTextItem> = None;
                for item in self
                    .items
                    .iter()
                    .filter(|item| item.page == page && item.kind == PdfTextItemKind::Text)
                {
                    if let Some(previous) = previous {
                        text.push_str(run_separator(previous, item));
                    }
                    text.push_str(&item.text);
                    previous = Some(item);
                }
                (page, text)
            })
            .collect()
    }
}

fn run_separator(previous: &PdfTextItem, current: &PdfTextItem) -> &'static str {
    let line_height = previous
        .bounds
        .height()
        .max(current.bounds.height())
        .max(previous.font_size)
        .max(1.0);
    if (previous.bounds.bottom() - current.bounds.bottom()).abs() > line_height * 0.5 {
        return "\n";
    }
    let touching_whitespace = previous.text.ends_with(char::is_whitespace)
        || current.text.starts_with(char::is_whitespace);
    let gap = current.bounds.left() - previous.bounds.right();
    let em = previous.font_size.max(current.font_size).max(1.0);
    if !touching_whitespace && gap > em * 0.15 {
        " "
    } else {
        ""
    }
}

impl super::PdfEngine {
    pub fn extract_positioned_text(
        &self,
        path: impl AsRef<Path>,
        selection: &PdfSelection,
        open_options: &PdfOpenOptions,
    ) -> Result<TextExtraction, PdfError> {
        let one_based_pages: HashSet<u32> = selection.one_based_pages().collect();
        let items = pdf_inspector::extract_text_with_positions_pages_with_password(
            path,
            Some(&one_based_pages),
            open_options.password_ref(),
        )
        .map_err(PdfError::from_inspector)?;

        let items = items
            .into_iter()
            .map(|item| map_text_item(item, selection.page_count()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(TextExtraction {
            selection: selection.clone(),
            items,
        })
    }
}

fn map_text_item(item: pdf_inspector::TextItem, page_count: u32) -> Result<PdfTextItem, PdfError> {
    let page = PdfPageIndex::from_one_based(item.page, page_count)?;
    let bounds = PdfPointRect::from_origin_size(item.x, item.y, item.width, item.height)?;
    let kind = match item.item_type {
        pdf_inspector::extractor::ItemType::Text => PdfTextItemKind::Text,
        pdf_inspector::extractor::ItemType::Image => PdfTextItemKind::Image,
        pdf_inspector::extractor::ItemType::Link(url) => PdfTextItemKind::Link(url),
        pdf_inspector::extractor::ItemType::FormField => PdfTextItemKind::FormField,
    };
    Ok(PdfTextItem {
        page,
        text: item.text,
        bounds,
        font: item.font,
        font_tag: item.font_tag,
        font_size: item.font_size,
        bold: item.is_bold,
        italic: item.is_italic,
        underline: item.is_underline,
        strikeout: item.is_strikeout,
        kind,
        marked_content_id: item.mcid,
    })
}
