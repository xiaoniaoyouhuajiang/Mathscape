use crate::PdfError;

/// Zero-based physical index in the original PDF artifact.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PdfPageIndex(u32);

impl PdfPageIndex {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub(crate) fn from_one_based(page: u32, page_count: u32) -> Result<Self, PdfError> {
        let index = page.checked_sub(1).ok_or(PdfError::PageOutOfBounds {
            index: page,
            page_count,
        })?;
        Self::checked(index, page_count)
    }

    pub(crate) fn to_one_based(self) -> u32 {
        self.0 + 1
    }

    pub(crate) fn checked(index: u32, page_count: u32) -> Result<Self, PdfError> {
        if index >= page_count {
            return Err(PdfError::PageOutOfBounds { index, page_count });
        }
        Ok(Self(index))
    }
}

/// Inclusive range of zero-based physical PDF pages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfPageRange {
    first: PdfPageIndex,
    last: PdfPageIndex,
}

impl PdfPageRange {
    pub fn new(first: PdfPageIndex, last: PdfPageIndex) -> Result<Self, PdfError> {
        if first > last {
            return Err(PdfError::NonIncreasingSelection {
                previous: first.get(),
                current: last.get(),
            });
        }
        Ok(Self { first, last })
    }

    pub const fn first(self) -> PdfPageIndex {
        self.first
    }

    pub const fn last(self) -> PdfPageIndex {
        self.last
    }
}

/// Canonical, document-ordered virtual slice of an immutable PDF.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfSelection {
    page_count: u32,
    pages: Vec<PdfPageIndex>,
}

impl PdfSelection {
    pub fn all(page_count: u32) -> Result<Self, PdfError> {
        if page_count == 0 {
            return Err(PdfError::EmptyDocument);
        }
        Ok(Self {
            page_count,
            pages: (0..page_count).map(PdfPageIndex::new).collect(),
        })
    }

    pub fn inclusive_range(
        page_count: u32,
        first: PdfPageIndex,
        last: PdfPageIndex,
    ) -> Result<Self, PdfError> {
        PdfPageIndex::checked(first.get(), page_count)?;
        PdfPageIndex::checked(last.get(), page_count)?;
        let range = PdfPageRange::new(first, last)?;
        Ok(Self {
            page_count,
            pages: (range.first().get()..=range.last().get())
                .map(PdfPageIndex::new)
                .collect(),
        })
    }

    pub fn from_pages(page_count: u32, pages: Vec<PdfPageIndex>) -> Result<Self, PdfError> {
        if page_count == 0 {
            return Err(PdfError::EmptyDocument);
        }
        if pages.is_empty() {
            return Err(PdfError::EmptySelection);
        }
        for page in &pages {
            PdfPageIndex::checked(page.get(), page_count)?;
        }
        for pair in pages.windows(2) {
            if pair[0] >= pair[1] {
                return Err(PdfError::NonIncreasingSelection {
                    previous: pair[0].get(),
                    current: pair[1].get(),
                });
            }
        }
        Ok(Self { page_count, pages })
    }

    pub const fn page_count(&self) -> u32 {
        self.page_count
    }

    pub fn pages(&self) -> &[PdfPageIndex] {
        &self.pages
    }

    pub fn ranges(&self) -> Vec<PdfPageRange> {
        let mut ranges = Vec::new();
        let mut first = self.pages[0];
        let mut last = first;
        for &page in &self.pages[1..] {
            if page.get() == last.get() + 1 {
                last = page;
            } else {
                ranges.push(PdfPageRange { first, last });
                first = page;
                last = page;
            }
        }
        ranges.push(PdfPageRange { first, last });
        ranges
    }

    /// Splits an arbitrary ordered selection into independently contiguous
    /// selections without changing physical page indices.
    pub fn contiguous_slices(&self) -> Vec<Self> {
        self.ranges()
            .into_iter()
            .map(|range| Self {
                page_count: self.page_count,
                pages: (range.first().get()..=range.last().get())
                    .map(PdfPageIndex::new)
                    .collect(),
            })
            .collect()
    }

    pub(crate) fn one_based_pages(&self) -> impl Iterator<Item = u32> + '_ {
        self.pages.iter().map(|page| page.to_one_based())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_compresses_discrete_pages_into_ranges() {
        let selection = PdfSelection::from_pages(
            12,
            vec![0, 1, 2, 5, 8, 9]
                .into_iter()
                .map(PdfPageIndex::new)
                .collect(),
        )
        .unwrap();

        let ranges = selection.ranges();
        assert_eq!(ranges.len(), 3);
        assert_eq!((ranges[0].first().get(), ranges[0].last().get()), (0, 2));
        assert_eq!((ranges[1].first().get(), ranges[1].last().get()), (5, 5));
        assert_eq!((ranges[2].first().get(), ranges[2].last().get()), (8, 9));
    }

    #[test]
    fn selection_rejects_engine_unsafe_input() {
        assert!(matches!(
            PdfSelection::from_pages(4, vec![PdfPageIndex::new(2), PdfPageIndex::new(2)]),
            Err(PdfError::NonIncreasingSelection { .. })
        ));
        assert!(matches!(
            PdfSelection::inclusive_range(4, PdfPageIndex::new(1), PdfPageIndex::new(4)),
            Err(PdfError::PageOutOfBounds { .. })
        ));
    }

    #[test]
    fn selection_splits_into_contiguous_slices_without_renumbering_pages() {
        let selection = PdfSelection::from_pages(
            20,
            vec![1, 2, 3, 8, 12, 13]
                .into_iter()
                .map(PdfPageIndex::new)
                .collect(),
        )
        .unwrap();

        let slices = selection.contiguous_slices();
        assert_eq!(slices.len(), 3);
        assert_eq!(
            slices[0].pages(),
            &[
                PdfPageIndex::new(1),
                PdfPageIndex::new(2),
                PdfPageIndex::new(3)
            ]
        );
        assert_eq!(slices[1].pages(), &[PdfPageIndex::new(8)]);
        assert_eq!(
            slices[2].pages(),
            &[PdfPageIndex::new(12), PdfPageIndex::new(13)]
        );
        assert!(slices.iter().all(|slice| slice.page_count() == 20));
    }
}
