//! PDF objects as JSON, for region payloads. A region's objects travel as a list (`g`) whose
//! first entries are the region's own; references between them are list positions (`{"r":i}`),
//! so the receiver numbers them its own way. Pages and comments outside the region are named by
//! their stable ids (`{"p":"f12.0"}`, `{"a":"nm:…"}`); other pages' trees, the catalogue and
//! other annotations are never copied (null). Everything else a region reaches is copied.
//!
//! Values: `null`, booleans, integers and reals as JSON numbers (a real always has a fraction or
//! exponent), strings `{"s":text}` or `{"b":base64}` (+ `"h":1` for hex), names `{"n":text}` or
//! `{"nb":base64}`, arrays, dictionaries `{"d":{key:value}}` (a key that isn't UTF-8 is NUL +
//! base64) and streams `{"d":{…},"x":base64 of the stored data}`.

use std::collections::{HashMap, HashSet, VecDeque};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use pdfcraft_cos::{Dict, Document as Cos, ObjRef, Object, PdfString, Stream};
use serde_json::{Map, Value};

use super::LiveError;
use crate::stable_ids::{AnnotId, PageId};

/// Objects one region may carry; more is refused rather than sent.
const MAX_OBJECTS: usize = 4096;
/// Nesting of direct objects inside one object.
const MAX_DEPTH: usize = 64;

/// What a region's objects may refer to without copying it.
#[derive(Default)]
pub(crate) struct Anchors {
    pub pages: HashMap<ObjRef, PageId>,
    pub comments: HashMap<ObjRef, AnnotId>,
    /// Every entry of every page's `/Annots`: never copied unless the region owns it.
    pub annotations: HashSet<ObjRef>,
}

pub(crate) struct Encoder<'a> {
    cos: &'a Cos,
    anchors: &'a Anchors,
    index: HashMap<ObjRef, usize>,
    objs: Vec<Value>,
    queue: VecDeque<(ObjRef, usize)>,
}

impl<'a> Encoder<'a> {
    pub fn new(cos: &'a Cos, anchors: &'a Anchors) -> Self {
        Self { cos, anchors, index: HashMap::new(), objs: Vec::new(), queue: VecDeque::new() }
    }

    /// Make `r` one of the region's objects; its position in the list.
    pub fn own(&mut self, r: ObjRef) -> Result<usize, LiveError> {
        if let Some(i) = self.index.get(&r) {
            return Ok(*i);
        }
        if self.objs.len() >= MAX_OBJECTS {
            return Err(LiveError::TooLarge);
        }
        let i = self.objs.len();
        self.index.insert(r, i);
        self.objs.push(Value::Null);
        self.queue.push_back((r, i));
        Ok(i)
    }

    /// A direct value; references it holds are copied, named or dropped.
    pub fn value(&mut self, o: &Object) -> Result<Value, LiveError> {
        self.value_at(o, 0)
    }

    fn value_at(&mut self, o: &Object, depth: usize) -> Result<Value, LiveError> {
        if depth > MAX_DEPTH {
            return Err(LiveError::TooLarge);
        }
        Ok(match o {
            Object::Null => Value::Null,
            Object::Bool(b) => Value::Bool(*b),
            Object::Int(i) => Value::from(*i),
            // A whole real is written as an integer, as saving and reopening a file reads it back:
            // both sides then send the same payload. JSON has no NaN or infinity (a reader treats
            // them as 0 anyway).
            Object::Real(r) if r.fract() == 0.0 && r.abs() < 9e15 => Value::from(*r as i64),
            Object::Real(r) => Value::from(if r.is_finite() { *r } else { 0.0 }),
            Object::String(s) => string(s),
            Object::Name(n) => name(n),
            Object::Array(a) => Value::Array(a.iter().map(|v| self.value_at(v, depth + 1)).collect::<Result<_, _>>()?),
            Object::Dict(d) => one("d", self.dict(d, depth)?),
            Object::Stream(s) => {
                // Writers set `/Length` from the data.
                let mut dict = s.dict.clone();
                dict.remove(b"Length");
                let mut m = Map::new();
                m.insert("d".into(), self.dict(&dict, depth)?);
                m.insert("x".into(), Value::String(B64.encode(&s.raw[..])));
                Value::Object(m)
            }
            Object::Ref(r) => self.reference(*r)?,
        })
    }

    fn dict(&mut self, d: &Dict, depth: usize) -> Result<Value, LiveError> {
        // Sorted, so both sides list the objects in the same order whatever their key order.
        let mut entries: Vec<(&Vec<u8>, &Object)> = d.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        let mut m = Map::new();
        for (k, v) in entries {
            let key = match std::str::from_utf8(k) {
                Ok(s) if !s.starts_with('\0') => s.to_owned(),
                _ => format!("\0{}", B64.encode(k)),
            };
            m.insert(key, self.value_at(v, depth + 1)?);
        }
        Ok(Value::Object(m))
    }

    fn reference(&mut self, r: ObjRef) -> Result<Value, LiveError> {
        if let Some(i) = self.index.get(&r) {
            return Ok(one("r", Value::from(*i)));
        }
        if let Some(page) = self.anchors.pages.get(&r) {
            return Ok(one("p", Value::String(page.to_string())));
        }
        if let Some(id) = self.anchors.comments.get(&r) {
            return Ok(one("a", Value::String(id.to_string())));
        }
        if self.anchors.annotations.contains(&r) || is_structural(&self.cos.get(r)) {
            return Ok(Value::Null);
        }
        let i = self.own(r)?;
        Ok(one("r", Value::from(i)))
    }

