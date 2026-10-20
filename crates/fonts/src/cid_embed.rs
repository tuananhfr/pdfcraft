//! Pure Inter subset preparation and Identity-H font dictionaries for generated PDF text.

use std::{collections::BTreeMap, fmt::Write as _};

use pdfcraft_cos::{Dict, Object, PdfString, Stream};
use skrifa::{
    FontRef, GlyphId, MetadataProvider,
    instance::{LocationRef, Size},
};

use crate::INTER_REGULAR;

fn normalized(c: char) -> char {
    if matches!(c, '\t' | '\n' | '\r') { ' ' } else { c }
}

/// Whether the complete string needs, and can use, the bundled Inter fallback.
pub fn needed(text: &str) -> bool {
    crate::first_undrawable(text).is_some() && first_missing(text).is_none()
}

/// The first character Inter cannot draw, keeping unsupported-script diagnostics unchanged.
pub fn first_missing(text: &str) -> Option<char> {
    let font = FontRef::new(INTER_REGULAR).ok();
    text.chars().find(|&c| font.as_ref().and_then(|f| f.charmap().map(normalized(c))).is_none_or(|g| g.to_u32() == 0))
}

/// Inter advances in points, without a Helvetica estimate.
pub fn width(text: &str, size: f64) -> f64 {
    let Ok(font) = FontRef::new(INTER_REGULAR) else { return 0.0 };
    let loc = LocationRef::default();
    let units = font.metrics(Size::unscaled(), loc).units_per_em;
    if units == 0 {
        return 0.0;
    }
    let metrics = font.glyph_metrics(Size::unscaled(), loc);
    text.chars().filter_map(|c| font.charmap().map(normalized(c))).filter_map(|g| metrics.advance_width(g)).map(f64::from).sum::<f64>()
        * size / f64::from(units)
}

#[derive(Clone, Debug)]
pub struct InterFont {
    pub dictionary: Dict,
    codes: BTreeMap<char, u16>,
}

