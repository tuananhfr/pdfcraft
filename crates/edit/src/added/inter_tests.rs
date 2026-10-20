use super::*;

#[test]
fn winansi_added_text_keeps_the_legacy_content_bytes_and_font() {
    for (text, bytes) in [("Hello", b"Hello".as_slice()), ("Café — 5€", b"Caf\xe9 \x97 5\x80".as_slice())] {
        let mut doc = Document::new_empty();
        let before = doc.object_numbers();
        let text = AddedText { text: text.into(), rect: [10.0, 10.0, 200.0, 40.0], ..Default::default() };
        let (content, resources) = draw(&mut doc, &Content::Text(text), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], &Dict::new(), false).unwrap();
        let mut expected = b"q 1 0 0 1 0 0 cm\nBT /PCFHelvetica 12 Tf 0 0 0 rg\n1 0 0 1 10 28.6 Tm 0 Tw ".to_vec();
        expected.extend(literal(bytes));
        expected.extend_from_slice(b" Tj\nET\nQ\n");
        assert_eq!(content, expected);
        let font = resources.get(b"Font").unwrap().as_dict().unwrap().get(b"PCFHelvetica").unwrap().as_dict().unwrap();
        assert_eq!(font.name(b"Subtype"), Some(b"Type1".as_slice()));
        assert_eq!(font.name(b"BaseFont"), Some(b"Helvetica".as_slice()));
        assert_eq!(font.name(b"Encoding"), Some(b"WinAnsiEncoding".as_slice()));
        assert_eq!(doc.object_numbers(), before);
    }
}

#[test]
fn inter_added_text_avoids_resource_collisions_and_justifies_with_tj() {
    let mut doc = Document::new_empty();
    let mut taken = Dict::new();
    taken.set(b"PCInter0".to_vec(), Object::Null);
    let text = AddedText {
        text: "Hà Nội Nguyễn Hương".into(), rect: [10.0, 10.0, 90.0, 70.0], align: Align::Justify, ..Default::default()
    };
    let (content, resources) = draw(&mut doc, &Content::Text(text), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], &taken, false).unwrap();
    assert!(resources.get(b"Font").unwrap().as_dict().unwrap().contains(b"PCInter1"));
    assert!(content.windows(4).any(|w| w == b"] TJ"));
    assert!(!content.windows(3).any(|w| w == b" Tw"));
}

#[test]
fn mixed_thai_and_vietnamese_keeps_the_existing_refusal() {
    let text = AddedText { text: "ก Nguyễn".into(), ..Default::default() };
    let expected = if pdfcraft_fonts::thai_has('ก') { 'ễ' } else { 'ก' };
    assert_eq!(first_undrawable(&text), Some(expected));
}
