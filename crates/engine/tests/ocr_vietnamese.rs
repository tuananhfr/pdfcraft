//! Synthetic Vietnamese scans; external models and optional craft-fonts are never required by CI.

use std::sync::Arc;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use pdfcraft_engine::{
    Edit, Session, export,
    ocr::{self, OcrPage, OcrSettings, PlacedWord},
};
use pdfcraft_render::{PageRenderer, RenderRequest};
use unicode_normalization::UnicodeNormalization;

const PHRASES: &[&str] = &["Cộng hòa xã hội chủ nghĩa Việt Nam", "Nguyễn", "đường", "Ở", "ữ"];

fn png_lines(font: &[u8]) -> Vec<u8> {
    let font = FontRef::try_from_slice(font).unwrap();
    let scaled = font.as_scaled(PxScale::from(54.0));
    let (width, height) = (1100u32, 440u32);
    let mut rgba = vec![255; (width * height * 4) as usize];
    for (row, text) in PHRASES.iter().enumerate() {
        let mut x = 40.0;
        for c in text.chars() {
            let id = scaled.glyph_id(c);
            assert_ne!(id.0, 0, "fixture font lacks {c}");
            let glyph = id.with_scale_and_position(scaled.scale(), point(x, 70.0 + row as f32 * 75.0));
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, coverage| {
                    let px = gx as i32 + bounds.min.x as i32;
                    let py = gy as i32 + bounds.min.y as i32;
                    if px >= 0 && py >= 0 && px < width as i32 && py < height as i32 {
                        let offset = (py as usize * width as usize + px as usize) * 4;
                        let ink = (255.0 * (1.0 - coverage)) as u8;
                        rgba[offset..offset + 3].fill(ink);
                    }
                });
            }
            x += scaled.h_advance(id);
        }
    }
    export::encode_png(width, height, &rgba).unwrap()
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (previous + usize::from(ca != *cb)).min(row[j] + 1).min(old + 1);
            previous = old;
        }
    }
    row[b.len()]
}