impl InterFont {
    /// Prepare all fallible work before callers mutate their document.
    pub fn new(text: &str) -> Result<Self, String> {
        if text.len() > 16 << 20 {
            return Err("Inter text exceeds 16 MiB".into());
        }
        let font = FontRef::new(INTER_REGULAR).map_err(|e| e.to_string())?;
        let mut glyphs = BTreeMap::new();
        for c in text.chars().map(normalized) {
            let gid = font.charmap().map(c).filter(|g| g.to_u32() != 0).ok_or_else(|| crate::undrawable_message("the text", c))?;
            glyphs.insert(c, u16::try_from(gid.to_u32()).map_err(|_| "Inter glyph limit")?);
        }
        if glyphs.len() > 65_534 {
            return Err("Inter character limit".into());
        }
        let mappings: Vec<_> = glyphs.iter().map(|(&c, &g)| (c, g)).collect();
        let program = crate::truetype_subset::subset(INTER_REGULAR, &mappings)?;
        let loc = LocationRef::default();
        let m = font.metrics(Size::unscaled(), loc);
        if m.units_per_em == 0 {
            return Err("Inter has no units per em".into());
        }
        let scale = 1000.0 / f64::from(m.units_per_em);
        let advances = font.glyph_metrics(Size::unscaled(), loc);
        let mut codes = BTreeMap::new();
        let mut gids = vec![0, 0];
        let mut widths = Vec::new();
        let mut cmap = String::from("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /PCInterUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
        for (block, chunk) in mappings.chunks(100).enumerate() {
            let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
            for (index, &(c, gid)) in chunk.iter().enumerate() {
                let cid = u16::try_from(block * 100 + index + 1).map_err(|_| "Inter CID limit")?;
                codes.insert(c, cid);
                gids.extend_from_slice(&gid.to_be_bytes());
                let advance = advances.advance_width(GlyphId::new(u32::from(gid))).ok_or("missing Inter advance")?;
                widths.push(Object::Real(f64::from(advance) * scale));
                let hex: String = c.encode_utf16(&mut [0; 2]).iter().map(|u| format!("{u:04X}")).collect();
                let _ = writeln!(cmap, "<{cid:04X}> <{hex}>");
            }
            cmap.push_str("endbfchar\n");
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        let hash = mappings.iter().fold(0u32, |h, (c, _)| h.wrapping_mul(31).wrapping_add(u32::from(*c)));
        let mut tag = String::new();
        let mut h = hash;
        for _ in 0..6 {
            tag.push(char::from(b'A' + (h % 26) as u8));
            h /= 26;
        }
        let name = format!("{tag}+Inter-Regular");
        let mut descriptor = Dict::new();
        descriptor.set(b"Type".to_vec(), Object::name("FontDescriptor"));
        descriptor.set(b"FontName".to_vec(), Object::name(&name));
        descriptor.set(b"Flags".to_vec(), Object::Int(32));
        descriptor.set(b"ItalicAngle".to_vec(), Object::Real(f64::from(m.italic_angle)));
        descriptor.set(b"Ascent".to_vec(), Object::Real(f64::from(m.ascent) * scale));
        descriptor.set(b"Descent".to_vec(), Object::Real(f64::from(m.descent) * scale));
        descriptor.set(b"CapHeight".to_vec(), Object::Real(f64::from(m.cap_height.unwrap_or(m.ascent)) * scale));
        descriptor.set(b"StemV".to_vec(), Object::Int(80));
        let bounds = m.bounds.ok_or("Inter has no bounds")?;
        descriptor.set(b"FontBBox".to_vec(), Object::Array([bounds.x_min, bounds.y_min, bounds.x_max, bounds.y_max].map(|v| Object::Real(f64::from(v) * scale)).to_vec()));
        let mut file = Dict::new();
        file.set(b"Length1".to_vec(), Object::Int(program.len() as i64));
        descriptor.set(b"FontFile2".to_vec(), Object::Stream(Stream::flate(file, &program)));
        let mut system = Dict::new();
        system.set(b"Registry".to_vec(), PdfString::literal(b"Adobe".to_vec()));
        system.set(b"Ordering".to_vec(), PdfString::literal(b"Identity".to_vec()));
        system.set(b"Supplement".to_vec(), Object::Int(0));
        let mut cid = Dict::new();
        cid.set(b"Type".to_vec(), Object::name("Font"));
        cid.set(b"Subtype".to_vec(), Object::name("CIDFontType2"));
        cid.set(b"BaseFont".to_vec(), Object::name(&name));
        cid.set(b"CIDSystemInfo".to_vec(), Object::Dict(system));
        cid.set(b"FontDescriptor".to_vec(), Object::Dict(descriptor));
        cid.set(b"CIDToGIDMap".to_vec(), Object::Stream(Stream::flate(Dict::new(), &gids)));
        cid.set(b"W".to_vec(), Object::Array(vec![Object::Int(1), Object::Array(widths)]));
        let mut dictionary = Dict::new();
        dictionary.set(b"Type".to_vec(), Object::name("Font"));
        dictionary.set(b"Subtype".to_vec(), Object::name("Type0"));
        dictionary.set(b"BaseFont".to_vec(), Object::name(&name));
        dictionary.set(b"Encoding".to_vec(), Object::name("Identity-H"));
        dictionary.set(b"DescendantFonts".to_vec(), Object::Array(vec![Object::Dict(cid)]));
        dictionary.set(b"ToUnicode".to_vec(), Object::Stream(Stream::flate(Dict::new(), cmap.as_bytes())));
        dictionary.set(b"PCInterSubset".to_vec(), Object::Bool(true));
        Ok(Self { dictionary, codes })
    }

    pub fn encode(&self, text: &str) -> Result<Vec<u8>, String> {
        if text.len() > 16 << 20 {
            return Err("Inter text exceeds 16 MiB".into());
        }
        let mut out = Vec::new();
        for c in text.chars().map(normalized) {
            out.extend_from_slice(&self.codes.get(&c).ok_or("text is outside the prepared Inter subset")?.to_be_bytes());
        }
        Ok(out)
    }
}
