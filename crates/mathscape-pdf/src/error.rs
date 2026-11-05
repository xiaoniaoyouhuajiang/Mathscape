use std::io;

/// Stable failures exposed by the PDF boundary.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PdfError {
    #[error("PDF I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("the input is not a PDF: {0}")]
    NotPdf(String),
    #[error("the PDF is encrypted or its password is invalid")]
    Encrypted,
    #[error("the PDF has an invalid structure")]
    InvalidStructure,
    #[error("failed to parse PDF: {0}")]
    Parse(String),
    #[error("PDF contains no pages")]
    EmptyDocument,
    #[error("page index {index} is outside a document with {page_count} pages")]
    PageOutOfBounds { index: u32, page_count: u32 },
    #[error("PDF backends disagreed on page count: inspector={inspector}, document={document}")]
    PageCountMismatch { inspector: u32, document: u32 },
    #[error("a PDF page selection must not be empty")]
    EmptySelection,
    #[error("PDF page selections must be strictly increasing; found {previous} before {current}")]
    NonIncreasingSelection { previous: u32, current: u32 },
    #[error("invalid PDF rectangle: {0}")]
    InvalidRectangle(&'static str),
    #[error("unsupported PDF page rotation {0}; expected a multiple of 90 degrees")]
    UnsupportedRotation(i64),
    #[error("page {page} has no inherited {attribute} attribute")]
    MissingPageAttribute { page: u32, attribute: &'static str },
    #[error("failed to render PDF page: {0}")]
    Render(String),
    #[error("text item is on page {item_page}, but geometry belongs to page {geometry_page}")]
    GeometryPageMismatch { item_page: u32, geometry_page: u32 },
}

impl PdfError {
    pub(crate) fn from_inspector(error: pdf_inspector::PdfError) -> Self {
        match error {
            pdf_inspector::PdfError::Io(error) => Self::Io(error),
            pdf_inspector::PdfError::NotAPdf(message) => Self::NotPdf(message),
            pdf_inspector::PdfError::Encrypted => Self::Encrypted,
            pdf_inspector::PdfError::InvalidStructure => Self::InvalidStructure,
            pdf_inspector::PdfError::Parse(message) => Self::Parse(message),
        }
    }

    pub(crate) fn from_lopdf(error: lopdf::Error) -> Self {
        match error {
            lopdf::Error::IO(error) => Self::Io(error),
            lopdf::Error::Decryption(_)
            | lopdf::Error::InvalidPassword
            | lopdf::Error::AlreadyEncrypted
            | lopdf::Error::UnsupportedSecurityHandler(_) => Self::Encrypted,
            lopdf::Error::MissingXrefEntry
            | lopdf::Error::Xref(_)
            | lopdf::Error::IndirectObject { .. }
            | lopdf::Error::ObjectIdMismatch
            | lopdf::Error::InvalidObjectStream(_)
            | lopdf::Error::InvalidOffset(_) => Self::InvalidStructure,
            other => Self::Parse(other.to_string()),
        }
    }
}
