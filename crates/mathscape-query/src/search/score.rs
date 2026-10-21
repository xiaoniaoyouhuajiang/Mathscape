//! Match scoring (pure). Ranks exact > prefix > substring > in-order chars.

use super::normalize::NormalizedText;

pub const SCORE_EXACT: u32 = 100;
pub const SCORE_PREFIX: u32 = 80;
pub const SCORE_BOUNDARY: u32 = 65;
pub const SCORE_SUBSTRING: u32 = 50;
pub const SCORE_SUBSEQUENCE: u32 = 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum MatchKind {
    Subsequence,
    Substring,
    Boundary,
    Prefix,
    Exact,
}

/// A term match: score plus matched positions as source char indexes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TermMatch {
    pub kind: MatchKind,
    pub score: u32,
    pub positions: Vec<usize>,
}

fn coverage_bonus(term: usize, hay: usize) -> u32 {
    // Up to 9 points: shorter haystacks (closer to the term) rank higher.
    u32::try_from(term * 9 / hay.max(1)).unwrap_or(9).min(9)
}

fn find(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&start| hay[start..start + needle.len()] == *needle)
}

fn positions(hay: &NormalizedText, start: usize, len: usize) -> Vec<usize> {
    hay.origin[start..start + len].to_vec()
}

/// A substring starting right after an ignored char (space, punctuation) or
/// at a script change is a "word boundary" match.
fn boundary_starts(hay: &NormalizedText, needle: &[char]) -> Option<usize> {
    if needle.len() > hay.chars.len() {
        return None;
    }
    (1..=hay.chars.len() - needle.len()).find(|&start| {
        hay.chars[start..start + needle.len()] == *needle
            && (hay.origin[start] > hay.origin[start - 1] + 1
                || hay.chars[start - 1].is_ascii_alphanumeric()
                    != hay.chars[start].is_ascii_alphanumeric())
    })
}

/// Greedy in-order match, then score by compactness of the matched span.
fn subsequence(hay: &NormalizedText, needle: &[char]) -> Option<TermMatch> {
    if needle.len() < 2 {
        return None;
    }
    let mut found = Vec::with_capacity(needle.len());
    let mut next = 0;
    for &c in needle {
        let offset = hay.chars[next..].iter().position(|&h| h == c)?;
        found.push(next + offset);
        next += offset + 1;
    }
    let span = found.last()? - found.first()? + 1;
    let compactness = u32::try_from(needle.len() * 20 / span)
        .unwrap_or(20)
        .min(20);
    Some(TermMatch {
        kind: MatchKind::Subsequence,
        score: SCORE_SUBSEQUENCE + compactness,
        positions: found.into_iter().map(|index| hay.origin[index]).collect(),
    })
}

/// Scores one normalized term against one normalized field.
pub fn score_term(term: &NormalizedText, hay: &NormalizedText) -> Option<TermMatch> {
    let needle = term.chars.as_slice();
    if needle.is_empty() || hay.is_empty() {
        return None;
    }
    if hay.chars == needle {
        return Some(TermMatch {
            kind: MatchKind::Exact,
            score: SCORE_EXACT,
            positions: positions(hay, 0, needle.len()),
        });
    }
    let bonus = coverage_bonus(needle.len(), hay.len());
    if hay.chars.starts_with(needle) {
        return Some(TermMatch {
            kind: MatchKind::Prefix,
            score: SCORE_PREFIX + bonus,
            positions: positions(hay, 0, needle.len()),
        });
    }
    if let Some(start) = boundary_starts(hay, needle) {
        return Some(TermMatch {
            kind: MatchKind::Boundary,
            score: SCORE_BOUNDARY + bonus,
            positions: positions(hay, start, needle.len()),
        });
    }
    if let Some(start) = find(&hay.chars, needle) {
        return Some(TermMatch {
            kind: MatchKind::Substring,
            score: SCORE_SUBSTRING + bonus,
            positions: positions(hay, start, needle.len()),
        });
    }
    subsequence(hay, needle)
}

/// Best match of a whole query against a set of fields. Field 0 is the
/// title; others (aliases) are worth 90%. The whole query (terms joined) is
/// tried first; otherwise every term must match some field (AND) and the
/// weakest term decides the score.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldMatch {
    pub score: u32,
    /// Matched source-char positions inside the title (field 0) only.
    pub title_positions: Vec<usize>,
    /// Index of the best-matching alias when it beat the title.
    pub alias: Option<usize>,
}

fn weighted(field: usize, score: u32) -> u32 {
    if field == 0 { score } else { score * 9 / 10 }
}