    /// Encode every object the region reached; the list `g`.
    pub fn finish(mut self) -> Result<Vec<Value>, LiveError> {
        while let Some((r, i)) = self.queue.pop_front() {
            let obj = self.cos.get(r);
            let v = self.value(&obj)?;
            if let Some(slot) = self.objs.get_mut(i) {
                *slot = v;
            }
        }
        Ok(self.objs)
    }
}

/// The page tree, the catalogue and form fields belong to the document, not to a region.
fn is_structural(o: &Object) -> bool {
    let Some(d) = o.as_dict() else { return false };
    matches!(d.name(b"Type"), Some(b"Page" | b"Pages" | b"Catalog")) || d.contains(b"FT") || (d.contains(b"Kids") && d.contains(b"T"))
}

fn one(key: &str, v: Value) -> Value {
    let mut m = Map::new();
    m.insert(key.into(), v);
    Value::Object(m)
}

fn string(s: &PdfString) -> Value {
    let mut m = Map::new();
    match std::str::from_utf8(&s.bytes) {
        Ok(text) => m.insert("s".into(), Value::String(text.to_owned())),
        Err(_) => m.insert("b".into(), Value::String(B64.encode(&s.bytes))),
    };
    if s.hex {
        m.insert("h".into(), Value::from(1));
    }
    Value::Object(m)
}

fn name(n: &[u8]) -> Value {
    match std::str::from_utf8(n) {
        Ok(text) => one("n", Value::String(text.to_owned())),
        Err(_) => one("nb", Value::String(B64.encode(n))),
    }
}

/// Where a payload's references land in the receiving document.
pub(crate) struct Targets<'a> {
    pub pages: &'a HashMap<PageId, ObjRef>,
    pub comments: &'a HashMap<AnnotId, ObjRef>,
}

/// Write the objects `g` into `cos`: positions in `preset` reuse existing objects, the others are
/// added. Their references, in list order.
pub(crate) fn decode_objects(cos: &mut Cos, g: &[Value], preset: &HashMap<usize, ObjRef>, targets: &Targets) -> Result<Vec<ObjRef>, LiveError> {
    if g.len() > MAX_OBJECTS {
        return Err(LiveError::TooLarge);
    }
    let refs: Vec<ObjRef> = (0..g.len()).map(|i| preset.get(&i).copied().unwrap_or_else(|| cos.add(Object::Null))).collect();
    for (r, v) in refs.iter().zip(g) {
        let obj = decode_value(v, &refs, targets, 0)?;
        cos.set(*r, obj);
    }
    Ok(refs)
}

pub(crate) fn decode_value(v: &Value, refs: &[ObjRef], targets: &Targets, depth: usize) -> Result<Object, LiveError> {
    if depth > MAX_DEPTH {
        return Err(LiveError::TooLarge);
    }
    Ok(match v {
        Value::Null => Object::Null,
        Value::Bool(b) => Object::Bool(*b),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Object::Int(i),
            (None, Some(f)) => Object::Real(f),
            _ => Object::Null,
        },
        Value::String(_) => return Err(malformed("a bare string")),
        Value::Array(a) => Object::Array(a.iter().map(|x| decode_value(x, refs, targets, depth + 1)).collect::<Result<_, _>>()?),
        Value::Object(m) => {
            if let Some(i) = m.get("r") {
                let i = i.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| malformed("a reference"))?;
                return refs.get(i).map(|r| Object::Ref(*r)).ok_or_else(|| malformed("a reference past the objects"));
            }
            if let Some(p) = m.get("p") {
                let page = p.as_str().and_then(PageId::parse).ok_or_else(|| malformed("a page id"))?;
                return Ok(targets.pages.get(&page).map_or(Object::Null, |r| Object::Ref(*r)));
            }
            if let Some(a) = m.get("a") {
                let id = a.as_str().and_then(AnnotId::parse).ok_or_else(|| malformed("a comment id"))?;
                return Ok(targets.comments.get(&id).map_or(Object::Null, |r| Object::Ref(*r)));
            }
            let hex = m.get("h").is_some_and(|h| h.as_u64() == Some(1));
            if let Some(s) = m.get("s") {
                let text = s.as_str().ok_or_else(|| malformed("a string"))?;
                return Ok(Object::String(PdfString { bytes: text.as_bytes().to_vec(), hex }));
            }
            if let Some(b) = m.get("b") {
                return Ok(Object::String(PdfString { bytes: base64(b)?, hex }));
            }
            if let Some(n) = m.get("n") {
                return Ok(Object::Name(n.as_str().ok_or_else(|| malformed("a name"))?.as_bytes().to_vec()));
            }
            if let Some(n) = m.get("nb") {
                return Ok(Object::Name(base64(n)?));
            }
            let d = m.get("d").and_then(Value::as_object).ok_or_else(|| malformed("an object"))?;
            let mut dict = Dict::new();
            for (k, v) in d {
                let key = match k.strip_prefix('\0') {
                    Some(b) => B64.decode(b).map_err(|_| malformed("a key"))?,
                    None => k.as_bytes().to_vec(),
                };
                dict.set(key, decode_value(v, refs, targets, depth + 1)?);
            }
            match m.get("x") {
                Some(x) => Object::Stream(Stream::from_raw(dict, base64(x)?)),
                None => Object::Dict(dict),
            }
        }
    })
}

fn base64(v: &Value) -> Result<Vec<u8>, LiveError> {
    v.as_str().and_then(|s| B64.decode(s).ok()).ok_or_else(|| malformed("base64 data"))
}

pub(crate) fn malformed(what: &str) -> LiveError {
    LiveError::Malformed(what.to_owned())
}
