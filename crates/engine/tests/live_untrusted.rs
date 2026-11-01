//! Co-editing payloads and comment anchors come from other people's editors through the room, so
//! they are untrusted: a payload may only change its own region, one refused leaves the document
//! exactly as it was, and none of it may panic, whatever it holds.

use std::sync::Arc;

use pdfcraft_engine::live::{LiveError, MAX_PAYLOAD_BYTES};
use pdfcraft_engine::{ClientPrefix, DocId, Session};

/// Three pages. Page 1 has the note `file-note-1` (8) and the widget of the field `name` (10);
/// page 2 a note without `/NM` (11). Object 12 is a note no page lists any more.
fn fixture() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [10 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 6 0 R >> >> >> >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 7 0 R /Resources << /Font << /F1 6 0 R >> >> /Annots [8 0 R 10 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Annots [11 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 400] >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "<< /Length 39 >>\nstream\nBT /F1 14 Tf 20 250 Td (Page one) Tj ET\nendstream",
        "<< /Type /Annot /Subtype /Text /NM (file-note-1) /Rect [40 40 60 60] /P 3 0 R /Contents (From the file) >>",
        "<< /Info 1 >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /Rect [20 200 180 220] /P 3 0 R /F 4 /DA (/Helv 10 Tf 0 g) >>",
        "<< /Type /Annot /Subtype /Text /Rect [40 40 60 60] /P 4 0 R /Contents (No name) >>",
        "<< /Type /Annot /Subtype /Text /Rect [10 10 20 20] /Contents (Gone) >>",
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn open(client: &str) -> (Session, DocId) {
    let mut s = Session::new().with_clock(|| 1_800_000_000);
    s.set_client_prefix(ClientPrefix::parse(client).unwrap());
    let id = s.open("live.pdf", None, Arc::new(fixture()), None).unwrap();
    s.set_live(id, true).unwrap();
    (s, id)
}

fn saved(s: &Session, id: DocId) -> Vec<u8> {
    s.save_bytes(id).unwrap().as_ref().clone()
}

/// `payload` under `region` is refused and the document stays byte for byte as it was.
fn refused(s: &mut Session, id: DocId, region: &str, payload: &str) -> LiveError {
    let before = saved(s, id);
    let err = s.apply_remote(id, region, payload).expect_err(region);
    assert_eq!(saved(s, id), before, "{region}: a refused payload changed the document");
    err
}

fn note_payload(page: &str) -> String {
    format!(
        r#"{{"v":1,"page":"{page}","at":0,"g":[{{"d":{{"Type":{{"n":"Annot"}},"Subtype":{{"n":"Text"}},"Rect":[1,1,2,2],"Contents":{{"s":"x"}}}}}}]}}"#
    )
}

/// A comment region named by object number writes that object: only a comment's own object no
/// page lists (or a free number) may be written, never the catalogue, a page, a widget, another
/// comment or a number past the end.
#[test]
fn a_comment_payload_writes_no_object_outside_its_region() {
    let (mut s, id) = open("0000aa");
    for num in [1u32, 2, 3, 6, 10, 8, 4_294_967_295, 4_294_967_294, 500] {
        refused(&mut s, id, &format!("annot:obj:{num}.0"), &note_payload("f3.0"));
    }
    // The generation must match the object's.
    refused(&mut s, id, "annot:obj:12.7", &note_payload("f3.0"));
    // The note no page lists comes back under its own number.
    s.apply_remote(id, "annot:obj:12.0", &note_payload("f3.0")).unwrap();
    assert!(s.annotation_ids(id).iter().any(|a| a.id.to_string() == "obj:12.0" && a.page == 0));
}

/// A payload larger than any editor sends is refused before it is read.
#[test]
fn an_oversized_payload_is_refused_unread() {
    let (mut s, id) = open("0000aa");
    let pad = "A".repeat(MAX_PAYLOAD_BYTES);
    for region in ["*", "pages", "annot:nm:file-note-1", "field:name"] {
        let err = refused(&mut s, id, region, &format!(r#"{{"v":1,"pdf":"{pad}"}}"#));
        assert_eq!(err, LiveError::TooLarge, "{region}");
    }
}
