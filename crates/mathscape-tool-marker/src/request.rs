use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use mathscape_pdf::PdfSelection;

use crate::{
    MarkerInstallation, MarkerPageRange, MarkerProfile, MarkerRequestError,
    error::validate_absolute_path,
};

/// A validated semantic request, independent of temporary output placement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerRequest {
    source_pdf: PathBuf,
    selection: PdfSelection,
    page_range: MarkerPageRange,
    profile: MarkerProfile,
}

impl MarkerRequest {
    pub fn new(
        source_pdf: impl Into<PathBuf>,
        selection: PdfSelection,
        profile: MarkerProfile,
    ) -> Result<Self, MarkerRequestError> {
        let source_pdf = validate_absolute_path(source_pdf.into(), "marker.source_pdf")?;
        let page_range = MarkerPageRange::try_from_selection(&selection)?;
        Ok(Self {
            source_pdf,
            selection,
            page_range,
            profile,
        })
    }

    pub fn source_pdf(&self) -> &Path {
        &self.source_pdf
    }

    pub const fn selection(&self) -> &PdfSelection {
        &self.selection
    }

    pub const fn page_range(&self) -> &MarkerPageRange {
        &self.page_range
    }

    pub const fn profile(&self) -> MarkerProfile {
        self.profile
    }

    /// Resolves this request into an argv-safe command specification.
    ///
    /// This method performs no filesystem writes and never invokes a shell.
    pub fn prepare(
        &self,
        installation: &MarkerInstallation,
        output_root: impl Into<PathBuf>,
    ) -> Result<MarkerCommandSpec, MarkerRequestError> {
        let output_root = validate_absolute_path(output_root.into(), "marker.output_root")?;
        if self.source_pdf == output_root {
            return Err(MarkerRequestError::ConflictingPaths);
        }
        let arguments = vec![
            self.source_pdf.clone().into_os_string(),
            OsString::from("--page_range"),
            OsString::from(self.page_range.as_str()),
            OsString::from("--output_format"),
            OsString::from("markdown"),
            // Preserve deterministic PDF page boundaries in the Markdown
            // artifact. The ingestion layer consumes these markers to build
            // the Markdown→PDF backlink map.
            OsString::from("--paginate_output"),
            OsString::from("--mode"),
            OsString::from(self.profile.mode().cli_value()),
            OsString::from("--output_dir"),
            output_root.clone().into_os_string(),
        ];
        Ok(MarkerCommandSpec {
            program: installation.executable().to_owned(),
            working_directory: installation.working_directory().to_owned(),
            arguments,
            source_pdf: self.source_pdf.clone(),
            output_root,
            page_range: self.page_range.clone(),
            profile: self.profile,
        })
    }
}

/// Complete process input produced exclusively by a fixed Marker profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerCommandSpec {
    program: PathBuf,
    working_directory: PathBuf,
    arguments: Vec<OsString>,
    source_pdf: PathBuf,
    output_root: PathBuf,
    page_range: MarkerPageRange,
    profile: MarkerProfile,
}

impl MarkerCommandSpec {
    pub fn program(&self) -> &Path {
        &self.program
    }

    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub fn source_pdf(&self) -> &Path {
        &self.source_pdf
    }

    /// Root passed to `--output_dir`. Marker creates a source-named child below it.
    pub fn output_root(&self) -> &Path {
        &self.output_root
    }

    pub const fn page_range(&self) -> &MarkerPageRange {
        &self.page_range
    }

    pub const fn profile(&self) -> MarkerProfile {
        self.profile
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use mathscape_pdf::{PdfPageIndex, PdfSelection};

    use super::*;

    fn absolute_test_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-paths")
            .join(name)
    }

    fn selection() -> PdfSelection {
        PdfSelection::inclusive_range(24, PdfPageIndex::new(1), PdfPageIndex::new(3)).unwrap()
    }

    #[test]
    fn fixes_fast_profile_as_separate_argv_without_llm_rewrite() {
        let source = absolute_test_path("数学 教材.pdf");
        let output = absolute_test_path("mathscape marker 输出");
        let installation = MarkerInstallation::new(absolute_test_path("marker_single")).unwrap();
        let request =
            MarkerRequest::new(&source, selection(), MarkerProfile::FastNativeV1).unwrap();
        let command = request.prepare(&installation, &output).unwrap();

        assert_eq!(command.program(), installation.executable());
        assert_eq!(command.source_pdf(), source);
        assert_eq!(command.output_root(), output);
        assert_eq!(command.page_range().as_str(), "1-3");
        assert_eq!(
            command.arguments(),
            &[
                source.clone().into_os_string(),
                OsString::from("--page_range"),
                OsString::from("1-3"),
                OsString::from("--output_format"),
                OsString::from("markdown"),
                OsString::from("--paginate_output"),
                OsString::from("--mode"),
                OsString::from("fast"),
                OsString::from("--output_dir"),
                output.clone().into_os_string(),
            ]
        );
        assert!(!command.arguments().iter().any(|argument| {
            argument == OsStr::new("--redo_inline_math") || argument == OsStr::new("--use_llm")
        }));
    }

    #[test]
    fn fixes_math_ocr_profile_to_balanced_local_recognition() {
        let installation = MarkerInstallation::new(absolute_test_path("marker_single")).unwrap();
        let request = MarkerRequest::new(
            absolute_test_path("book.pdf"),
            selection(),
            MarkerProfile::MathOcrV1,
        )
        .unwrap();
        let command = request
            .prepare(&installation, absolute_test_path("output"))
            .unwrap();

        let mode_position = command
            .arguments()
            .iter()
            .position(|argument| argument == OsStr::new("--mode"))
            .unwrap();
        assert_eq!(command.arguments()[mode_position + 1], "balanced");
        assert_eq!(
            command.profile().inline_math_policy(),
            crate::InlineMathPolicy::LocalRecognitionAuto
        );
    }

    #[test]
    fn rejects_ambiguous_relative_or_conflicting_paths() {
        assert!(matches!(
            MarkerInstallation::new("marker_single"),
            Err(MarkerRequestError::RelativePath {
                field: "marker.executable",
                ..
            })
        ));
        assert!(matches!(
            MarkerRequest::new("book.pdf", selection(), MarkerProfile::FastNativeV1),
            Err(MarkerRequestError::RelativePath {
                field: "marker.source_pdf",
                ..
            })
        ));

        let source = absolute_test_path("same-path.pdf");
        let installation = MarkerInstallation::new(absolute_test_path("marker_single")).unwrap();
        let request =
            MarkerRequest::new(&source, selection(), MarkerProfile::FastNativeV1).unwrap();
        assert!(matches!(
            request.prepare(&installation, source),
            Err(MarkerRequestError::ConflictingPaths)
        ));
    }

    #[test]
    fn rejects_a_non_contiguous_selection_before_execution() {
        let selection = PdfSelection::from_pages(
            24,
            vec![1, 2, 3, 8]
                .into_iter()
                .map(PdfPageIndex::new)
                .collect(),
        )
        .unwrap();

        assert!(matches!(
            MarkerRequest::new(
                absolute_test_path("book.pdf"),
                selection,
                MarkerProfile::MathOcrV1,
            ),
            Err(MarkerRequestError::NonContiguousSelection { range_count: 2 })
        ));
    }
}
