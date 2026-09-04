/// Marker execution mode fixed by a versioned Mathscape profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkerMode {
    Fast,
    Balanced,
}

impl MarkerMode {
    pub const fn cli_value(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
        }
    }
}

/// Meaning of inline-math handling under the selected profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InlineMathPolicy {
    /// Keep good embedded text; display equations may still use local block OCR.
    PreserveExtractedText,
    /// Let Marker's balanced local recognition detect and repair inline math.
    LocalRecognitionAuto,
}

/// Immutable conversion policies understood by the first Marker adapter.
///
/// A new behavior requires a new enum variant and profile ID. This prevents a
/// Marker upgrade or host-device default from silently changing provenance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkerProfile {
    FastNativeV1,
    MathOcrV1,
}

impl MarkerProfile {
    pub const SUPPORTED_MARKER_MAJOR: u64 = 2;

    pub const fn id(self) -> &'static str {
        match self {
            Self::FastNativeV1 => "mathscape.marker.fast-native.v1",
            Self::MathOcrV1 => "mathscape.marker.math-ocr.v1",
        }
    }

    pub const fn mode(self) -> MarkerMode {
        match self {
            Self::FastNativeV1 => MarkerMode::Fast,
            Self::MathOcrV1 => MarkerMode::Balanced,
        }
    }

    pub const fn inline_math_policy(self) -> InlineMathPolicy {
        match self {
            Self::FastNativeV1 => InlineMathPolicy::PreserveExtractedText,
            Self::MathOcrV1 => InlineMathPolicy::LocalRecognitionAuto,
        }
    }
}
