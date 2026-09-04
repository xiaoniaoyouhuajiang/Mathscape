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
}

#[cfg(test)]
mod tests {
    use mathscape_pdf::{PdfPageIndex, PdfSelection};

    use super::*;

    #[test]
    fn preserves_mathscape_zero_based_pages() {
        let selection = PdfSelection::from_pages(
            30,
            vec![0, 5, 6, 7, 20]
                .into_iter()
                .map(PdfPageIndex::new)
                .collect(),
        )
        .unwrap();
        assert_eq!(
            MarkerPageRange::from_selection(&selection).as_str(),
            "0,5-7,20"
        );
    }
}
