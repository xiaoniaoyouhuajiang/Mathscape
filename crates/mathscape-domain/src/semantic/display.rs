//! Which of a shared object's names to show for the current display language.

use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayLang {
    #[default]
    Zh,
    En,
}

impl DisplayLang {
    /// `zh`, `zh-CN`, `en-US`, … by primary subtag; anything else is `None`.
    pub fn from_tag(tag: &str) -> Option<Self> {
        let primary = tag.split(['-', '_']).next().unwrap_or("");
        if primary.eq_ignore_ascii_case("zh") {
            Some(Self::Zh)
        } else if primary.eq_ignore_ascii_case("en") {
            Some(Self::En)
        } else {
            None
        }
    }

    /// Name shown for a shared object that has none.
    pub fn fallback_entity_label(self) -> &'static str {
        match self {
            Self::Zh => "共享对象",
            Self::En => "Shared object",
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Zh => Self::En,
            Self::En => Self::Zh,
        }
    }
}

/// Operator and function names that read as notation, not English words.
const NOTATION_WORDS: &[&str] = &[
    "arccos", "arcsin", "arctan", "arg", "aut", "cos", "cosh", "cot", "coth", "csc", "deg", "det",
    "dim", "exp", "gal", "gcd", "hom", "inf", "ker", "lcm", "lim", "liminf", "limsup", "log",
    "max", "min", "mod", "ord", "rank", "sec", "sgn", "sin", "sinh", "span", "spec", "sup", "tan",
    "tanh", "var",
];

/// `plain_label` carries no language tag: Han text reads as Chinese, a Latin
/// word (3+ letters) without Han as English, and notation such as `ℤ/nℤ`,
/// `gcd(a, b)` or `\lim a_n` fits either.
fn plain_label_fits(text: &str, lang: DisplayLang) -> bool {
    let han = text.chars().any(is_han);
    match lang {
        DisplayLang::Zh => han || !has_english_word(text),
        DisplayLang::En => !han,
    }
}

fn has_english_word(text: &str) -> bool {
    let mut rest = text;
    let mut words = Vec::new();
    while let Some(c) = rest.chars().next() {
        if c == '\\' {
            // A TeX command such as `\lim` or `\mathbb` is notation.
            rest = rest[1..].trim_start_matches(|c: char| c.is_ascii_alphabetic());
        } else if c.is_ascii_alphabetic() {
            let end = rest
                .find(|c: char| !c.is_ascii_alphabetic())
                .unwrap_or(rest.len());
            words.push(&rest[..end]);
            rest = &rest[end..];
        } else {
            rest = &rest[c.len_utf8()..];
        }
    }
    words.into_iter().any(|word| {
        word.len() >= 3 && !NOTATION_WORDS.contains(&word.to_ascii_lowercase().as_str())
    })
}

fn is_han(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{20000}'..='\u{2FA1F}')
}

fn non_empty(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The `preferred_labels` texts in display order for `lang`: that language
/// first, then the other one, then labels with any other tag, deduplicated.
pub fn preferred_labels_for(payload: &Value, lang: DisplayLang) -> Vec<String> {
    let items: Vec<(Option<DisplayLang>, &str)> = payload
        .get("preferred_labels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|l| {
            let label = non_empty(l.get("label"))?;
            let tag = l.get("lang").and_then(Value::as_str).unwrap_or("");
            Some((DisplayLang::from_tag(tag), label))
        })
        .collect();
    let mut out: Vec<String> = Vec::new();
    for want in [Some(lang), Some(lang.other()), None] {
        for (tag, label) in &items {
            if *tag == want && !out.iter().any(|s| s == label) {
                out.push((*label).to_owned());
            }
        }
    }
    out
}

