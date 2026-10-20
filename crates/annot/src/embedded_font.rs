//! Materialize the pure FreeText builder's prepared Inter font in its destination document.

use pdfcraft_cos::{Document, Object, Stream};

use crate::AnnotError;

pub(crate) fn register(doc: &mut Document, stream: &mut Stream) -> Result<(), AnnotError> {
    let Some(mut resources) = stream.dict.get(b"Resources").and_then(Object::as_dict).cloned() else { return Ok(()) };
    let Some(mut fonts) = resources.get(b"Font").and_then(Object::as_dict).cloned() else { return Ok(()) };
    let Some(font) = fonts.get(b"PCInter").and_then(Object::as_dict) else { return Ok(()) };
    if !matches!(font.get(b"PCInterSubset"), Some(Object::Bool(true))) {
        return Ok(());
    }
    let font = pdfcraft_fonts::font_resources::register(doc, font).map_err(AnnotError::Invalid)?;
    fonts.set(b"PCInter".to_vec(), font);
    resources.set(b"Font".to_vec(), Object::Dict(fonts));
    stream.dict.set(b"Resources".to_vec(), Object::Dict(resources));
    Ok(())
}
