//! Co-editing payloads and comment anchors come from other people's editors through the room, so
//! they are untrusted: a payload may only change its own region, one refused leaves the document
//! exactly as it was, and none of it may panic, whatever it holds.

use std::sync::Arc;

use pdfcraft_engine::anchors::Anchor;
use pdfcraft_engine::live::{LiveError, MAX_PAYLOAD_BYTES};
use pdfcraft_engine::{ClientPrefix, DocId, Edit, NewAnnotation, NoteIcon, Session, Shape, Style};

/// Three pages. Page 1 has the note `file-note-1` (8) and the widget of the field `name` (10);
/// page 2 a note without `/NM` (11). Object 12 is a note no page lists any more.
fn objects() -> Vec<String> {
    [
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
    ]
    .map(str::to_owned)
    .to_vec()
}

fn fixture() -> Vec<u8> {
    pdf(&objects().into_iter().map(Some).collect::<Vec<_>>())
}

/// A file of `objs` numbered from 1; `None` is a free entry of generation 1.
fn pdf(objs: &[Option<String>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut entries = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        match o {
            Some(o) => {
                entries.push(format!("{:010} 00000 n \n", out.len()));
                out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
            }
            None => entries.push("0000000000 00001 f \n".to_owned()),
        }
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for e in entries {
        out.extend_from_slice(e.as_bytes());
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

/// A comment's `/Popup` set to a page or another comment by one payload doesn't make that
/// object the comment's own for the next: it is never written, nor moved in any `/Annots`.
#[test]
fn a_popup_pointing_outside_the_comment_is_not_its_own() {
    let (mut s, id) = open("0000aa");
    let page_two = s.page_ids(id)[1].to_string();
    for target in [format!(r#"{{"p":"{page_two}"}}"#), r#"{"a":"obj-11-0"}"#.to_owned(), r#"{"a":"nm:obj-11-0"}"#.to_owned()] {
        let pointing = format!(
            r#"{{"v":1,"page":"f3.0","at":0,"g":[{{"d":{{"Type":{{"n":"Annot"}},"Subtype":{{"n":"Text"}},"NM":{{"s":"file-note-1"}},"Rect":[40,40,60,60],"Popup":{target}}}}}]}}"#
        );
        // Accepted or refused, the next payload must not reach the target.
        let _ = s.apply_remote(id, "annot:nm:file-note-1", &pointing);
        let others =
            |s: &mut Session| (s.page_ids(id), s.annotation_ids(id).into_iter().filter(|a| a.id.to_string() != "nm:file-note-1").collect::<Vec<_>>());
        let before = others(&mut s);
        let hijack = r#"{"v":1,"page":"f3.0","at":0,"g":[{"d":{"Type":{"n":"Annot"},"Subtype":{"n":"Text"},"NM":{"s":"file-note-1"},"Rect":[40,40,60,60],"Popup":{"r":1}}},{"d":{"Type":{"n":"Annot"},"Subtype":{"n":"Popup"},"Rect":[0,0,1,1]}}]}"#;
        let _ = s.apply_remote(id, "annot:nm:file-note-1", hijack);
        assert_eq!(others(&mut s), before, "{target}: another page or comment changed");
        assert_eq!(s.get(id).unwrap().info.pages.len(), 3, "{target}");
    }
}

/// A free object number may still be referred to from elsewhere (here the catalogue's
/// `/OpenAction`): a comment is never written at one, only at a comment's own object.
#[test]
fn a_comment_payload_never_fills_a_free_object_number() {
    let mut objs: Vec<Option<String>> = objects().into_iter().map(Some).collect();
    objs[0] = objs[0].take().map(|c| c.replace("/Pages 2 0 R", "/Pages 2 0 R /OpenAction 9 0 R"));
    objs[8] = None;
    let bytes = pdf(&objs);
    let mut s = Session::new().with_clock(|| 1_800_000_000);
    s.set_client_prefix(ClientPrefix::parse("0000aa").unwrap());
    let id = s.open("live.pdf", None, Arc::new(bytes), None).unwrap();
    s.set_live(id, true).unwrap();
    let payload =
        r#"{"v":1,"page":"f3.0","at":0,"g":[{"d":{"S":{"n":"JavaScript"},"JS":{"s":"app.alert(1)"},"Subtype":{"n":"Text"},"Rect":[1,1,2,2]}}]}"#;
    for region in ["annot:obj:9.1", "annot:obj:9.0"] {
        refused(&mut s, id, region, payload);
    }
}

/// A page listed twice in the order is a malformed payload, not thousands of page moves.
#[test]
fn a_page_order_listing_a_page_twice_is_refused() {
    let (mut s, id) = open("0000aa");
    let order: Vec<&str> = std::iter::repeat_n(["\"f5.0\"", "\"f3.0\"", "\"f4.0\""], 2000).flatten().collect();
    refused(&mut s, id, "pages", &format!(r#"{{"v":1,"order":[{}],"rot":[]}}"#, order.join(",")));
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

/// A comment region installs only a comment: never a widget, a link, a pop-up or anything of a
/// form field's, and its pop-up is a new object or the one it had.
#[test]
fn a_comment_payload_installs_only_a_comment() {
    let (mut s, id) = open("0000aa");
    let root = |extra: &str| {
        format!(r#"{{"v":1,"page":"f3.0","at":0,"g":[{{"d":{{"Type":{{"n":"Annot"}},"NM":{{"s":"new-1"}},"Rect":[1,1,2,2],{extra}}}}}]}}"#)
    };
    for extra in [
        r#""Subtype":{"n":"Widget"},"FT":{"n":"Tx"},"T":{"s":"evil"}"#,
        r#""Subtype":{"n":"Link"},"A":{"d":{"S":{"n":"URI"},"URI":{"s":"https://example.com"}}}"#,
        r#""Subtype":{"n":"Popup"}"#,
        r#""Subtype":{"n":"Text"},"FT":{"n":"Btn"}"#,
        r#""Subtype":{"n":"Text"},"Kids":[]"#,
        r#""Subtype":{"n":"Text"},"Parent":{"a":"nm:file-note-1"}"#,
        r#""Subtype":{"n":"Text"},"Popup":{"a":"nm:file-note-1"}"#,
        r#""Subtype":{"n":"Text"},"Popup":{"p":"f4.0"}"#,
    ] {
        refused(&mut s, id, "annot:nm:new-1", &root(extra));
    }
    s.apply_remote(id, "annot:nm:new-1", &root(r#""Subtype":{"n":"Text"}"#)).unwrap();
    assert!(s.annotation_ids(id).iter().any(|a| a.id.to_string() == "nm:new-1"));
}

/// A comment payload comes out under its region's id and leaves every other comment's: a root
/// renamed to another comment's `/NM` would turn both into object ids (the other one's lock no
/// longer names it), and one renamed to a fresh name would leave a comment no lock names.
#[test]
fn a_comment_payload_keeps_its_id_and_every_other_comments() {
    let (mut s, id) = open("0000aa");
    let named = |nm: &str| {
        format!(
            r#"{{"v":1,"page":"f3.0","at":0,"g":[{{"d":{{"Type":{{"n":"Annot"}},"Subtype":{{"n":"Text"}},"NM":{{"s":"{nm}"}},"Rect":[40,40,60,60],"Contents":{{"s":"edited"}}}}}}]}}"#
        )
    };
    for nm in ["obj-11-0", "fresh-name"] {
        refused(&mut s, id, "annot:nm:file-note-1", &named(nm));
        refused(&mut s, id, "annot:nm:new-2", &named(nm));
    }
    s.apply_remote(id, "annot:nm:file-note-1", &named("file-note-1")).unwrap();
}

/// Ids are lock keys: another spelling of a comment's or a page's id names nothing.
#[test]
fn another_spelling_of_an_id_is_refused() {
    let (mut s, id) = open("0000aa");
    for region in ["annot:obj:012.0", "annot:obj:12.00"] {
        assert!(matches!(refused(&mut s, id, region, &note_payload("f3.0")), LiveError::Malformed(_)), "{region}");
    }
    refused(&mut s, id, "annot:obj:12.0", &note_payload("f03.0"));
}

// ── fuzzing ───────────────────────────────────────────────────────────────────────────────────

/// A small deterministic generator (xorshift64*): the same cases on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n.max(1) as u64).unwrap_or(0)
    }

    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

const NUMBERS: [&str; 12] =
    ["0", "1", "-1", "7", "4294967295", "4294967296", "18446744073709551615", "-9223372036854775808", "1e308", "-1e308", "0.5", "99999999"];

/// `json` with one random change: a byte flipped or dropped, a cut, a number swapped for an
/// extreme one, a reference pointed elsewhere, or a list or object nested deep.
fn mutate(rng: &mut Rng, json: &str) -> String {
    let mut bytes = json.as_bytes().to_vec();
    match rng.below(8) {
        0 if !bytes.is_empty() => {
            let i = rng.below(bytes.len());
            bytes[i] = *rng.pick(b"{}[]\":,0123456789-.erpsdgxnbav\\ ");
        }
        1 if !bytes.is_empty() => {
            let i = rng.below(bytes.len());
            bytes.remove(i);
        }
        2 => bytes.truncate(rng.below(bytes.len() + 1)),
        3 => {
            // Every number in turn may become an extreme one.
            let s = String::from_utf8_lossy(&bytes).into_owned();
            let mut out = String::new();
            let mut chars = s.chars().peekable();
            while let Some(c) = chars.next() {
                if c.is_ascii_digit() && rng.below(3) == 0 {
                    while chars.peek().is_some_and(char::is_ascii_digit) {
                        chars.next();
                    }
                    out.push_str(rng.pick(&NUMBERS));
                } else {
                    out.push(c);
                }
            }
            return out;
        }
        4 => {
            let s = String::from_utf8_lossy(&bytes).into_owned();
            let target = rng.pick(&["{\"r\":0}", "{\"r\":4096}", "{\"r\":-1}", "{\"p\":\"f1.0\"}", "{\"a\":\"obj:1.0\"}", "{\"p\":\"nzzzzzz.1\"}"]);
            return s.replacen("{\"r\":", &format!("{target},\"_\":{{\"r\":"), 1);
        }
        5 => {
            let depth = 1 + rng.below(300);
            let s = String::from_utf8_lossy(&bytes).into_owned();
            return s.replacen("\"g\":[", &format!("\"g\":[{}0{},", "[".repeat(depth), "]".repeat(depth)), 1);
        }
        6 => {
            let s = String::from_utf8_lossy(&bytes).into_owned();
            let key = rng.pick(&["\"page\":\"f3.0\"", "\"page\":\"f4.0\"", "\"at\":18446744073709551615", "\"del\":1", "\"v\":2"]);
            return s.replacen("\"v\":1", &format!("\"v\":1,{key}"), 1);
        }
        _ => {}
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The payloads a real editor sends for each kind of region, made by editing a second session.
fn genuine_payloads() -> Vec<(String, String)> {
    let (mut s, id) = open("0000bb");
    let shape = Shape::Note { at: [100.0, 120.0], icon: NoteIcon::Comment };
    s.apply(id, Edit::AddAnnotation(NewAnnotation { page: 1, style: Style::default_for(&shape), shape, contents: "Hi".into(), author: "B".into() }))
        .unwrap();
    s.apply(id, Edit::MovePages { pages: vec![2], to: 0 }).unwrap();
    let mut out = Vec::new();
    for change in s.take_live_changes(id) {
        for region in change.regions {
            if let Ok(payload) = s.live_payload(id, &region) {
                out.push((region.to_string(), payload));
            }
        }
    }
    for region in ["annot:nm:file-note-1", "annot:obj:11.0", "field:name", "pages", "*"] {
        let region = pdfcraft_engine::live::Region::parse(region).unwrap();
        if let Ok(payload) = s.live_payload(id, &region) {
            out.push((region.to_string(), payload));
        }
    }
    assert!(out.len() >= 5, "a payload of each kind: {:?}", out.iter().map(|o| &o.0).collect::<Vec<_>>());
    out
}

const REGIONS: [&str; 14] = [
    "*",
    "pages",
    "annot:nm:file-note-1",
    "annot:obj:11.0",
    "annot:obj:1.0",
    "annot:obj:4294967295.65535",
    "annot:nm:x",
    "field:name",
    "field:nope",
    "field:\u{202e}",
    "annot:",
    "page",
    "",
    "annot:obj:12.0",
];

/// Thousands of mutated genuine payloads under random region ids: none panics, and every one
/// refused leaves the document as it was.
#[test]
fn mutated_payloads_never_panic_and_a_refused_one_changes_nothing() {
    let genuine = genuine_payloads();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut s, mut id) = open("0000aa");
    let (mut applied, mut refused_count) = (0, 0);
    for round in 0..1500 {
        // A fresh document now and then, so later cases don't all hit one already mangled.
        if round % 100 == 99 {
            (s, id) = open("0000aa");
        }
        let (region, payload) = rng.pick(&genuine).clone();
        let region = if rng.below(4) == 0 { (*rng.pick(&REGIONS)).to_owned() } else { region };
        let mut payload = payload;
        for _ in 0..=rng.below(3) {
            payload = mutate(&mut rng, &payload);
        }
        let before = saved(&s, id);
        match s.apply_remote(id, &region, &payload) {
            Ok(_) => applied += 1,
            Err(_) => {
                refused_count += 1;
                assert_eq!(saved(&s, id), before, "round {round}: {region} refused but the document changed");
            }
        }
    }
    assert!(applied > 0 && refused_count > 0, "both kinds were tried: {applied} applied, {refused_count} refused");
}

/// Random and mutated anchors: reading, placing and resolving them never panics and never
/// touches the document.
#[test]
fn mutated_anchors_never_panic_and_never_change_the_document() {
    let (mut s, id) = open("0000aa");
    let genuine = [
        r#"{"v":1,"region":"annot:nm:file-note-1","pos":{"page":"f3.0","quads":[[40,60,60,60,40,40,60,40]]},"soft":{"index":0,"text":""},"label":"Trang 1"}"#,
        r#"{"v":1,"region":null,"pos":{"page":"f3.0","quads":[[20,265,100,265,20,248,100,248]]},"soft":{"index":0,"text":"Page one"},"label":"Trang 1"}"#,
        r#"{"v":1,"region":"field:name","pos":{"page":null,"quads":[]},"soft":{"index":2,"text":"Page"},"label":"Trang 3"}"#,
    ];
    let before = saved(&s, id);
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut anchors = Vec::new();
    for round in 0..3000 {
        let mut json = (*rng.pick(&genuine)).to_owned();
        for _ in 0..=rng.below(3) {
            json = mutate(&mut rng, &json);
        }
        if round % 500 == 0 {
            // Huge lists of quads and long text.
            json = format!(
                r#"{{"v":1,"pos":{{"quads":[{}]}},"soft":{{"index":0,"text":"{}"}}}}"#,
                vec!["[1,2,3,4,5,6,7,8]"; 5000].join(","),
                "é".repeat(5000)
            );
        }
        if let Ok(anchor) = Anchor::parse(&json) {
            let _ = s.anchor_place(id, &anchor);
            anchors.push(anchor);
        }
    }
    assert!(!anchors.is_empty());
    let _ = s.resolve_anchors(id, &anchors);
    assert_eq!(saved(&s, id), before, "anchors changed the document");
}
