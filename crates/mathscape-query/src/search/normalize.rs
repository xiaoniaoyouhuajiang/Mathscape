//! Text normalization for search (pure; reused by future agent tools).
//!
//! Folds full-width forms to half-width, lowercases, and drops whitespace,
//! punctuation and LaTeX delimiters (`$`, `\(`, `\)`, `\[`, `\]`, the
//! backslash and braces), so `$\mathbb{R}$`, `\mathbb R` and `ＭＡＴＨＢＢ Ｒ`
//! all normalize to `mathbbr`. Mathematical operators that carry meaning
//! (`.`, `_`, `^`, `+`, `=`, `<`, `>`, `*`, `/`, `'`, `|`) are kept. No pinyin.

/// Normalized text plus, for every kept char, its char index in the source.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NormalizedText {
    pub chars: Vec<char>,
    pub origin: Vec<usize>,
}

impl NormalizedText {
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }
    pub fn len(&self) -> usize {
        self.chars.len()
    }
    pub fn as_string(&self) -> String {
        self.chars.iter().collect()
    }
}

/// Full-width → half-width (U+FF01–U+FF5E, ideographic space).
pub fn fold_width(c: char) -> char {
    match c {
        '\u{3000}' => ' ',
        '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        _ => c,
    }
}

/// Whether a (width-folded) char is ignored when matching.
pub fn is_ignorable(c: char) -> bool {
    if c.is_whitespace() || c.is_control() {
        return true;
    }
    if c.is_ascii() {
        return c.is_ascii_punctuation() && !".^_+=<>*/'|".contains(c);
    }
    matches!(c,
        '\u{00B7}'                      // middle dot ·
        | '\u{2010}'..='\u{2027}'       // dashes, quotes, bullet, ellipsis
        | '\u{2030}'..='\u{205E}'       // more general punctuation
        | '\u{3001}'..='\u{3006}'       // 、。〃〄々〆 (keep 〇 U+3007)
        | '\u{3008}'..='\u{301F}'       // 〈〉《》「」『』【】〔〕…
        | '\u{30FB}'                    // katakana middle dot ・
        | '\u{FE10}'..='\u{FE6F}'       // vertical / small form punctuation
        | '\u{FF61}'..='\u{FF65}'       // half-width CJK punctuation
    )
}

pub fn normalize(input: &str) -> NormalizedText {
    let mut result = NormalizedText::default();
    for (index, raw) in input.chars().enumerate() {
        let c = fold_width(raw);
        if is_ignorable(c) {
            continue;
        }
        for lower in c.to_lowercase() {
            result.chars.push(lower);
            result.origin.push(index);
        }
    }
    result
}

/// Splits a query into normalized terms on whitespace (including full-width
/// spaces); terms that normalize to nothing are dropped.
pub fn query_terms(query: &str) -> Vec<NormalizedText> {
    query
        .split(|c: char| fold_width(c).is_whitespace())
        .map(normalize)
        .filter(|term| !term.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(value: &str) -> String {
        normalize(value).as_string()
    }

    #[test]
    fn folds_width_case_and_punctuation() {
        assert_eq!(n("ＡＢＣ　ｄｅｆ"), "abcdef");
        assert_eq!(n("Cauchy–Schwarz 不等式"), "cauchyschwarz不等式");
        assert_eq!(n("定理（3．2）"), "定理3.2");
        assert_eq!(n("《数学分析》·第一章"), "数学分析第一章");
        assert_eq!(n("“引理”、命题。"), "引理命题");
    }

    #[test]
    fn strips_latex_delimiters() {
        assert_eq!(n("$\\mathbb{R}$ 上的函数"), "mathbbr上的函数");
        assert_eq!(n("\\(a_{n}\\) 收敛"), "a_n收敛");
        assert_eq!(n("\\[x^2+y^2=1\\]"), "x^2+y^2=1");
        assert_eq!(n("f'(x)"), "f'x");
    }

    #[test]
    fn origin_points_back_to_source_chars() {
        let text = normalize("Ａ b-C");
        assert_eq!(text.as_string(), "abc");
        assert_eq!(text.origin, vec![0, 2, 4]);
    }

    #[test]
    fn query_terms_split_on_all_spaces() {
        let terms: Vec<_> = query_terms(" 定理\u{3000}3.2  $$ ")
            .iter()
            .map(NormalizedText::as_string)
            .collect();
        assert_eq!(terms, vec!["定理", "3.2"]);
        assert!(query_terms("  ，。 ").is_empty());
    }
}
