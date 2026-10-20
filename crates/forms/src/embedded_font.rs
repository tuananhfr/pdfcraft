//! Register generated Inter resources in the form and the widget's default appearance together.

use pdfcraft_cos::{Document, Object, PdfString, Stream};
use pdfcraft_fonts::cid_embed::{self, InterFont};

use crate::{Field, FieldKind, FormError, Widget, acroform, appearance, flags};

const ORIGINAL_DA: &[u8] = b"PCInterOriginalDA";

fn set_da(doc: &mut Document, object: pdfcraft_cos::ObjRef, fallback: &str, da: &str) -> Result<(), FormError> {
    let original = doc.get(object).as_dict().and_then(|d| d.get(b"DA")).and_then(|o| doc.resolve(o).as_string().cloned()).unwrap_or_else(|| PdfString::text(fallback));
    doc.update_dict(object, |d| {
        if !d.contains(ORIGINAL_DA) {
            d.set(ORIGINAL_DA.to_vec(), original);
        }
        d.set(b"DA".to_vec(), PdfString::text(da));
    })?;
    Ok(())
}

fn restore_da(doc: &mut Document, field: &Field, widget: &Widget) -> Result<Field, FormError> {
    let mut field = field.clone();
    let mut objects = vec![field.obj];
    if widget.obj != field.obj {
        objects.push(widget.obj);
    }
    for object in objects {
        let original = doc.get(object).as_dict().and_then(|d| d.get(ORIGINAL_DA)).and_then(Object::as_string).cloned();
        if let Some(original) = original {
            if object == field.obj {
                field.da = original.to_text();
            }
            doc.update_dict(object, |d| {
                d.set(b"DA".to_vec(), original);
                d.remove(ORIGINAL_DA);
            })?;
        }
    }
    Ok(field)
}

pub(crate) fn appearance(
    doc: &mut Document,
    field: &Field,
    widget: &Widget,
    values: &[String],
    format: bool,
) -> Result<(Stream, Option<char>), FormError> {
    let displayed = if format && matches!(field.kind, FieldKind::Text | FieldKind::Combo) && values.len() == 1 {
        vec![crate::af::format_value(&field.actions.format, values.first().map_or("", String::as_str))]
    } else {
        values.to_vec()
    };
    let text = if field.kind == FieldKind::List {
        field.options.iter().map(|(_, display)| display.as_str()).collect::<Vec<_>>().join("\n")
    } else if field.kind == FieldKind::Combo {
        displayed.iter().map(|v| field.options.iter().find(|(export, _)| export == v).map_or(v.as_str(), |(_, display)| display.as_str())).collect::<Vec<_>>().join(", ")
    } else {
        displayed.first().cloned().unwrap_or_default()
    };
    if field.has(flags::PASSWORD) || !cid_embed::needed(&text) {
        let field = restore_da(doc, field, widget)?;
        return Ok(appearance::field_appearance_drawn(doc, &field, widget, values, format));
    }
    let root = doc.root().ok_or(FormError::NoForm)?;
    let entry = doc.get(root).as_dict().and_then(|c| c.get(b"AcroForm").cloned()).ok_or(FormError::NoForm)?;
    let mut form = acroform(doc).ok_or(FormError::NoForm)?;
    let mut resources = form.get(b"DR").and_then(|o| doc.dict(o)).unwrap_or_default();
    let mut fonts = resources.get(b"Font").and_then(|o| doc.dict(o)).unwrap_or_default();
    let normalized = text.replace(['\t', '\n', '\r'], " ");
    let existing = fonts.iter().find_map(|(name, object)| {
        let dictionary = doc.dict(object)?;
        if !matches!(dictionary.get(b"PCInterSubset"), Some(Object::Bool(true))) {
            return None;
        }
        let metrics = pdfcraft_fonts::pdf::Metrics::from_dict(doc, &dictionary);
        metrics.encode(&normalized)?;
        String::from_utf8(name.clone()).ok().filter(|name| name.starts_with("PCInter") && name.bytes().all(|b| b.is_ascii_alphanumeric()))
    });
    let name = match existing {
        Some(name) => name,
        None => {
            let prepared = InterFont::new(&text).map_err(FormError::Invalid)?;
            let mut index = 0usize;
            while fonts.contains(format!("PCInter{index}").as_bytes()) {
                index = index.saturating_add(1);
            }
            let name = format!("PCInter{index}");
            let font = pdfcraft_fonts::font_resources::register(doc, &prepared.dictionary).map_err(FormError::Invalid)?;
            fonts.set(name.as_bytes().to_vec(), font);
            name
        }
    };
    resources.set(b"Font".to_vec(), Object::Dict(fonts));
    form.set(b"DR".to_vec(), Object::Dict(resources));
    match entry {
        Object::Ref(r) => doc.set(r, Object::Dict(form)),
        _ => doc.update_dict(root, |d| d.set(b"AcroForm".to_vec(), Object::Dict(form)))?,
    }
    let wd = doc.get(widget.obj);
    let old_da = wd.as_dict().and_then(|d| d.get(b"DA")).and_then(|o| doc.resolve(o).as_string().map(PdfString::to_text));
    let da = appearance::parse_da(old_da.as_deref().unwrap_or(&field.da));
    let da = format!("/{name} {} Tf {}", appearance::fmt(da.size), da.color);
    set_da(doc, field.obj, &field.da, &da)?;
    if widget.obj != field.obj {
        set_da(doc, widget.obj, old_da.as_deref().unwrap_or(&field.da), &da)?;
    }
    let mut field = field.clone();
    field.da = da;
    Ok(appearance::field_appearance_drawn(doc, &field, widget, values, format))
}
