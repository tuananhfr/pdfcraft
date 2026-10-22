//! Memory budget: opening admission and the byte allowances of caches and history.
//!
//! Wasm (32-bit, inside the browser's limits) admits a 1 GiB working set; native defaults to
//! 4 GiB, overridable with `PDFCRAFT_MEMORY_MIB` (64–16384). An estimate above three quarters of
//! the budget opens in light read-only mode; above the budget, opening is refused.
//!
//! The estimate is taken before the whole file is parsed (natively, before it is read): three
//! copies of the input (source buffer, object tables, decoded object streams), 16 KiB per declared
//! page, the largest image's RGBA size, plus the canvas, history, open-time and per-worker
//! decoded-image reserves. Only the first and last 2 MiB are sampled, so compressed object streams
//! and indirect `/Width`/`/Height` values are missed. This is admission, not an allocator limit.

use pdfcraft_render::OpenError;

pub const LIGHT_NOTICE: &str = "Opened in light read-only mode to limit memory use. Editing and large thumbnails are disabled.";
pub const REFUSE_NOTICE: &str = "This PDF exceeds the memory budget. Close other documents or open a smaller PDF.";

const MIB: u64 = 1024 * 1024;
const SAMPLE: usize = 2 * 1024 * 1024;
const INPUT_COPIES: u64 = 3;
const PAGE_BYTES: u64 = 16 * 1024;
const OPEN_RESERVE: u64 = 32 * MIB;
const WORKER_IMAGE_CACHE: u64 = 64 * MIB;

/// Canvas rasters of one document (pages, tiles, thumbnails, organize grid, print previews).
pub const CACHE_BYTES: usize = 256 * 1024 * 1024;
/// Canvas rasters in light mode; also the cap of a single light-mode render.
pub use pdfcraft_render::LIGHT_RASTER_BYTES as LIGHT_CACHE_BYTES;
/// Undo/redo snapshots of one document; a shared source buffer is charged once.
#[cfg(target_arch = "wasm32")]
pub const HISTORY_BYTES: usize = 128 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
pub const HISTORY_BYTES: usize = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Normal,
    Light,
}

#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub bytes: u64,
}

impl Budget {
    /// The budget of the platform this build runs on.
    pub fn platform() -> Self {
        #[cfg(target_arch = "wasm32")]
        let bytes = 1024 * MIB;
        #[cfg(not(target_arch = "wasm32"))]
        let bytes =
            std::env::var("PDFCRAFT_MEMORY_MIB").ok().and_then(|v| v.parse::<u64>().ok()).filter(|v| (64..=16384).contains(v)).unwrap_or(4096) * MIB;
        Self { bytes }
    }

    /// How a file of `file_size` bytes, of which `sample` was read, may be opened.
    pub fn admit(self, file_size: u64, sample: &[u8]) -> Result<Admission, OpenError> {
        let estimate = estimate(file_size, sample);
        if estimate > self.bytes {
            Err(OpenError::Unsupported(REFUSE_NOTICE.into()))
        } else if estimate > self.bytes / 4 * 3 {
            Ok(Admission::Light)
        } else {
            Ok(Admission::Normal)
        }
    }
}

fn number_after(bytes: &[u8], key: &[u8]) -> u64 {
    let Some(at) = bytes.windows(key.len()).position(|w| w == key) else { return 0 };
    let rest = bytes.get(at.saturating_add(key.len())..).unwrap_or_default();
    rest.iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .take_while(u8::is_ascii_digit)
        .fold(0u64, |n, c| n.saturating_mul(10).saturating_add(u64::from(c - b'0')))
}

/// Declared page count and largest direct image size (RGBA bytes) in `sample`.
fn scan(sample: &[u8]) -> (u64, u64) {
    let (mut pages, mut image) = (0u64, 0u64);
    // Bounded windows: no stream decoding, recursion or allocation driven by PDF numbers.
    for dict in sample.split(|c| *c == b'<') {
        let end = dict.iter().position(|c| *c == b'>').unwrap_or(dict.len()).min(4096);
        let dict = dict.get(..end).unwrap_or_default();
        // Outline items carry /Count too (open descendants); only page-tree nodes count pages.
        if dict.windows(5).any(|w| w == b"/Kids") || dict.windows(6).any(|w| w == b"/Pages") {
            pages = pages.max(number_after(dict, b"/Count"));
        }
        if dict.windows(6).any(|w| w == b"/Image") {
            image = image.max(number_after(dict, b"/Width").saturating_mul(number_after(dict, b"/Height")).saturating_mul(4));
        }
    }
    (pages, image)
}

