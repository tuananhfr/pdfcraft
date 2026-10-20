use std::sync::Arc;

use pdfcraft_cos::{Document, SaveOptions, write_incremental};
use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RequestKind};

const VIETNAMESE: &str = "Nguyễn Thị Hương – Hà Nội";

fn document() -> Document {
    Document::open(Arc::new(crate::tests::fixture(1))).unwrap()
}

fn extract(doc: &Document) -> String {
    let bytes = write_incremental(doc, &SaveOptions::default()).unwrap();
    let mut renderer = PageRenderer::new(Arc::new(bytes), RenderConfig::default());
    let result = renderer.render(RenderRequest { page: 0, kind: RequestKind::Text, ..Default::default() });
    assert!(result.error.is_none(), "{:?}", result.error);
    result.text.unwrap().plain_text()
}

#[test]
fn vietnamese_text_box_and_typewriter_are_extractable_after_reopen_and_restyle() {
    use pdfcraft_annot::{Meta, NewAnnotation, Shape, Style};
    for shape in [
        Shape::TextBox { rect: [10.0, 210.0, 195.0, 270.0], font_size: 10.0 },
        Shape::Typewriter { rect: [10.0, 210.0, 195.0, 270.0], font_size: 10.0 },
    ] {
        let mut doc = document();
        let meta = Meta { date: Some("D:20261021000000Z".into()), id: "vietnamese".into() };
        let new = NewAnnotation { page: 0, shape, style: Style::default(), contents: VIETNAMESE.into(), author: String::new() };
        let index = pdfcraft_annot::add_annotation(&mut doc, &new, &meta).unwrap();
        assert!(extract(&doc).contains(VIETNAMESE));
        pdfcraft_annot::set_contents(&mut doc, 0, index, "Trần Đặng – Huế", &meta).unwrap();
        assert!(extract(&doc).contains("Trần Đặng – Huế"));
        pdfcraft_annot::move_annotation(&mut doc, 0, index, 1.0, 0.0, &meta).unwrap();
        assert!(extract(&doc).contains("Trần Đặng – Huế"));
    }
}

#[test]
fn vietnamese_added_text_is_extractable_and_wraps_with_inter_advances() {
    use pdfcraft_edit::{AddedText, Content, add_content};
    let mut doc = document();
    let text = AddedText { rect: [10.0, 210.0, 195.0, 270.0], text: VIETNAMESE.into(), size: 10.0, ..Default::default() };
    add_content(&mut doc, 0, &Content::Text(text.clone())).unwrap();
    assert!(extract(&doc).contains(VIETNAMESE));
    let narrow = AddedText { rect: [10.0, 210.0, 65.0, 270.0], ..text };
    let lines = pdfcraft_edit::added::lines(&narrow);
    assert_eq!(lines, pdfcraft_fonts::wrap_with(VIETNAMESE, 10.0, 55.0, pdfcraft_fonts::cid_embed::width));
    assert!(lines.len() > 1);
    assert!(lines.iter().all(|line| pdfcraft_fonts::cid_embed::width(line, 10.0) <= 55.0));
}

#[test]
fn vietnamese_field_value_is_extractable_after_save() {
    use pdfcraft_forms::{FieldValue, NewField, add_field, set_value};
    let mut doc = document();
    let name = add_field(&mut doc, 0, [10.0, 210.0, 195.0, 240.0], &NewField::Text { multiline: false }, None).unwrap();
    set_value(&mut doc, &name, &FieldValue::Text(VIETNAMESE.into())).unwrap();
    assert!(extract(&doc).contains(VIETNAMESE));
}

#[test]
fn replacing_and_deleting_added_text_releases_only_its_own_inter_resource() {
    use pdfcraft_edit::{AddedText, Content, add_content, delete_content, update_content};
    let mut doc = document();
    let first = AddedText { rect: [10.0, 210.0, 195.0, 270.0], text: VIETNAMESE.into(), size: 10.0, ..Default::default() };
    let second = AddedText { rect: [10.0, 170.0, 195.0, 200.0], text: "Trần Đặng – Huế".into(), ..first.clone() };
    add_content(&mut doc, 0, &Content::Text(first.clone())).unwrap();
    add_content(&mut doc, 0, &Content::Text(second)).unwrap();
    update_content(&mut doc, 0, 0, &Content::Text(AddedText { text: "Hello".into(), ..first })).unwrap();
    assert!(extract(&doc).contains("Trần Đặng – Huế"));
    delete_content(&mut doc, 0, 1).unwrap();
    let page = pdfcraft_model::pages(&doc).remove(0);
    let resources = doc.dict(page.dict.get(b"Resources").unwrap()).unwrap();
    let fonts = doc.dict(resources.get(b"Font").unwrap()).unwrap();
    assert!(!fonts.iter().any(|(name, _)| name.starts_with(b"PCInter")));
    assert!(extract(&doc).contains("Hello"));
}
