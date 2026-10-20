//! Word text runs preserve PDF visual rows and font names without embedding font assets.

use crate::{esc, hex};

fn font_style(name: &str) -> Option<(&str, bool, bool)> {
    let name = name.trim();
    let name = match name.split_once('+') {
        Some((prefix, family)) if prefix.len() == 6 && prefix.bytes().all(|b| b.is_ascii_uppercase()) => family.trim(),
        _ => name,
    };
    if name.is_empty() || name.eq_ignore_ascii_case("unknown") {
        return None;
    }
    let (family, style) = name.rsplit_once('-').unwrap_or((name, ""));
    let style = style.to_ascii_lowercase();
    let style = style.strip_suffix("mt").unwrap_or(&style);
    let recognized =
        matches!(style, "bold" | "italic" | "oblique" | "bolditalic" | "boldoblique" | "regular" | "roman" | "book" | "medium" | "black" | "heavy");
    let family = if recognized { family } else { name };
    let family = match family.to_ascii_lowercase().as_str() {
        "arial" | "arialmt" => "Arial",
        "timesnewroman" | "timesnewromanps" | "timesnewromanpsmt" => "Times New Roman",
        "couriernew" | "couriernewps" | "couriernewpsmt" => "Courier New",
        _ => family,
    };
    if family.trim().is_empty() {
        return None;
    }
    Some((
        family,
        recognized && matches!(style, "bold" | "bolditalic" | "boldoblique" | "black" | "heavy"),
        recognized && (style.contains("italic") || style.contains("oblique")),
    ))
}

pub(crate) fn run_xml(text: &str, size: f64, bold: bool, italic: bool, color: [f64; 3], font: Option<&str>) -> String {
    let font = font.and_then(font_style);
    let mut rpr = String::new();
    if let Some((family, _, _)) = font {
        let family = esc(family);
        rpr.push_str(&format!("<w:rFonts w:ascii=\"{family}\" w:hAnsi=\"{family}\" w:cs=\"{family}\" w:eastAsia=\"{family}\"/>"));
    }
    if bold || font.is_some_and(|(_, bold, _)| bold) {
        rpr.push_str("<w:b/>");
    }
    if italic || font.is_some_and(|(_, _, italic)| italic) {
        rpr.push_str("<w:i/>");
    }
    if let Some(h) = hex(color) {
        rpr.push_str(&format!("<w:color w:val=\"{h}\"/>"));
    }
    let half_points = if size.is_finite() { (size * 2.0).round().clamp(2.0, 3276.0) as i64 } else { 24 };
    rpr.push_str(&format!("<w:sz w:val=\"{half_points}\"/>"));
    let mut xml = format!("<w:r><w:rPr>{rpr}</w:rPr>");
    let mut chars = text.chars().peekable();
    let mut line = String::new();
    while let Some(ch) = chars.next() {
        if matches!(ch, '\r' | '\n') {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            xml.push_str(&format!("<w:t xml:space=\"preserve\">{}</w:t><w:br/>", esc(&line)));
            line.clear();
        } else {
            line.push(ch);
        }
    }
    xml.push_str(&format!("<w:t xml:space=\"preserve\">{}</w:t></w:r>", esc(&line)));
    xml
}

#[cfg(test)]
mod tests {
    use crate::tests::part;
    use crate::{Block, Page, docx};

    fn page() -> Page {
        Page {
            width: 612.0,
            height: 792.0,
            blocks: vec![Block {
                text: "First line Second line".into(),
                visual_text: Some("First line\nSecond line".into()),
                font: Some("ABCDEF+Arial-BoldMT".into()),
                rect: [72.0, 700.0, 300.0, 728.0],
                size: 12.0,
                bold: false,
                italic: false,
                color: [0.0; 3],
                rotation: 0.0,
            }],
            images: Vec::new(),
        }
    }

    #[test]
    fn docx_keeps_visual_lines_in_one_paragraph() {
        let xml = part(&docx(&[page()], "Lines"), "word/document.xml");
        assert_eq!(xml.matches("<w:p>").count(), 1, "{xml}");
        assert_eq!(xml.matches("<w:br/>").count(), 1, "{xml}");
        assert!(xml.contains("First line</w:t><w:br/><w:t xml:space=\"preserve\">Second line"), "{xml}");
    }

