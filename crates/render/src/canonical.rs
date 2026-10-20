//! Canonical composition (Unicode NFC) of searched text that remembers where each character came
//! from, so a match found in the composed text maps back to the glyphs that drew it.
//!
//! Text layers disagree on normalization: one producer writes "ử" as U+1EED, another as "u"
//! U+031B U+0309, often one glyph per mark. Search composes both the query and the page, so the
//! two spellings find each other, while every diacritic still counts ("a" never matches "ả").

use unicode_normalization::char::{canonical_combining_class, compose, decompose_canonical};

/// A character of composed text and the first and last glyph (inclusive) it was made from.
pub(crate) type Spanned = (char, usize, usize);

/// NFC of `chars`, each paired with its glyph; every output character spans all the glyphs its
/// source characters came from.
pub(crate) fn compose_spanned(chars: &[(char, usize)]) -> Vec<Spanned> {
    let mut decomposed: Vec<Spanned> = Vec::with_capacity(chars.len());
    for &(c, glyph) in chars {
        decompose_canonical(c, |d| decomposed.push((d, glyph, glyph)));
    }
    reorder_marks(&mut decomposed);
    let mut out: Vec<Spanned> = Vec::with_capacity(decomposed.len());
    let mut starter: Option<usize> = None;
    // Combining class of the last character pushed after the starter; `None` when the starter is
    // the last one, so nothing stands between it and the next character.
    let mut last_class: Option<u8> = None;
    for (c, first, last) in decomposed {
        let class = canonical_combining_class(c);
        let blocked = last_class.is_some_and(|l| l == 0 || l >= class);
        if !blocked
            && let Some(s) = starter.and_then(|s| out.get_mut(s))
            && let Some(composed) = compose(s.0, c)
        {
            *s = (composed, s.1.min(first), s.2.max(last));
            continue;
        }
        if class == 0 {
            starter = Some(out.len());
            last_class = None;
        } else {
            last_class = Some(class);
        }
        out.push((c, first, last));
    }
    out
}

/// Canonical ordering: each run of combining marks sorted by combining class, stably.
fn reorder_marks(chars: &mut [Spanned]) {
    let mut i = 0;
    while i < chars.len() {
        let run = chars.get(i..).map_or(0, |rest| rest.iter().take_while(|c| canonical_combining_class(c.0) != 0).count());
        if run > 1
            && let Some(marks) = chars.get_mut(i..i + run)
        {
            marks.sort_by_key(|c| canonical_combining_class(c.0));
        }
        i += run.max(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spanned: &[Spanned]) -> String {
        spanned.iter().map(|c| c.0).collect()
    }

    fn one_glyph_per_char(s: &str) -> Vec<(char, usize)> {
        s.chars().enumerate().map(|(i, c)| (c, i)).collect()
    }

    #[test]
    fn decomposed_vietnamese_composes_and_spans_its_glyphs() {
        // "Thử" as T, h, u, horn, hook above: the composed "ử" covers glyphs 2 to 4.
        let got = compose_spanned(&one_glyph_per_char("Thu\u{31B}\u{309}"));
        assert_eq!(got, vec![('T', 0, 0), ('h', 1, 1), ('\u{1EED}', 2, 4)]);
    }

    #[test]
    fn precomposed_text_is_unchanged() {
        let s = "Nguyễn Văn Ánh";
        let got = compose_spanned(&one_glyph_per_char(s));
        assert_eq!(text(&got), s);
        assert!(got.iter().enumerate().all(|(i, c)| c.1 == i && c.2 == i));
    }

    #[test]
    fn marks_in_either_order_compose_alike() {
        // Hook above (230) before horn (216) is canonically the same "ử".
        let got = compose_spanned(&one_glyph_per_char("u\u{309}\u{31B}"));
        assert_eq!(got, vec![('\u{1EED}', 0, 2)]);
    }

    #[test]
    fn a_mark_with_no_precomposed_form_stays_apart() {
        // "x" with an acute has no precomposed letter.
        assert_eq!(text(&compose_spanned(&one_glyph_per_char("x\u{301}"))), "x\u{301}");
        assert_eq!(text(&compose_spanned(&one_glyph_per_char("\u{301}a"))), "\u{301}a", "a leading mark has nothing to join");
    }

    #[test]
    fn hostile_mark_runs_are_bounded() {
        let mut s = String::from("a");
        s.extend(std::iter::repeat_n("\u{301}\u{323}", 50_000));
        let got = compose_spanned(&one_glyph_per_char(&s));
        assert_eq!(got.first().map(|c| c.0), Some('\u{1EA1}'), "the dot below composes first");
        assert_eq!(got.len(), 100_000);
    }
}
