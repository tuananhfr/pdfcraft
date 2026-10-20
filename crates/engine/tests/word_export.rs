//! Word export receives visual rows and source font metadata from synthetic PDFs.

use pdfcraft_cos::{Dict, Document, Object, SaveOptions, Stream};
use pdfcraft_engine::{Session, compare::OfficeFormat};
use std::sync::Arc;

fn source(content: &[u8]) -> Arc<Vec<u8>> {
    let mut doc = Document::new_empty();
    let pages = doc.add(Object::Dict(Dict::new()));
    let contents = doc.add(Object::Stream(Stream::flate(Dict::new(), content)));
    let mut font = Dict::new();
    font.set(b"Type".to_vec(), Object::name("Font"));
    font.set(b"Subtype".to_vec(), Object::name("Type1"));
    font.set(b"BaseFont".to_vec(), Object::name("ABCDEF+Arial-BoldMT"));
    font.set(b"Encoding".to_vec(), Object::name("WinAnsiEncoding"));
    let mut fonts = Dict::new();
    fonts.set(b"F1".to_vec(), Object::Dict(font));
    let mut resources = Dict::new();
    resources.set(b"Font".to_vec(), Object::Dict(fonts));
    let mut page = Dict::new();
    page.set(b"Type".to_vec(), Object::name("Page"));
    page.set(b"Parent".to_vec(), Object::Ref(pages));
    page.set(b"Contents".to_vec(), Object::Ref(contents));
    page.set(b"Resources".to_vec(), Object::Dict(resources));
    page.set(b"MediaBox".to_vec(), Object::Array([0, 0, 612, 792].map(Object::Int).to_vec()));
    let page = doc.add(Object::Dict(page));
    doc.update_dict(pages, |p| {
        p.set(b"Type".to_vec(), Object::name("Pages"));
        p.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(page)]));
        p.set(b"Count".to_vec(), Object::Int(1));
    })
    .unwrap();
    let mut root = Dict::new();
    root.set(b"Type".to_vec(), Object::name("Catalog"));
    root.set(b"Pages".to_vec(), Object::Ref(pages));
    let root = doc.add(Object::Dict(root));
    doc.trailer_mut().set(b"Root".to_vec(), Object::Ref(root));
    Arc::new(pdfcraft_cos::write_full(&doc, &SaveOptions::default()).unwrap())
}

#[test]
fn export_pages_keeps_grouped_visual_rows_and_source_font() {
    let mut session = Session::new();
    let bytes = source(b"BT /F1 12 Tf 72 700 Td (First) Tj ( line) Tj 0 -16 Td (Second line) Tj ET");
    let id = session.open("lines.pdf", None, bytes, None).unwrap();
    let doc = session.get(id).unwrap();
    let pages = doc.export_pages();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].blocks.len(), 1);
    let block = &pages[0].blocks[0];
    assert_eq!(block.text, "First line Second line");
    assert_eq!(block.visual_text.as_deref(), Some("First line\nSecond line"));
    assert_eq!(block.font.as_deref(), Some("ABCDEF+Arial-BoldMT"));
    assert!(block.bold);
    let html = String::from_utf8(doc.export_office(OfficeFormat::Html)).unwrap();
    assert!(html.contains("First line Second line"), "{html}");
}

#[test]
fn export_pages_keeps_visual_hyphens_for_word() {
    let mut session = Session::new();
    let bytes = source(b"BT /F1 12 Tf 72 700 Td (wrap-) Tj 0 -16 Td (ped text) Tj ET");
    let id = session.open("hyphen.pdf", None, bytes, None).unwrap();
    let doc = session.get(id).unwrap();
    let pages = doc.export_pages();
    assert_eq!(pages[0].blocks.len(), 1);
    assert_eq!(pages[0].blocks[0].text, "wrapped text");
    assert_eq!(pages[0].blocks[0].visual_text.as_deref(), Some("wrap-\nped text"));
}