    #[test]
    fn docx_maps_subset_postscript_font_to_family_and_style() {
        let xml = part(&docx(&[page()], "Fonts"), "word/document.xml");
        assert!(xml.contains("<w:rFonts w:ascii=\"Arial\" w:hAnsi=\"Arial\" w:cs=\"Arial\" w:eastAsia=\"Arial\"/>"), "{xml}");
        assert!(xml.contains("<w:b/>"), "{xml}");
        assert!(xml.contains("<w:sz w:val=\"24\"/>"), "{xml}");
        assert!(!xml.contains("ABCDEF+") && !xml.contains("BoldMT"), "{xml}");
        for (name, family, bold, italic) in [
            ("ArialMT", "Arial", false, false),
            ("Arial-BoldItalicMT", "Arial", true, true),
            ("TimesNewRomanPS-ItalicMT", "Times New Roman", false, true),
            ("CourierNewPS-BoldMT", "Courier New", true, false),
            ("Helvetica-Oblique", "Helvetica", false, true),
            ("Custom-Family", "Custom-Family", false, false),
        ] {
            assert_eq!(super::font_style(name), Some((family, bold, italic)), "{name}");
        }
    }

    #[test]
    fn docx_omits_unknown_font_and_preserves_explicit_style() {
        for font in [None, Some(""), Some("  "), Some("Unknown"), Some("ABCDEF+")] {
            let mut page = page();
            page.blocks[0].font = font.map(str::to_owned);
            page.blocks[0].bold = true;
            page.blocks[0].italic = true;
            let xml = part(&docx(&[page], "Unknown font"), "word/document.xml");
            assert!(!xml.contains("<w:rFonts"), "{xml}");
            assert!(xml.contains("<w:b/><w:i/>"), "{xml}");
        }
    }

    #[test]
    fn docx_handles_explicit_breaks_without_inventing_breaks_for_spaces() {
        for (text, count) in [("First\r\nSecond\rThird\n", 3), ("First\n\nThird", 2), ("First Second", 0)] {
            let mut page = page();
            page.blocks[0].text = text.into();
            page.blocks[0].visual_text = None;
            let xml = part(&docx(&[page], "Breaks"), "word/document.xml");
            assert_eq!(xml.matches("<w:br/>").count(), count, "{xml}");
            let body = xml.split_once("<w:body>").unwrap().1;
            assert!(!body.contains('\r') && !body.contains('\n'), "{xml}");
        }
    }

    #[test]
    fn docx_keeps_font_and_breaks_in_positioned_text_and_table_cells() {
        let mut positioned = page();
        positioned.blocks[0].rotation = 45.0;
        let xml = part(&docx(&[positioned], "Positioned"), "word/document.xml");
        assert!(xml.contains("<w:txbxContent>"), "{xml}");
        assert!(xml.contains("<w:rFonts w:ascii=\"Arial\""), "{xml}");
        assert_eq!(xml.matches("<w:br/>").count(), 1, "{xml}");
        let mut table = page();
        let template = table.blocks.remove(0);
        for row in 0..3 {
            for col in 0..2 {
                let mut cell = template.clone();
                let (x, y) = (72.0 + f64::from(col) * 200.0, 700.0 - f64::from(row) * 40.0);
                cell.rect = [x, y, x + 120.0, y + 28.0];
                table.blocks.push(cell);
            }
        }
        let xml = part(&docx(&[table], "Table"), "word/document.xml");
        assert!(xml.contains("<w:tbl>"), "{xml}");
        assert_eq!(xml.matches("<w:rFonts w:ascii=\"Arial\"").count(), 6, "{xml}");
        assert_eq!(xml.matches("<w:br/>").count(), 6, "{xml}");
    }

    #[test]
    fn docx_escapes_text_and_font_attributes() {
        let mut page = page();
        page.blocks[0].font = Some("A&B\"<Family>".into());
        page.blocks[0].visual_text = Some("A&B\n<C>".into());
        let xml = part(&docx(&[page], "Escapes"), "word/document.xml");
        assert!(xml.contains("w:ascii=\"A&amp;B&quot;&lt;Family&gt;\""), "{xml}");
        assert!(xml.contains("A&amp;B</w:t><w:br/><w:t xml:space=\"preserve\">&lt;C&gt;"), "{xml}");
    }
}
