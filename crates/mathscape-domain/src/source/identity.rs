use serde::{Deserialize, Serialize};

use crate::{Confidence, DomainError, EntityKind, EntityPayload, StableId};

use super::{ensure_non_blank, ensure_optional_non_blank};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkKind {
    Textbook,
    ProblemBook,
    Paper,
    Notes,
    Other,
}

/// Minimal bibliographic contributor. Identity reconciliation is deferred.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Contributor {
    display_name: String,
    identifier: Option<String>,
}

impl Contributor {
    pub fn new(display_name: String, identifier: Option<String>) -> Result<Self, DomainError> {
        ensure_non_blank(&display_name, "contributor.display_name")?;
        ensure_optional_non_blank(identifier.as_deref(), "contributor.identifier")?;
        Ok(Self {
            display_name,
            identifier,
        })
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn identifier(&self) -> Option<&str> {
        self.identifier.as_deref()
    }
}

/// Abstract bibliographic work, independent of a concrete edition or file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SourceWork {
    title: String,
    authors: Vec<Contributor>,
    kind: WorkKind,
}

impl SourceWork {
    pub fn new(
        title: String,
        authors: Vec<Contributor>,
        kind: WorkKind,
    ) -> Result<Self, DomainError> {
        ensure_non_blank(&title, "source_work.title")?;
        Ok(Self {
            title,
            authors,
            kind,
        })
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn authors(&self) -> &[Contributor] {
        &self.authors
    }

    pub fn kind(&self) -> WorkKind {
        self.kind
    }
}

impl EntityPayload for SourceWork {
    fn entity_kind(&self) -> EntityKind {
        EntityKind::SourceWork
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExternalIdentifier {
    scheme: String,
    value: String,
}

impl ExternalIdentifier {
    pub fn new(scheme: String, value: String) -> Result<Self, DomainError> {
        ensure_non_blank(&scheme, "external_identifier.scheme")?;
        ensure_non_blank(&value, "external_identifier.value")?;
        Ok(Self { scheme, value })
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

/// A concrete bibliographic edition that artifacts can realize.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceEdition {
    work: StableId,
    edition_statement: Option<String>,
    publisher: Option<String>,
    publication_year: Option<u16>,
    language: String,
    identifiers: Vec<ExternalIdentifier>,
    bibliographic_confidence: Confidence,
}

impl SourceEdition {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        work: StableId,
        edition_statement: Option<String>,
        publisher: Option<String>,
        publication_year: Option<u16>,
        language: String,
        identifiers: Vec<ExternalIdentifier>,
        bibliographic_confidence: Confidence,
    ) -> Result<Self, DomainError> {
        ensure_optional_non_blank(
            edition_statement.as_deref(),
            "source_edition.edition_statement",
        )?;
        ensure_optional_non_blank(publisher.as_deref(), "source_edition.publisher")?;
        ensure_non_blank(&language, "source_edition.language")?;
        Ok(Self {
            work,
            edition_statement,
            publisher,
            publication_year,
            language,
            identifiers,
            bibliographic_confidence,
        })
    }

    pub fn work(&self) -> &StableId {
        &self.work
    }

    pub fn edition_statement(&self) -> Option<&str> {
        self.edition_statement.as_deref()
    }

    pub fn publisher(&self) -> Option<&str> {
        self.publisher.as_deref()
    }

    pub fn publication_year(&self) -> Option<u16> {
        self.publication_year
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn identifiers(&self) -> &[ExternalIdentifier] {
        &self.identifiers
    }

    pub fn bibliographic_confidence(&self) -> Confidence {
        self.bibliographic_confidence
    }
}

impl EntityPayload for SourceEdition {
    fn entity_kind(&self) -> EntityKind {
        EntityKind::SourceEdition
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_style_work_and_edition_are_valid() {
        let work = SourceWork::new(
            "数学分析中的典型问题与方法".to_owned(),
            vec![],
            WorkKind::ProblemBook,
        )
        .unwrap();
        assert_eq!(work.entity_kind(), EntityKind::SourceWork);

        let edition = SourceEdition::new(
            StableId::parse("emir-fixture:work/analysis-typical-problems-methods").unwrap(),
            None,
            None,
            None,
            "zh-CN".to_owned(),
            vec![],
            Confidence::new(0.45).unwrap(),
        )
        .unwrap();
        assert_eq!(edition.language(), "zh-CN");
    }
}
