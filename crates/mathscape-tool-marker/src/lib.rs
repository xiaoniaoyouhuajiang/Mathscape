//! Marker capability discovery and controlled process adapter.

use mathscape_pdf::PdfSelection;

/// Validated value for Marker's zero-based `--page_range` option.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerPageRange(String);

impl MarkerPageRange {
    pub fn from_selection(selection: &PdfSelection) -> Self {
        let value = selection
            .ranges()
            .into_iter()
            .map(|range| {
                let first = range.first().get();
                let last = range.last().get();
                if first == last {
                    first.to_string()
                } else {
                    format!("{first}-{last}")
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the exact two argv entries to append to `marker_single`.
    pub fn command_args(&self) -> [&str; 2] {
        ["--page_range", self.as_str()]
    }
}

#[cfg(test)]
mod tests {
    use mathscape_pdf::{PdfPageIndex, PdfSelection};

    use super::*;

    #[test]
    fn marker_range_preserves_mathscape_zero_based_pages() {
        let selection = PdfSelection::from_pages(
            30,
            vec![0, 5, 6, 7, 20]
                .into_iter()
                .map(PdfPageIndex::new)
                .collect(),
        )
        .unwrap();
        let marker_range = MarkerPageRange::from_selection(&selection);
        assert_eq!(marker_range.as_str(), "0,5-7,20");
        assert_eq!(marker_range.command_args(), ["--page_range", "0,5-7,20"]);
    }
}
