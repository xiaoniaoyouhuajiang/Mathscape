use mathscape_pdf::PdfSelection;

use crate::MarkerRequestError;

/// Validated value for Marker's zero-based `--page_range` option.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerPageRange {
    first: u32,
    last: u32,
    value: String,
}

impl MarkerPageRange {
    pub fn try_from_selection(selection: &PdfSelection) -> Result<Self, MarkerRequestError> {
        let ranges = selection.ranges();
        if ranges.len() != 1 {
            return Err(MarkerRequestError::NonContiguousSelection {
                range_count: ranges.len(),
            });
        }
        let range = ranges[0];
        let first = range.first().get();
        let last = range.last().get();
        let value = if first == last {
            first.to_string()
        } else {
            format!("{first}-{last}")
        };
        Ok(Self { first, last, value })
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub const fn first(&self) -> u32 {
        self.first
    }

    pub const fn last(&self) -> u32 {
        self.last
    }
}

#[cfg(test)]
mod tests {
    use mathscape_pdf::{PdfPageIndex, PdfSelection};

    use super::*;

    #[test]
    fn preserves_a_contiguous_zero_based_range() {
        let selection =
            PdfSelection::inclusive_range(30, PdfPageIndex::new(5), PdfPageIndex::new(7)).unwrap();
        assert_eq!(
            MarkerPageRange::try_from_selection(&selection)
                .unwrap()
                .as_str(),
            "5-7"
        );
    }

    #[test]
    fn rejects_non_contiguous_ranges() {
        let selection = PdfSelection::from_pages(
            30,
            vec![0, 5, 6, 7, 20]
                .into_iter()
                .map(PdfPageIndex::new)
                .collect(),
        )
        .unwrap();
        assert!(matches!(
            MarkerPageRange::try_from_selection(&selection),
            Err(MarkerRequestError::NonContiguousSelection { range_count: 3 })
        ));
    }
}