fn extracted(s: &Session, id: pdfcraft_engine::DocId, page: usize) -> String {
    s.get(id).unwrap().text_lines(page).iter().map(|l| l.text.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

fn image_streams(bytes: &[u8]) -> Vec<(lopdf::ObjectId, Vec<u8>)> {
    let doc = lopdf::Document::load_mem(bytes).unwrap();
    doc.objects
        .iter()
        .filter_map(|(id, object)| {
            let stream = object.as_stream().ok()?;
            (stream.dict.get(b"Subtype").and_then(lopdf::Object::as_name).ok() == Some(b"Image")).then(|| (*id, stream.content.clone()))
        })
        .collect()
}

#[test]
fn vietnamese_ocr_recognizes_rotated_scan_and_round_trips_without_changing_images() {
    if !ocr::available_for_language("vi") {
        eprintln!("skipped: Vietnamese OCR models not installed");
        return;
    }
    let Some(font) = pdfcraft_fonts::CRAFT_FONTS.iter().find(|f| f.family == "Noto Sans" && f.style == "Regular") else {
        eprintln!("skipped: CRAFT_FONTS_DIR has no Noto Sans Regular fixture font");
        return;
    };
    let png = png_lines(font.bytes);
    let mut s = Session::new();
    let image = image::load_from_memory(&png).unwrap().to_rgba8();
    let turned = image::imageops::rotate270(&image);
    let turned = export::encode_png(turned.width(), turned.height(), turned.as_raw()).unwrap();
    let bytes = s.create_from_images(&[("scan.png".into(), png), ("turned.png".into(), turned)]).unwrap();
    let id = s.open("vietnamese.pdf", None, bytes, None).unwrap();
    // Counter-rotate pixels and set /Rotate so the rendered page is upright.
    s.apply(id, Edit::RotatePages { pages: vec![1], degrees: 90 }).unwrap();
    let baseline = s.get(id).unwrap().bytes.clone();
    let before_images: Vec<_> = (0..2).map(|p| s.get(id).unwrap().page_images(p)).collect();
    let settings = OcrSettings { language: "vi".into(), dpi: 150.0, ..Default::default() };
    let job = s.ocr_job(id, &[], settings).unwrap();
    let recognizer = ocr::engine_for_language("vi").unwrap();
    let found = job.run(&recognizer, |_, _| true);
    let source = PHRASES.join(" ");
    let recognized = found[0].text();
    assert_eq!(recognized.nfc().collect::<String>(), recognized);
    let cer = distance(&source, &recognized) as f64 / source.chars().count() as f64;
    eprintln!("Vietnamese page source={source:?}, recognized={recognized:?}, CER={cer:.6}");
    // The 200-line reference corpus measured 1.06%; allow 5% for detector crops and rasterization,
    // including the known model-level C/c confusion, without hiding case errors.
    assert!(cer <= 0.05, "CER {cer}: {recognized:?}");
    for exact in ["Nguyễn", "đường", "Ở", "ữ"] {
        assert!(recognized.contains(exact), "{recognized}");
    }
    assert_eq!(found.len(), 2);
    assert_eq!(found[1].text(), recognized, "rotation must preserve recognition");
    assert_eq!(found[0].words.len(), found[1].words.len());
    let view_points = |page: usize, word: &PlacedWord| {
        let info = &s.get(id).unwrap().info.pages[page];
        [word.origin, [word.origin[0] + word.across[0], word.origin[1] + word.across[1]], [word.origin[0] + word.up[0], word.origin[1] + word.up[1]]]
            .map(|p| info.user_to_view(p[0] as f32, p[1] as f32))
    };
    for (upright, rotated) in found[0].words.iter().zip(&found[1].words) {
        for (a, b) in view_points(0, upright).into_iter().zip(view_points(1, rotated)) {
            assert!((a[0] - b[0]).abs() <= 2.0 && (a[1] - b[1]).abs() <= 2.0, "rotated word placement: {a:?} vs {b:?}");
        }
    }
    s.apply_ocr(id, &found).unwrap();
    assert_eq!(s.get(id).unwrap().undo_history().len(), 2);
    let saved = s.save_copy_bytes(id).unwrap();
    assert_eq!(image_streams(&baseline), image_streams(&saved));
    if let Some(dir) = std::env::var_os("PDFCRAFT_OCR_TEST_ARTIFACTS") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vietnamese-searchable.pdf"), &*saved).unwrap();
        std::fs::write(dir.join("recognized.txt"), &recognized).unwrap();
    }
    let reopened = s.open("saved.pdf", None, saved, None).unwrap();
    for (page, images) in before_images.iter().enumerate() {
        assert_eq!(extracted(&s, reopened, page), recognized);
        let after_images = s.get(id).unwrap().page_images(page);
        assert_eq!(after_images.len(), images.len());
        for (before, after) in images.iter().zip(&after_images) {
            assert_eq!(
                (after.object, after.rect, after.matrix, after.width, after.height),
                (before.object, before.rect, before.matrix, before.width, before.height)
            );
        }
        let render =
            |bytes: Arc<Vec<u8>>| PageRenderer::new(bytes, Default::default()).render(RenderRequest { page, scale: 1.0, ..Default::default() });
        let before = render(baseline.clone());
        let after = render(s.get(reopened).unwrap().bytes.clone());
        assert!(before.error.is_none() && after.error.is_none());
        assert_eq!(before.rgba, after.rgba);
    }
    s.undo(id).unwrap();
    assert_eq!(s.get(id).unwrap().bytes, baseline);
    assert!(extracted(&s, id, 0).is_empty() && extracted(&s, id, 1).is_empty());
    s.redo(id).unwrap();
    assert_eq!(extracted(&s, id, 0), recognized);
}

#[test]
fn vietnamese_ocr_unicode_subsets_survive_repeated_stamps_and_one_batch_undo() {
    let mut s = Session::new();
    let bytes = s.create_blank(612.0, 792.0, 2).unwrap();
    let id = s.open("unicode.pdf", None, bytes.clone(), None).unwrap();
    let placed = |text: &str, y| PlacedWord { text: text.into(), origin: [20.0, y], across: [280.0, 0.0], up: [0.0, 16.0] };
    let pages: Vec<_> = (0..2).map(|page| OcrPage { page, words: vec![placed(PHRASES[0], 700.0)], already_on_page: 0, skipped: None }).collect();
    s.apply_ocr(id, &pages).unwrap();
    s.apply_ocr(id, &[OcrPage { page: 0, words: vec![placed("Nguyễn đường Ở ữ", 650.0)], already_on_page: 0, skipped: None }]).unwrap();
    let saved = s.save_copy_bytes(id).unwrap();
    let reopened = s.open("saved.pdf", None, saved, None).unwrap();
    assert_eq!(extracted(&s, reopened, 0), format!("{} Nguyễn đường Ở ữ", PHRASES[0]));
    assert_eq!(extracted(&s, reopened, 1), PHRASES[0]);
    s.undo(id).unwrap();
    assert_eq!(extracted(&s, id, 0), PHRASES[0]);
    s.undo(id).unwrap();
    assert_eq!(s.get(id).unwrap().bytes, bytes);
}

#[test]
fn vietnamese_ocr_every_precomposed_letter_round_trips() {
    let mut letters = String::from("Đđ");
    for base in "aăâeêioôơuưyAĂÂEÊIOÔƠUƯY".chars() {
        for tone in ["", "\u{301}", "\u{300}", "\u{309}", "\u{303}", "\u{323}"] {
            letters.extend(format!("{base}{tone}").nfc());
        }
    }
    let mut s = Session::new();
    let bytes = s.create_blank(612.0, 792.0, 1).unwrap();
    let id = s.open("alphabet.pdf", None, bytes, None).unwrap();
    s.apply_ocr(
        id,
        &[OcrPage {
            page: 0,
            words: vec![PlacedWord { text: letters.clone(), origin: [20.0, 700.0], across: [570.0, 0.0], up: [0.0, 20.0] }],
            already_on_page: 0,
            skipped: None,
        }],
    )
    .unwrap();
    let saved = s.save_copy_bytes(id).unwrap();
    let reopened = s.open("saved.pdf", None, saved, None).unwrap();
    assert_eq!(extracted(&s, reopened, 0), letters);
}
