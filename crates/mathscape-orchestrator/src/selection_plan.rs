use mathscape_pdf::PdfSelection;

/// One contiguous physical-page slice suitable for an independent conversion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversionSlice {
    ordinal: usize,
    selection: PdfSelection,
}

impl ConversionSlice {
    pub const fn ordinal(&self) -> usize {
        self.ordinal
    }

    pub const fn selection(&self) -> &PdfSelection {
        &self.selection
    }
}

/// Deterministic decomposition of a user selection into contiguous runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversionSelectionPlan {
    requested: PdfSelection,
    slices: Vec<ConversionSlice>,
}

impl ConversionSelectionPlan {
    pub fn new(requested: PdfSelection) -> Self {
        let slices = requested
            .contiguous_slices()
            .into_iter()
            .enumerate()
            .map(|(ordinal, selection)| ConversionSlice { ordinal, selection })
            .collect();
        Self { requested, slices }
    }

    pub const fn requested(&self) -> &PdfSelection {
        &self.requested
    }

    pub fn slices(&self) -> &[ConversionSlice] {
        &self.slices
    }

    pub fn into_slices(self) -> Vec<ConversionSlice> {
        self.slices
    }
}

#[cfg(test)]
mod tests {
    use mathscape_pdf::PdfPageIndex;

    use super::*;

    #[test]
    fn plans_one_conversion_per_contiguous_range() {
        let requested = PdfSelection::from_pages(
            30,
            vec![1, 2, 3, 8, 20, 21]
                .into_iter()
                .map(PdfPageIndex::new)
                .collect(),
        )
        .unwrap();
        let plan = ConversionSelectionPlan::new(requested.clone());

        assert_eq!(plan.requested(), &requested);
        assert_eq!(plan.slices().len(), 3);
        assert_eq!(plan.slices()[0].ordinal(), 0);
        assert_eq!(
            plan.slices()[0].selection().pages(),
            &[
                PdfPageIndex::new(1),
                PdfPageIndex::new(2),
                PdfPageIndex::new(3)
            ]
        );
        assert_eq!(
            plan.slices()[1].selection().pages(),
            &[PdfPageIndex::new(8)]
        );
        assert_eq!(
            plan.slices()[2].selection().pages(),
            &[PdfPageIndex::new(20), PdfPageIndex::new(21)]
        );
    }

    #[test]
    fn keeps_a_contiguous_selection_as_one_slice() {
        let requested =
            PdfSelection::inclusive_range(12, PdfPageIndex::new(4), PdfPageIndex::new(7)).unwrap();
        let plan = ConversionSelectionPlan::new(requested.clone());

        assert_eq!(plan.slices().len(), 1);
        assert_eq!(plan.slices()[0].selection(), &requested);
    }
}
