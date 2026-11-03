//! What the shell's "Nhận xét" panel heads a thread with (`label`): the page and the text the
//! anchor marks, else the part of the page it is on. Written with the anchor, never read back.

use pdfcraft_engine::anchors::{self, Anchor};
use pdfcraft_engine::{DocId, Session};
use pdfcraft_render::PageInfo;

/// Characters of the marked text a label quotes (wordcraft's limit).
const LABEL_QUOTE_CHARS: usize = 60;

fn label(anchor: &Anchor, page: Option<&PageInfo>) -> String {
    let number = anchor.index.saturating_add(1).to_string();
    let args = [("label", number.as_str())];
    if !anchor.text.is_empty() {
        let page = crate::i18n::fmt(tl!("Page {label}"), &args);
        return format!("{page} · “{}”", anchors::shorten(&anchor.text, LABEL_QUOTE_CHARS));
    }
    let centre = page
        .filter(|p| p.height > 0.0)
        .and_then(|p| Some((anchor.shown_box(p)?, f64::from(p.height))))
        .map(|(b, height)| (b[1] + b[3]) / 2.0 / height)
        .filter(|y| y.is_finite());
    let template = match centre {
        Some(y) if y < 1.0 / 3.0 => tl!("Page {label} · top"),
        Some(y) if y < 2.0 / 3.0 => tl!("Page {label} · middle"),
        Some(_) => tl!("Page {label} · bottom"),
        None => tl!("Page {label}"),
    };
    crate::i18n::fmt(template, &args)
}

/// `anchor` as the shell stores it, for document `id` as it is now.
pub(super) fn anchor_json(session: &Session, id: DocId, anchor: &Anchor) -> String {
    let page = session.get(id).and_then(|d| d.info.pages.get(anchor.index));
    anchor.to_json(&label(anchor, page), page)
}