/// The shared object's display name for `lang`: a `plain_label` written in
/// that language, else the `preferred_labels` entry for it, else whatever
/// name exists (`plain_label` before the other language's label).
pub fn semantic_display_label(payload: &Value, lang: DisplayLang) -> Option<String> {
    let plain = non_empty(payload.get("plain_label"));
    if let Some(p) = plain.filter(|p| plain_label_fits(p, lang)) {
        return Some(p.to_owned());
    }
    let labels = payload.get("preferred_labels").and_then(Value::as_array);
    let tagged = labels.into_iter().flatten().find(|l| {
        l.get("lang")
            .and_then(Value::as_str)
            .and_then(DisplayLang::from_tag)
            == Some(lang)
    });
    if let Some(label) = tagged.and_then(|l| non_empty(l.get("label"))) {
        return Some(label.to_owned());
    }
    plain
        .map(str::to_owned)
        .or_else(|| preferred_labels_for(payload, lang).into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entity(plain: Option<&str>) -> Value {
        let mut p = json!({ "preferred_labels": [
            { "lang": "en", "label": "Cassini identity" },
            { "lang": "zh", "label": "Cassini 恒等式" }
        ] });
        if let Some(plain) = plain {
            p["plain_label"] = json!(plain);
        }
        p
    }

    #[test]
    fn picks_the_label_for_the_display_language() {
        let p = entity(None);
        assert_eq!(
            semantic_display_label(&p, DisplayLang::Zh).as_deref(),
            Some("Cassini 恒等式")
        );
        assert_eq!(
            semantic_display_label(&p, DisplayLang::En).as_deref(),
            Some("Cassini identity")
        );
    }

    #[test]
    fn plain_label_is_used_only_in_its_own_language() {
        let p = entity(Some("精确说法：Cassini 恒等式"));
        assert_eq!(
            semantic_display_label(&p, DisplayLang::Zh).as_deref(),
            Some("精确说法：Cassini 恒等式")
        );
        assert_eq!(
            semantic_display_label(&p, DisplayLang::En).as_deref(),
            Some("Cassini identity")
        );
        let p = entity(Some("Cassini's identity, exact form"));
        assert_eq!(
            semantic_display_label(&p, DisplayLang::Zh).as_deref(),
            Some("Cassini 恒等式")
        );
        let p = entity(Some("F(n+1)F(n−1) − F(n)²"));
        assert_eq!(
            semantic_display_label(&p, DisplayLang::En).as_deref(),
            Some("F(n+1)F(n−1) − F(n)²")
        );
        assert_eq!(
            semantic_display_label(&p, DisplayLang::Zh).as_deref(),
            Some("F(n+1)F(n−1) − F(n)²")
        );
        for notation in [
            "gcd(a, b) = 1",
            "\\lim_{n} a_n",
            "det A",
            "sinh x",
            "rank A",
        ] {
            let p = entity(Some(notation));
            assert_eq!(
                semantic_display_label(&p, DisplayLang::Zh).as_deref(),
                Some(notation)
            );
        }
        let p = entity(Some("Norm of a Ring element"));
        assert_eq!(
            semantic_display_label(&p, DisplayLang::Zh).as_deref(),
            Some("Cassini 恒等式")
        );
        let p = entity(Some("ℤ/nℤ"));
        assert_eq!(
            semantic_display_label(&p, DisplayLang::Zh).as_deref(),
            Some("ℤ/nℤ")
        );
    }

    #[test]
    fn unnamed_objects_get_a_fallback_in_the_display_language() {
        assert_eq!(DisplayLang::Zh.fallback_entity_label(), "共享对象");
        assert_eq!(DisplayLang::En.fallback_entity_label(), "Shared object");
        assert_eq!(DisplayLang::default(), DisplayLang::Zh);
    }

    #[test]
    fn falls_back_to_any_name_and_reads_regional_tags() {
        let only_zh = json!({ "plain_label": "白话", "preferred_labels": [{ "lang": "zh-CN", "label": "中文名" }] });
        assert_eq!(
            semantic_display_label(&only_zh, DisplayLang::En).as_deref(),
            Some("白话")
        );
        let only_zh = json!({ "preferred_labels": [{ "lang": "zh-CN", "label": "中文名" }] });
        assert_eq!(
            semantic_display_label(&only_zh, DisplayLang::En).as_deref(),
            Some("中文名")
        );
        assert_eq!(semantic_display_label(&json!({}), DisplayLang::Zh), None);
        assert_eq!(
            preferred_labels_for(&entity(None), DisplayLang::Zh),
            vec!["Cassini 恒等式", "Cassini identity"]
        );
    }
}
