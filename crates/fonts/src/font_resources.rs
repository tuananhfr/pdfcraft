//! Document registration of pure generated font dictionaries.

use pdfcraft_cos::{Dict, Document, Object};

fn validate(object: &Object, depth: usize, budget: &mut usize) -> Result<(), String> {
    if depth > 16 || *budget == 0 {
        return Err("prepared font dictionary exceeds its budget".into());
    }
    *budget -= 1;
    match object {
        Object::Dict(d) => {
            for (_, value) in d.iter() {
                validate(value, depth + 1, budget)?;
            }
        }
        Object::Array(values) => {
            for value in values {
                validate(value, depth + 1, budget)?;
            }
        }
        Object::Stream(stream) => {
            for (_, value) in stream.dict.iter() {
                validate(value, depth + 1, budget)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn promote(doc: &mut Document, object: &Object) -> Object {
    match object {
        Object::Dict(d) => {
            let mut out = Dict::new();
            for (key, value) in d.iter() {
                out.set(key.clone(), promote(doc, value));
            }
            Object::Ref(doc.add(Object::Dict(out)))
        }
        Object::Stream(s) => Object::Ref(doc.add(Object::Stream(s.clone()))),
        Object::Array(values) => Object::Array(values.iter().map(|v| promote(doc, v)).collect()),
        other => other.clone(),
    }
}

/// Promote a prepared font's streams and dictionaries before serializing the PDF.
pub fn register(doc: &mut Document, dictionary: &Dict) -> Result<Object, String> {
    let mut budget = 200_000;
    for (_, value) in dictionary.iter() {
        validate(value, 1, &mut budget)?;
    }
    Ok(promote(doc, &Object::Dict(dictionary.clone())))
}