/// Estimated working set of opening a file of `file_size` bytes whose start (and end) is `bytes`.
pub fn estimate(file_size: u64, bytes: &[u8]) -> u64 {
    let (mut pages, mut image) = scan(bytes.get(..bytes.len().min(SAMPLE)).unwrap_or_default());
    if bytes.len() > SAMPLE {
        let (p, i) = scan(bytes.get(bytes.len().saturating_sub(SAMPLE).max(SAMPLE)..).unwrap_or_default());
        pages = pages.max(p);
        image = image.max(i);
    }
    let workers = if cfg!(target_arch = "wasm32") { 1 } else { crate::render_threads() as u64 };
    let reserve = (CACHE_BYTES as u64)
        .saturating_add(HISTORY_BYTES as u64)
        .saturating_add(OPEN_RESERVE)
        .saturating_add(workers.saturating_mul(WORKER_IMAGE_CACHE));
    file_size.saturating_mul(INPUT_COPIES).saturating_add(pages.saturating_mul(PAGE_BYTES)).saturating_add(image).saturating_add(reserve)
}

/// Conservative cost of changed objects, including newly added streams.
pub(crate) fn history_bytes(cos: &pdfcraft_cos::Document) -> usize {
    fn object_bytes(object: &pdfcraft_cos::Object, depth: usize) -> usize {
        use pdfcraft_cos::Object;
        if depth > 64 {
            return usize::MAX;
        }
        let base = std::mem::size_of::<Object>();
        base.saturating_add(match object {
            Object::Name(v) => v.capacity(),
            Object::String(v) => v.bytes.capacity(),
            Object::Array(v) => {
                v.iter().fold(v.capacity().saturating_mul(std::mem::size_of::<Object>()), |n, o| n.saturating_add(object_bytes(o, depth + 1)))
            }
            Object::Dict(v) => v.iter().fold(0usize, |n, (k, o)| n.saturating_add(k.capacity()).saturating_add(object_bytes(o, depth + 1))),
            Object::Stream(v) => v
                .raw
                .len()
                .saturating_add(v.dict.iter().fold(0usize, |n, (k, o)| n.saturating_add(k.capacity()).saturating_add(object_bytes(o, depth + 1)))),
            _ => 0,
        })
    }
    cos.modified_objects().into_iter().fold(std::mem::size_of::<pdfcraft_cos::Document>(), |n, num| {
        n.saturating_add(object_bytes(&cos.get(pdfcraft_cos::ObjRef::new(num, cos.generation(num))), 0))
    })
}

/// Charge a shared source only once, excluding the source the current document already owns.
pub(crate) fn retained_history_bytes<'a>(documents: impl Iterator<Item = &'a pdfcraft_cos::Document>, current: &pdfcraft_cos::Document) -> usize {
    let mut seen = std::collections::HashSet::new();
    seen.insert(std::sync::Arc::as_ptr(current.bytes()));
    documents.fold(0usize, |total, cos| {
        let source = if seen.insert(std::sync::Arc::as_ptr(cos.bytes())) { cos.bytes().len() } else { 0 };
        total.saturating_add(source).saturating_add(history_bytes(cos))
    })
}

