//! Draw added Latin Unicode text with a subset of the bundled Inter face.

use pdfcraft_cos::{Dict, Document, Object};
use pdfcraft_fonts::{
    cid_embed::{self, InterFont},
    literal,
};

use crate::{
    EditError,
    added::{AddedText, Align, lines, text_rect},
    n,
};

pub(crate) fn draw(doc: &mut Document, text: &AddedText, taken: &Dict, out: &mut Vec<u8>) -> Result<Dict, EditError> {
    let prepared = InterFont::new(&text.text).map_err(EditError::Invalid)?;
    let mut index = 0usize;
    while taken.contains(format!("PCInter{index}").as_bytes()) {
        index = index.saturating_add(1);
    }
    let name = format!("PCInter{index}");
    let font = pdfcraft_fonts::font_resources::register(doc, &prepared.dictionary).map_err(EditError::Invalid)?;
    let mut fonts = Dict::new();
    fonts.set(name.as_bytes().to_vec(), font);
    let mut resources = Dict::new();
    resources.set(b"Font".to_vec(), Object::Dict(fonts));
    let rect = text_rect(text);
    let [r, g, b] = text.color.map(|v| v.clamp(0.0, 1.0));
    out.extend(format!("BT /{name} {} Tf {} {} {} rg\n", n(text.size), n(r), n(g), n(b)).bytes());
    let lines = lines(text);
    for (i, line) in lines.iter().enumerate() {
        let width = cid_embed::width(line, text.size);
        let x = match text.align {
            Align::Left | Align::Justify => rect[0],
            Align::Center => rect[0] + (rect[2] - rect[0] - width) / 2.0,
            Align::Right => rect[2] - width,
        };
        let y = rect[3] - (i as f64 * 1.2 + 0.95) * text.size;
        out.extend(format!("1 0 0 1 {} {} Tm ", n(x), n(y)).bytes());
        let spaces = line.matches(' ').count();
        if text.align == Align::Justify && i + 1 < lines.len() && spaces > 0 && text.size > 0.0 {
            // Tw applies only to single-byte code 32; Identity-H needs explicit TJ adjustments.
            let extra = (rect[2] - rect[0] - width).max(0.0) * 1000.0 / text.size / spaces as f64;
            out.push(b'[');
            for part in line.split_inclusive(' ') {
                out.extend(literal(&prepared.encode(part).map_err(EditError::Invalid)?));
                if part.ends_with(' ') {
                    out.extend(format!(" {} ", n(-extra)).bytes());
                }
            }
            out.extend_from_slice(b"] TJ\n");
        } else {
            out.extend(literal(&prepared.encode(line).map_err(EditError::Invalid)?));
            out.extend_from_slice(b" Tj\n");
        }
    }
    out.extend_from_slice(b"ET\n");
    Ok(resources)
}
