use serde::{Deserialize, Serialize};

use crate::{ActorRef, DomainError, StableId};

/// A normalized confidence value in the closed interval `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Confidence(f64);

impl Confidence {
    pub fn new(value: f64) -> Result<Self, DomainError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(DomainError::InvalidConfidence);
        }
        Ok(Self(value))
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Confidence {
    type Error = DomainError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Confidence> for f64 {
    fn from(value: Confidence) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceStatus {
    Unknown,
    Located,
    Exact,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseStatus {
    Raw,
    Parsed,
    Reviewed,
    Corrected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentationStatus {
    Proposed,
    Checked,
    Disputed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypingStatus {
    Unchecked,
    Parsed,
    Typechecked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofStatus {
    None,
    Candidate,
    Proved,
    KernelVerified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentStatus {
    Unmatched,
    Candidate,
    Reviewed,
    Accepted,
    Disputed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurationStatus {
    Raw,
    Accepted,
    Canonical,
    Deprecated,
    Refuted,
}

/// Independent trust axes; no single score can promote another axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrustRecord {
    provenance: ProvenanceStatus,
    parse: ParseStatus,
    segmentation: SegmentationStatus,
    typing: TypingStatus,
    proof: ProofStatus,
    alignment: AlignmentStatus,
    curation: CurationStatus,
    confidence: Confidence,
    evidence: Vec<StableId>,
    reviewers: Vec<ActorRef>,
}

impl TrustRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provenance: ProvenanceStatus,
        parse: ParseStatus,
        segmentation: SegmentationStatus,
        typing: TypingStatus,
        proof: ProofStatus,
        alignment: AlignmentStatus,
        curation: CurationStatus,
        confidence: Confidence,
        evidence: Vec<StableId>,
        reviewers: Vec<ActorRef>,
    ) -> Self {
        Self {
            provenance,
            parse,
            segmentation,
            typing,
            proof,
            alignment,
            curation,
            confidence,
            evidence,
            reviewers,
        }
    }

    pub fn provenance(&self) -> ProvenanceStatus {
        self.provenance
    }

    pub fn parse(&self) -> ParseStatus {
        self.parse
    }

    pub fn segmentation(&self) -> SegmentationStatus {
        self.segmentation
    }

    pub fn typing(&self) -> TypingStatus {
        self.typing
    }

    pub fn proof(&self) -> ProofStatus {
        self.proof
    }

    pub fn alignment(&self) -> AlignmentStatus {
        self.alignment
    }

    pub fn curation(&self) -> CurationStatus {
        self.curation
    }

    pub fn confidence(&self) -> Confidence {
        self.confidence
    }

    pub fn evidence(&self) -> &[StableId] {
        &self.evidence
    }

    pub fn reviewers(&self) -> &[ActorRef] {
        &self.reviewers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_is_bounded_and_finite() {
        assert_eq!(Confidence::new(0.95).unwrap().get(), 0.95);
        assert!(Confidence::new(-0.1).is_err());
        assert!(Confidence::new(1.1).is_err());
        assert!(Confidence::new(f64::NAN).is_err());
    }
}