/// Read a document file after admitting it. Probing and reading share one handle and the read is
/// bounded, so a file that grows after the probe cannot evade admission.
pub fn read_file(path: &std::path::Path) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let budget = Budget::platform();
    // Checked before opening too: opening a FIFO would block.
    if !std::fs::metadata(path).map_err(|e| e.to_string())?.is_file() {
        return Err("only regular document files can be opened".into());
    }
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("only regular document files can be opened".into());
    }
    let size = metadata.len();
    budget.admit(size, &[]).map_err(|e| e.to_string())?;
    let mut sample = Vec::new();
    file.by_ref().take(SAMPLE as u64).read_to_end(&mut sample).map_err(|e| e.to_string())?;
    if size > SAMPLE as u64 {
        file.seek(SeekFrom::Start(size.saturating_sub(SAMPLE as u64).max(SAMPLE as u64))).map_err(|e| e.to_string())?;
        file.by_ref().take(SAMPLE as u64).read_to_end(&mut sample).map_err(|e| e.to_string())?;
    }
    budget.admit(size, &sample).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take((budget.bytes / INPUT_COPIES).saturating_add(1)).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    budget.admit(bytes.len() as u64, &bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed() -> u64 {
        estimate(0, b"")
    }

    #[test]
    fn admission_thresholds_and_hostile_dimensions() {
        let budget = Budget { bytes: 4 * (fixed() + 300 * MIB) };
        assert_eq!(budget.admit(MIB, b"<< /Type /Pages /Count 1 >>").unwrap(), Admission::Normal);
        // Light above three quarters of the budget, refused above it.
        let light = (budget.bytes / 4 * 3 - fixed()) / INPUT_COPIES + 1;
        assert_eq!(budget.admit(light - 2, b"").unwrap(), Admission::Normal);
        assert_eq!(budget.admit(light, b"").unwrap(), Admission::Light);
        let refused = (budget.bytes - fixed()) / INPUT_COPIES + 1;
        assert_eq!(budget.admit(refused - 1, b"").unwrap(), Admission::Light);
        assert!(budget.admit(refused, b"").is_err());
        assert!(budget.admit(100, b"<< /Subtype /Image /Width 9999999999999999999999 /Height 999999999999999 >>").is_err());
        assert!(budget.admit(100, b"<< /Type /Pages /Count 999999999999999999999 >>").is_err());
    }

    #[test]
    fn only_page_tree_counts_are_pages() {
        assert_eq!(estimate(0, b"<< /Type /Outlines /First 3 0 R /Last 9 0 R /Count 100005 >>"), fixed());
        assert_eq!(estimate(0, b"<< /Type /Pages /Kids [3 0 R] /Count 10 >>") - fixed(), 10 * PAGE_BYTES);
    }

    #[test]
    fn the_largest_image_counts_not_their_sum() {
        let one = b"<< /Subtype /Image /Width 1000 /Height 1000 >>".to_vec();
        let many = one.repeat(50);
        assert_eq!(estimate(0, &many), estimate(0, &one));
        assert_eq!(estimate(0, &one) - fixed(), 4_000_000);
    }

    #[test]
    fn a_large_scan_opens_normally_with_the_default_desktop_budget() {
        // A 200 MiB, 500-page scan with A4 images at 300 dpi.
        let sample = b"<< /Type /Pages /Count 500 >> << /Subtype /Image /Width 2480 /Height 3508 >>";
        assert_eq!(Budget { bytes: 4096 * MIB }.admit(200 * MIB, sample).unwrap(), Admission::Normal);
    }
}

#[cfg(test)]
mod opening_tests {
    use super::*;
    use crate::{Edit, Session};
    use std::sync::Arc;

    #[test]
    fn light_open_and_refusal_preserve_source_and_existing_draft() {
        let mut session = Session::new();
        let blank = session.create_blank(200.0, 300.0, 1).unwrap();
        let id = session.open_new("draft.pdf", blank).unwrap();
        session.apply(id, Edit::SetInfo { key: "Title".into(), value: "Existing draft".into() }).unwrap();
        let draft = session.get(id).unwrap().bytes.clone();
        let bytes = Arc::new(crate::tests::fixture(1));
        let source = bytes.clone();
        // An estimate between three quarters of the budget and the budget: light mode.
        let context = session.open_context().with_memory_budget(Budget { bytes: estimate(bytes.len() as u64, &bytes).saturating_mul(5) / 4 });
        let prepared = context.prepare_open("large.pdf", None, bytes.clone(), None).unwrap();
        let light = session.commit_open(prepared);
        let doc = session.get(light).unwrap();
        assert!(doc.light_memory());
        assert_eq!(doc.read_only_reason.as_deref(), Some(LIGHT_NOTICE));
        assert!(!doc.editable());
        assert!(session.apply(light, Edit::SetInfo { key: "Title".into(), value: "No".into() }).is_err());
        assert!(session.get(light).unwrap().undo_history().is_empty());
        let context = session.open_context().with_memory_budget(Budget { bytes: 10 });
        assert_eq!(context.prepare_open("too-big.pdf", None, bytes.clone(), None).err().unwrap().to_string(), REFUSE_NOTICE);
        assert_eq!(*bytes, *source);
        assert_eq!(session.get(id).unwrap().bytes, draft);
        assert_eq!(session.get(id).unwrap().info.pages.len(), 1);
        assert!(session.get(id).unwrap().dirty);
        // Existing fixtures stay under the light threshold with the platform budget.
        let normal = session.open("normal.pdf", None, bytes, None).unwrap();
        assert!(!session.get(normal).unwrap().light_memory());
        assert!(session.get(normal).unwrap().editable());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod file_tests {
    use super::*;

    #[test]
    fn sparse_oversize_input_is_refused_without_touching_an_existing_draft() {
        let dir = std::env::temp_dir().join(format!("pdfcraft-admission-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let draft = dir.join("draft.pdf");
        let bytes = crate::tests::fixture(1);
        std::fs::write(&draft, &bytes).unwrap();
        let oversized = dir.join("oversized.pdf");
        std::fs::File::create(&oversized).unwrap().set_len(Budget::platform().bytes / INPUT_COPIES + 1).unwrap();
        assert_eq!(read_file(&oversized).unwrap_err(), REFUSE_NOTICE);
        assert_eq!(read_file(&draft).unwrap(), bytes);
        assert_eq!(std::fs::read(&draft).unwrap(), bytes);
        assert!(read_file(&dir).is_err(), "a directory is not a document");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
