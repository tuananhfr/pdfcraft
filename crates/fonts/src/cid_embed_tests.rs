use super::cid_embed::*;
use crate::{INTER_REGULAR, font_resources::register, pdf::Metrics};
use pdfcraft_cos::{Document, Object};
use skrifa::{
    FontRef, MetadataProvider,
    instance::{LocationRef, Size},
};

#[test]
fn vietnamese_subset_has_identity_unicode_widths_and_only_used_outlines() {
    let text = "Nguyễn Thị Hương – Hà Nội Ā Ơ Ỵ";
    let prepared = InterFont::new(text).unwrap();
    assert_eq!(prepared.dictionary.name(b"Subtype"), Some(b"Type0".as_slice()));
    assert_eq!(prepared.dictionary.name(b"Encoding"), Some(b"Identity-H".as_slice()));
    let mut doc = Document::new_empty();
    let reference = register(&mut doc, &prepared.dictionary).unwrap();
    let dictionary = doc.dict(&reference).unwrap();
    let metrics = Metrics::from_dict(&doc, &dictionary);
    let encoded = prepared.encode(text).unwrap();
    assert_eq!(metrics.encode(text).unwrap(), encoded);
    assert_eq!(metrics.decode(&encoded), text);
    let cmap = doc.resolve(dictionary.get(b"ToUnicode").unwrap());
    let Object::Stream(cmap) = cmap.as_ref() else { panic!("ToUnicode stream") };
    let cmap = String::from_utf8(cmap.decoded().unwrap()).unwrap();
    let descendant = doc.dict(dictionary.get(b"DescendantFonts").unwrap().as_array().unwrap().first().unwrap()).unwrap();
    assert_eq!(descendant.name(b"Subtype"), Some(b"CIDFontType2".as_slice()));
    let map = doc.resolve(descendant.get(b"CIDToGIDMap").unwrap());
    let Object::Stream(map) = map.as_ref() else { panic!("CIDToGIDMap stream") };
    let map = map.decoded().unwrap();
    let unique = text.chars().collect::<std::collections::BTreeSet<_>>().len();
    let widths = descendant.get(b"W").unwrap().as_array().unwrap();
    assert_eq!(widths.first().and_then(Object::as_int), Some(1));
    assert_eq!(widths.get(1).unwrap().as_array().unwrap().len(), unique);
    assert_eq!(map.len(), (unique + 1) * 2);
    let original = FontRef::new(INTER_REGULAR).unwrap();
    for ch in text.chars() {
        let code = prepared.encode(&ch.to_string()).unwrap();
        let cid = u16::from_be_bytes(code.as_slice().try_into().unwrap());
        assert!(cmap.contains(&format!("<{cid:04X}> <{:04X}>", u32::from(ch))));
        let measured = width(&ch.to_string(), 1.0);
        assert!((metrics.width(u32::from(cid)) - measured).abs() < 1e-8);
        let offset = usize::from(cid) * 2;
        let gid = u16::from_be_bytes(map.get(offset..offset + 2).unwrap().try_into().unwrap());
        assert_eq!(u32::from(gid), original.charmap().map(ch).unwrap().to_u32());
    }
    let subset = crate::pdf::embedded_sfnt(&doc, &dictionary).unwrap();
    assert!(subset.len() < INTER_REGULAR.len() / 2, "{} vs {}", subset.len(), INTER_REGULAR.len());
    let subset = FontRef::new(&subset).unwrap();
    assert_eq!(subset.charmap().map('Z'), None);
    for ch in text.chars() {
        let gid = subset.charmap().map(ch).unwrap();
        assert_eq!(Some(gid), original.charmap().map(ch));
        let metrics = |f: &FontRef<'_>| f.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(gid);
        assert_eq!(metrics(&subset), metrics(&original));
        if !ch.is_whitespace() {
            let outline = |font: &FontRef<'_>| crate::script::bounded_outline(font, gid, 1.0).unwrap();
            assert_eq!(outline(&subset), outline(&original), "{ch}");
        }
    }
}

#[test]
fn inter_selection_keeps_winansi_and_missing_glyph_behavior() {
    for text in ["Hello", "Café — 5€", "", "a\r\nb\t"] {
        assert!(!needed(text), "{text:?}");
    }
    assert!(needed("Nguyễn Thị Hương – Hà Nội"));
    assert!(needed("a\u{0306}\u{0301}"));
    assert_eq!(first_missing("ก"), Some('ก'));
    assert!(!needed("Nguyễn ก"));
    assert!(InterFont::new("ก").is_err());
    let prepared = InterFont::new("ệ\nộ").unwrap();
    assert_eq!(prepared.encode("\t").unwrap(), prepared.encode(" ").unwrap());
    assert!(prepared.encode("Z").is_err());
}

#[test]
fn subset_order_is_deterministic_and_advances_are_additive() {
    let first = InterFont::new("ệộ ệ").unwrap();
    let second = InterFont::new("ộệ ").unwrap();
    assert_eq!(first.dictionary, second.dictionary);
    let units = width("ệộ", 12.0);
    assert!((units - width("ệ", 12.0) - width("ộ", 12.0)).abs() < 1e-10);
}

#[test]
fn latin_extended_subset_roundtrips_every_character_covered_by_inter() {
    let font = FontRef::new(INTER_REGULAR).unwrap();
    let text: String = (0x0100..=0x024f).chain(0x1e00..=0x1eff).filter_map(char::from_u32)
        .filter(|&c| font.charmap().map(c).is_some_and(|g| g.to_u32() != 0)).collect();
    let prepared = InterFont::new(&text).unwrap();
    let mut doc = Document::new_empty();
    let registered = register(&mut doc, &prepared.dictionary).unwrap();
    let dictionary = doc.dict(&registered).unwrap();
    assert_eq!(Metrics::from_dict(&doc, &dictionary).decode(&prepared.encode(&text).unwrap()), text);
}

#[test]
fn malformed_subset_input_and_deep_registration_return_errors_without_writing() {
    assert!(crate::truetype_subset::subset(b"broken", &[]).is_err());
    let mut dictionary = pdfcraft_cos::Dict::new();
    for _ in 0..20 {
        let mut parent = pdfcraft_cos::Dict::new();
        parent.set(b"child".to_vec(), Object::Dict(dictionary));
        dictionary = parent;
    }
    let mut doc = Document::new_empty();
    let before = doc.object_numbers();
    assert!(register(&mut doc, &dictionary).is_err());
    assert_eq!(doc.object_numbers(), before);
}