fn best_field(term: &NormalizedText, fields: &[NormalizedText]) -> Option<(usize, TermMatch, u32)> {
    fields
        .iter()
        .enumerate()
        .filter_map(|(index, field)| {
            score_term(term, field).map(|found| {
                let score = weighted(index, found.score);
                (index, found, score)
            })
        })
        .max_by(|a, b| a.2.cmp(&b.2).then(b.0.cmp(&a.0)))
}

pub fn score_fields(terms: &[NormalizedText], fields: &[NormalizedText]) -> Option<FieldMatch> {
    if terms.is_empty() || fields.is_empty() {
        return None;
    }
    let whole = NormalizedText {
        chars: terms
            .iter()
            .flat_map(|term| term.chars.iter().copied())
            .collect(),
        origin: vec![],
    };
    let whole_match = best_field(&whole, fields);
    if terms.len() == 1
        || whole_match
            .as_ref()
            .is_some_and(|m| m.1.kind >= MatchKind::Substring)
    {
        let (field, found, score) = whole_match?;
        return Some(FieldMatch {
            score,
            title_positions: if field == 0 { found.positions } else { vec![] },
            alias: (field != 0).then(|| field - 1),
        });
    }
    let mut weakest = u32::MAX;
    let mut title_positions = Vec::new();
    let mut alias = None;
    for term in terms {
        let (field, found, score) = best_field(term, fields)?;
        weakest = weakest.min(score);
        if field == 0 {
            title_positions.extend(found.positions);
        } else {
            alias.get_or_insert(field - 1);
        }
    }
    title_positions.sort_unstable();
    title_positions.dedup();
    // AND matches rank below a contiguous whole-query match of equal kind.
    Some(FieldMatch {
        score: weakest.saturating_sub(5),
        title_positions,
        alias,
    })
}

#[cfg(test)]
mod tests {
    use super::super::normalize::{normalize, query_terms};
    use super::*;

    fn s(term: &str, hay: &str) -> Option<(MatchKind, u32)> {
        score_term(&normalize(term), &normalize(hay)).map(|m| (m.kind, m.score))
    }

    #[test]
    fn ranks_exact_prefix_boundary_substring_subsequence() {
        let exact = s("整理稿", "整理稿").unwrap();
        let prefix = s("整理", "整理稿").unwrap();
        let boundary = s("lemma", "Zorn lemma").unwrap();
        let substring = s("理稿", "整理稿").unwrap();
        let subsequence = s("整稿", "整理稿").unwrap();
        assert_eq!(exact.0, MatchKind::Exact);
        assert_eq!(prefix.0, MatchKind::Prefix);
        assert_eq!(boundary.0, MatchKind::Boundary);
        assert_eq!(substring.0, MatchKind::Substring);
        assert_eq!(subsequence.0, MatchKind::Subsequence);
        assert!(exact.1 > prefix.1 && prefix.1 > boundary.1);
        assert!(boundary.1 > substring.1 && substring.1 > subsequence.1);
        assert_eq!(s("xyz", "整理稿"), None);
        assert_eq!(s("稿整", "整理稿"), None, "order matters");
    }

    #[test]
    fn normalization_applies_to_both_sides() {
        assert_eq!(s("ＣＡＵＣＨＹ", "cauchy").unwrap().0, MatchKind::Exact);
        assert_eq!(
            s("\\mathbb R", "$\\mathbb{R}$").unwrap().0,
            MatchKind::Exact
        );
        assert_eq!(
            s("定理3.2", "定理 3.2（中值）").unwrap().0,
            MatchKind::Prefix
        );
    }

    #[test]
    fn shorter_titles_rank_higher_for_same_kind() {
        assert!(s("极限", "极限定义").unwrap().1 > s("极限", "极限的定义与例子").unwrap().1);
    }

    #[test]
    fn positions_point_at_source_chars() {
        let found = score_term(&normalize("ab"), &normalize("x A-b")).unwrap();
        assert_eq!(found.positions, vec![2, 4]);
    }

    #[test]
    fn multi_term_queries_require_every_term() {
        let fields = [normalize("Cauchy 中值定理"), normalize("微分中值")];
        let both = score_fields(&query_terms("定理 cauchy"), &fields).unwrap();
        assert!(both.score > 0);
        assert_eq!(score_fields(&query_terms("定理 lagrange"), &fields), None);
        let split_across = score_fields(&query_terms("微分 cauchy"), &fields).unwrap();
        assert_eq!(split_across.alias, Some(0));
    }

    #[test]
    fn alias_matches_weigh_less_than_title() {
        let title =
            score_fields(&query_terms("极限"), &[normalize("极限"), normalize("x")]).unwrap();
        let alias =
            score_fields(&query_terms("极限"), &[normalize("x"), normalize("极限")]).unwrap();
        assert!(title.score > alias.score);
        assert_eq!(alias.alias, Some(0));
        assert!(alias.title_positions.is_empty());
    }
}
