//! `erpHost` rules 1–9 against a fake shell, natively.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use pdfcraft_engine::{DocId, Edit, FieldValue};

use super::*;
use crate::PdfCraftApp;
use crate::editing::SaveTarget;

const FORM: &[u8] = include_bytes!("../../tests/data/form.pdf");

#[derive(Default)]
struct Log {
    now: f64,
    saves: Vec<(String, u64, SaveReason)>,
    waiting: Vec<Reply>,
    calls: Vec<String>,
}

#[derive(Clone, Default)]
struct Fake(Rc<RefCell<Log>>);

impl Host for Fake {
    fn save(&self, doc_id: &str, bytes: &[u8], revision: u64, reason: SaveReason, reply: Reply) {
        assert!(bytes.starts_with(b"%PDF"), "the document's own bytes");
        let mut log = self.0.borrow_mut();
        log.saves.push((doc_id.to_string(), revision, reason));
        log.waiting.push(reply);
    }
    fn dirty(&self, doc_id: &str, dirty: bool) {
        self.0.borrow_mut().calls.push(format!("dirty({doc_id}, {dirty})"));
    }
    fn opened(&self, doc_id: &str, ok: bool, reason: Option<&str>) {
        self.0.borrow_mut().calls.push(format!("opened({doc_id}, {ok}, {reason:?})"));
    }
    fn closed(&self, doc_id: &str) {
        self.0.borrow_mut().calls.push(format!("closed({doc_id})"));
    }
    fn now_ms(&self) -> f64 {
        self.0.borrow().now
    }
}

impl Fake {
    /// The shell answers the oldest save waiting.
    fn answer(&self, result: SaveResult) {
        let reply = self.0.borrow_mut().waiting.remove(0);
        reply(result);
    }
    fn saves(&self) -> Vec<(String, u64, SaveReason)> {
        self.0.borrow().saves.clone()
    }
    fn waiting(&self) -> usize {
        self.0.borrow().waiting.len()
    }
    fn take_calls(&self) -> Vec<String> {
        std::mem::take(&mut self.0.borrow_mut().calls)
    }
}

fn app() -> (PdfCraftApp, Fake) {
    let fake = Fake::default();
    let mut app = PdfCraftApp::new();
    app.online = Some(Online::new(Box::new(fake.clone())));
    (app, fake)
}

fn deliver(app: &PdfCraftApp, doc_id: &str, format: &str, bytes: Vec<u8>) {
    let doc = HostDocument::new(doc_id.into(), Some(1.0), "Hợp đồng.pdf".into(), format.into(), bytes);
    app.online.as_ref().unwrap().inbox().lock().unwrap().push(doc);
}

/// Host document `doc_id` (form.pdf), open and active.
fn open(app: &mut PdfCraftApp, doc_id: &str) -> DocId {
    deliver(app, doc_id, "pdf", FORM.to_vec());
    app.open_host_documents();
    app.active_ids().unwrap().1
}

fn edit(app: &mut PdfCraftApp, id: DocId, value: &str) {
    app.active = app.views.iter().position(|v| v.id == id);
    assert!(app.apply_edit(Edit::SetFieldValue { name: "name".into(), value: FieldValue::Text(value.into()) }));
}

/// One frame at `now`.
fn tick(app: &mut PdfCraftApp, fake: &Fake, now: f64) {
    fake.0.borrow_mut().now = now;
    app.online_step();
}

fn dirty(app: &PdfCraftApp, id: DocId) -> bool {
    app.session.get(id).unwrap().dirty
}

fn generation(app: &PdfCraftApp, id: DocId) -> u64 {
    app.session.get(id).unwrap().edit_generation()
}

fn saved() -> SaveResult {
    SaveResult::Saved { version: Some(2) }
}

/// Rules 1 and 3: AutoSave 3 s after the last edit, clean once `saved`, as of the revision sent.
#[test]
fn autosave_sends_three_seconds_after_the_last_edit_and_saved_cleans_the_document() {
    let (mut app, fake) = app();
    let id = open(&mut app, "d1");
    assert_eq!(fake.take_calls(), ["opened(d1, true, None)"]);
    assert_eq!(app.session.get(id).unwrap().name, "Hợp đồng.pdf", "the title is the display name");
    tick(&mut app, &fake, 0.0);
    edit(&mut app, id, "Ada");
    tick(&mut app, &fake, 1000.0);
    assert_eq!(fake.take_calls(), ["dirty(d1, true)"]);
    tick(&mut app, &fake, 3999.0);
    assert!(fake.saves().is_empty());
    tick(&mut app, &fake, 4000.0);
    assert_eq!(fake.saves(), [("d1".to_string(), generation(&app, id), SaveReason::Auto)]);
    assert_eq!(app.save_state(id), Some(crate::SaveState::Saving));
    assert!(dirty(&app, id), "unsaved until the shell answers");
    fake.answer(saved());
    tick(&mut app, &fake, 4100.0);
    assert!(!dirty(&app, id));
    assert_eq!(app.save_state(id), None);
    assert_eq!(app.host_version(id), Some(2));
    assert_eq!(fake.take_calls(), ["dirty(d1, false)"]);
    tick(&mut app, &fake, 60_000.0);
    assert_eq!(fake.saves().len(), 1, "nothing new, nothing sent");
}

/// Rules 1 and 2: an edit made while a save is in flight stays unsaved, later edits coalesce
/// into one follow-up save of the newest revision.
#[test]
fn edits_made_while_a_save_is_in_flight_stay_dirty_and_coalesce_into_one_follow_up() {
    let (mut app, fake) = app();
    let id = open(&mut app, "d1");
    edit(&mut app, id, "Ada");
    tick(&mut app, &fake, 0.0);
    tick(&mut app, &fake, 3000.0);
    let first = fake.saves()[0].1;
    edit(&mut app, id, "Grace");
    tick(&mut app, &fake, 3500.0);
    edit(&mut app, id, "Hopper");
    tick(&mut app, &fake, 4000.0);
    tick(&mut app, &fake, 9000.0);
    assert_eq!(fake.saves().len(), 1, "one save in flight per document");
    fake.answer(saved());
    tick(&mut app, &fake, 9100.0);
    assert!(dirty(&app, id), "Grace and Hopper were not in the saved revision");
    let saves = fake.saves();
    assert_eq!(saves.len(), 2, "one follow-up: {saves:?}");
    assert_ne!(saves[1].1, first);
    assert_eq!(saves[1].1, generation(&app, id), "the newest revision");
    fake.answer(saved());
    tick(&mut app, &fake, 9200.0);
    assert!(!dirty(&app, id));
}

/// Rule 4: a conflict stops AutoSave, keeps the document dirty and is never retried; flush skips it.
#[test]
fn a_conflict_stops_autosave_and_is_never_retried() {
    let (mut app, fake) = app();
    let id = open(&mut app, "d1");
    edit(&mut app, id, "Ada");
    tick(&mut app, &fake, 0.0);
    tick(&mut app, &fake, 3000.0);
    fake.answer(SaveResult::Conflict { latest_version: Some(7) });
    tick(&mut app, &fake, 3100.0);
    assert!(dirty(&app, id));
    assert!(app.toast.is_none(), "the shell shows the conflict");
    edit(&mut app, id, "Grace");
    for now in [4000.0, 8000.0, 40_000.0] {
        tick(&mut app, &fake, now);
    }
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();
    app.online.as_ref().unwrap().flush_handle().flush(Box::new(move || flag.set(true)));
    assert!(done.get(), "nothing to wait for");
    assert_eq!(fake.saves().len(), 1, "no retry, by AutoSave or flush");
    assert!(dirty(&app, id));
}

/// Rule 5: offline and retryable failures keep the document dirty and AutoSave running; the same
/// revision is not sent again (the shell retries it), a newer one is.
#[test]
fn offline_and_retryable_failures_keep_autosave_running() {
    for answer in [SaveResult::Offline, SaveResult::Failed { retryable: true, message: "Máy chủ bận".into() }] {
        let (mut app, fake) = app();
        let id = open(&mut app, "d1");
        edit(&mut app, id, "Ada");
        tick(&mut app, &fake, 0.0);
        tick(&mut app, &fake, 3000.0);
        fake.answer(answer.clone());
        tick(&mut app, &fake, 3100.0);
        assert!(dirty(&app, id), "{answer:?}");
        assert!(app.toast.is_none() && app.save_state(id).is_none(), "the shell shows the state: {answer:?}");
        tick(&mut app, &fake, 20_000.0);
        assert_eq!(fake.saves().len(), 1, "the shell keeps and retries that revision");
        edit(&mut app, id, "Grace");
        tick(&mut app, &fake, 20_000.0);
        tick(&mut app, &fake, 23_000.0);
        let saves = fake.saves();
        assert_eq!(saves.len(), 2, "{answer:?}");
        assert_eq!(saves[1], ("d1".to_string(), generation(&app, id), SaveReason::Auto));
    }
}

/// Rule 6: a failure that won't go away stops AutoSave and shows the shell's message where save
/// errors show (the notice and the tab's "Save failed").
#[test]
fn a_non_retryable_failure_stops_autosave_and_shows_the_message() {
    let (mut app, fake) = app();
    let id = open(&mut app, "d1");
    edit(&mut app, id, "Ada");
    tick(&mut app, &fake, 0.0);
    tick(&mut app, &fake, 3000.0);
    let message = "Tệp quá lớn để lưu lên máy chủ.";
    fake.answer(SaveResult::Failed { retryable: false, message: message.into() });
    tick(&mut app, &fake, 3100.0);
    assert!(dirty(&app, id));
    assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some(message));
    assert_eq!(app.save_state(id), Some(crate::SaveState::Failed(message.into())));
    edit(&mut app, id, "Grace");
    tick(&mut app, &fake, 4000.0);
    tick(&mut app, &fake, 40_000.0);
    assert_eq!(fake.saves().len(), 1, "AutoSave stopped");
    // Save is the user's own: it still goes, and a success resumes AutoSave.
    app.save_active(SaveTarget::InPlace);
    assert_eq!(fake.saves()[1].2, SaveReason::Manual);
    fake.answer(saved());
    tick(&mut app, &fake, 41_000.0);
    assert!(!dirty(&app, id));
    edit(&mut app, id, "Hopper");
    tick(&mut app, &fake, 41_000.0);
    tick(&mut app, &fake, 44_000.0);
    assert_eq!(fake.saves()[2].2, SaveReason::Auto);
}

/// `flush`: every document with unsaved changes is handed over, clean ones are not, and it settles
/// then, without waiting for the answers (the shell tracks those).
#[test]
fn flush_hands_every_changed_document_over_and_settles_without_waiting_for_answers() {
    let (mut app, fake) = app();
    let a = open(&mut app, "a");
    let b = open(&mut app, "b");
    let _clean = open(&mut app, "c");
    edit(&mut app, a, "Ada");
    edit(&mut app, b, "Grace");
    tick(&mut app, &fake, 100.0);
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();
    app.online.as_ref().unwrap().flush_handle().flush(Box::new(move || flag.set(true)));
    let saves = fake.saves();
    assert_eq!(saves, [("a".to_string(), generation(&app, a), SaveReason::Flush), ("b".to_string(), generation(&app, b), SaveReason::Flush)]);
    assert!(done.get(), "both handed over, neither answered");
    assert!(dirty(&app, a) && dirty(&app, b), "unsaved until the shell answers");
    fake.answer(saved());
    fake.answer(saved());
    tick(&mut app, &fake, 200.0);
    assert!(!dirty(&app, a) && !dirty(&app, b), "saved as of the flushed revision");
    // Nothing changed since: an empty flush settles at once.
    let again = Rc::new(Cell::new(false));
    let flag = again.clone();
    app.online.as_ref().unwrap().flush_handle().flush(Box::new(move || flag.set(true)));
    assert!(again.get());
    assert_eq!(fake.saves().len(), 2);
}

/// `flush` behind a save in flight: the newest revision follows it, and flush settles once that
/// one is handed over, not answered.
#[test]
fn flush_waits_behind_a_save_in_flight_and_sends_the_newest_after_it() {
    let (mut app, fake) = app();
    let id = open(&mut app, "d1");
    edit(&mut app, id, "Ada");
    tick(&mut app, &fake, 0.0);
    tick(&mut app, &fake, 3000.0);
    edit(&mut app, id, "Grace");
    tick(&mut app, &fake, 3100.0);
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();
    app.online.as_ref().unwrap().flush_handle().flush(Box::new(move || flag.set(true)));
    assert_eq!(fake.saves().len(), 1, "still one in flight");
    assert!(!done.get(), "the newest revision is not handed over yet");
    fake.answer(saved());
    assert_eq!(fake.saves()[1], ("d1".to_string(), generation(&app, id), SaveReason::Flush), "sent as soon as the first was answered");
    assert!(done.get(), "handed over; its answer is the shell's");
    fake.answer(saved());
    tick(&mut app, &fake, 3200.0);
    assert!(!dirty(&app, id));
}

/// A follow-up waiting behind a save that conflicts is dropped: nothing is retried.
#[test]
fn a_flush_follow_up_is_dropped_when_the_save_before_it_conflicts() {
    let (mut app, fake) = app();
    let id = open(&mut app, "d1");
    edit(&mut app, id, "Ada");
    tick(&mut app, &fake, 0.0);
    tick(&mut app, &fake, 3000.0);
    edit(&mut app, id, "Grace");
    tick(&mut app, &fake, 3100.0);
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();
    app.online.as_ref().unwrap().flush_handle().flush(Box::new(move || flag.set(true)));
    fake.answer(SaveResult::Conflict { latest_version: None });
    assert!(done.get());
    assert_eq!(fake.saves().len(), 1);
    assert_eq!(fake.waiting(), 0);
}

fn acknowledge(app: &PdfCraftApp, doc_id: &str, revision: u64, version: f64) {
    let ack = Acknowledged::new(doc_id.into(), revision as f64, Some(version)).unwrap();
    app.online.as_ref().unwrap().acknowledgements().lock().unwrap().push(ack);
}

/// An AutoSave of the current revision, answered `offline`; returns that revision.
fn autosave_offline(app: &mut PdfCraftApp, fake: &Fake, id: DocId, value: &str, now: f64) -> u64 {
    edit(app, id, value);
    tick(app, fake, now);
    tick(app, fake, now + 3000.0);
    let revision = fake.saves().last().unwrap().1;
    assert_eq!(revision, generation(app, id));
    fake.answer(SaveResult::Offline);
    tick(app, fake, now + 3100.0);
    revision
}

/// `acknowledged` after `offline`: the shell's own retry stored that revision, so it counts as
/// `saved` (rule 1); a document the app doesn't hold is ignored.
#[test]
fn acknowledged_after_offline_saves_the_document() {
    let (mut app, fake) = app();
    let id = open(&mut app, "d1");
    let revision = autosave_offline(&mut app, &fake, id, "Ada", 0.0);
    assert!(dirty(&app, id));
    fake.take_calls();
    acknowledge(&app, "unknown", revision, 9.0);
    tick(&mut app, &fake, 3200.0);
    assert!(dirty(&app, id), "not this document");
    acknowledge(&app, "d1", revision, 5.0);
    tick(&mut app, &fake, 3300.0);
    assert!(!dirty(&app, id));
    assert_eq!(app.host_version(id), Some(5));
    assert_eq!(fake.take_calls(), ["dirty(d1, false)"]);
    tick(&mut app, &fake, 60_000.0);
    assert_eq!(fake.saves().len(), 1, "nothing left to send");
}

/// Rule 1 for `acknowledged`: edits made after the revision sent stay unsaved, and an
/// acknowledgement older than the last one changes nothing.
#[test]
fn an_older_acknowledged_is_ignored_and_later_edits_stay_dirty() {
    let (mut app, fake) = app();
    let id = open(&mut app, "d1");
    let first = autosave_offline(&mut app, &fake, id, "Ada", 0.0);
    edit(&mut app, id, "Grace");
    acknowledge(&app, "d1", first, 2.0);
    tick(&mut app, &fake, 3200.0);
    assert!(dirty(&app, id), "Grace came after the acknowledged revision");
    assert_eq!(app.host_version(id), Some(2));
    let second = autosave_offline(&mut app, &fake, id, "Hopper", 3200.0);
    acknowledge(&app, "d1", second, 3.0);
    acknowledge(&app, "d1", first, 2.0);
    tick(&mut app, &fake, 7000.0);
    assert!(!dirty(&app, id));
    assert_eq!(app.host_version(id), Some(3), "the older acknowledgement is ignored");
}

/// `flush` hands over again a revision left to the shell's retry: only an acknowledged revision
/// counts as stored.
#[test]
fn flush_hands_over_a_revision_answered_offline() {
    let (mut app, fake) = app();
    let id = open(&mut app, "d1");
    let revision = autosave_offline(&mut app, &fake, id, "Ada", 0.0);
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();
    app.online.as_ref().unwrap().flush_handle().flush(Box::new(move || flag.set(true)));
    assert!(done.get());
    assert_eq!(fake.saves()[1], ("d1".to_string(), revision, SaveReason::Flush));
}

/// Rule 10: the page's own reload warning counts local documents only; the shell guards its own.
#[test]
fn the_reload_warning_counts_local_documents_only() {
    use std::sync::atomic::Ordering;
    let ctx = egui::Context::default();
    let (mut app, _fake) = app();
    let host = open(&mut app, "d1");
    app.open_bytes("Local.pdf", None, FORM.to_vec()).unwrap();
    let local = app.views[1].id;
    edit(&mut app, host, "Ada");
    app.guard_quit(&ctx);
    assert!(!app.unsaved_flag.load(Ordering::Relaxed), "a host document alone never warns");
    edit(&mut app, local, "Grace");
    app.guard_quit(&ctx);
    assert!(app.unsaved_flag.load(Ordering::Relaxed));
}

/// Rules 3 and 9: Save goes to the shell (reason `manual`, never a picker), after a save in flight.
#[test]
fn save_goes_to_the_shell_after_a_save_in_flight() {
    let (mut app, fake) = app();
    let id = open(&mut app, "d1");
    edit(&mut app, id, "Ada");
    tick(&mut app, &fake, 0.0);
    tick(&mut app, &fake, 3000.0);
    edit(&mut app, id, "Grace");
    assert!(!app.save_active(SaveTarget::InPlace));
    assert_eq!(fake.saves().len(), 1, "serialised behind the AutoSave in flight");
    assert_eq!(app.save_state(id), Some(crate::SaveState::Saving));
    fake.answer(saved());
    tick(&mut app, &fake, 3100.0);
    let saves = fake.saves();
    assert_eq!(saves[1], ("d1".to_string(), generation(&app, id), SaveReason::Manual));
    fake.answer(saved());
    tick(&mut app, &fake, 3200.0);
    assert!(!dirty(&app, id));
    assert!(app.save_active(SaveTarget::InPlace), "nothing to send");
    assert_eq!(fake.saves().len(), 2);
}

/// A save prompt's Save closes the tab once the shell has saved it, and the shell hears it closed.
#[test]
fn a_close_prompt_save_closes_once_the_shell_saved() {
    let (mut app, fake) = app();
    let id = open(&mut app, "d1");
    edit(&mut app, id, "Ada");
    tick(&mut app, &fake, 0.0);
    fake.take_calls();
    let ctx = egui::Context::default();
    app.request_close_tab(0);
    app.resolve_close(&ctx, Some(true));
    assert_eq!(fake.saves()[0].2, SaveReason::Manual);
    assert_eq!(app.views.len(), 1, "open until saved");
    fake.answer(saved());
    tick(&mut app, &fake, 100.0);
    assert!(app.views.is_empty());
    assert_eq!(fake.take_calls(), ["closed(d1)"]);
    assert!(!app.can_reopen_closed(), "a server document is reopened by the shell, not as a local copy");
}

/// A signature is never invalidated without asking: no AutoSave and no flush of a signed document;
/// Save asks first, and only the confirmed Save is sent.
#[test]
fn a_signed_document_is_saved_only_by_a_confirmed_save() {
    let (mut app, fake) = app();
    deliver(&app, "d1", "pdf", signed());
    app.open_host_documents();
    let id = app.views[0].id;
    assert!(app.session.get(id).unwrap().is_signed());
    edit(&mut app, id, "Ada");
    tick(&mut app, &fake, 0.0);
    let notice = app.toast.take().map(|t| t.0).unwrap_or_default();
    assert!(notice.starts_with("AutoSave is off"), "{notice}");
    for now in [3000.0, 31_000.0, 120_000.0] {
        tick(&mut app, &fake, now);
    }
    assert!(app.toast.is_none(), "said once");
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();
    app.online.as_ref().unwrap().flush_handle().flush(Box::new(move || flag.set(true)));
    assert!(done.get());
    assert!(fake.saves().is_empty(), "neither AutoSave nor flush sends it");
    app.save_active(SaveTarget::InPlace);
    assert!(app.held_save.is_some(), "Save asks first");
    assert!(fake.saves().is_empty());
    app.confirm_held_save();
    assert_eq!(fake.saves(), [("d1".to_string(), generation(&app, id), SaveReason::Manual)]);
    // What the user confirmed may be flushed; it is already on its way.
    tick(&mut app, &fake, 121_000.0);
    app.online.as_ref().unwrap().flush_handle().flush(Box::new(|| {}));
    assert_eq!(fake.saves().len(), 1);
    fake.answer(saved());
    tick(&mut app, &fake, 121_100.0);
    assert!(!dirty(&app, id));
    edit(&mut app, id, "Grace");
    tick(&mut app, &fake, 121_200.0);
    tick(&mut app, &fake, 200_000.0);
    assert_eq!(fake.saves().len(), 1, "AutoSave stays off");
}

/// Rule 8 and local files: documents from the device keep the drafts store and saving as before;
/// host documents never reach the store.
#[test]
fn local_documents_keep_drafts_and_host_documents_never_reach_them() {
    use crate::drafts::{DraftInbox, DraftSink, Drafts};

    #[derive(Clone, Default)]
    struct Store(Rc<RefCell<(f64, Vec<String>)>>);
    impl DraftSink for Store {
        fn available(&self) -> bool {
            true
        }
        fn ready(&self) -> Result<(), String> {
            Ok(())
        }
        fn put(&self, key: &str, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
            self.0.borrow_mut().1.push(key.to_string());
            Ok(())
        }
        fn remove(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn now_ms(&self) -> f64 {
            self.0.borrow().0
        }
    }

    let (mut app, fake) = app();
    let store = Store::default();
    app.drafts = Some(Drafts::new(Box::new(store.clone()), DraftInbox::default()));
    let host = open(&mut app, "d1");
    app.open_bytes("Local.pdf", None, FORM.to_vec()).unwrap();
    let local = app.views[1].id;
    assert!(app.is_host_doc(host) && !app.is_host_doc(local));
    edit(&mut app, host, "Ada");
    edit(&mut app, local, "Grace");
    app.drafts_step();
    tick(&mut app, &fake, 0.0);
    store.0.borrow_mut().0 = 60_000.0;
    app.drafts_step();
    tick(&mut app, &fake, 60_000.0);
    assert_eq!(store.0.borrow().1, [format!("doc-{}", local.0)], "only the local document is drafted");
    assert!(fake.saves().iter().all(|s| s.0 == "d1"), "the local document never goes to the shell");
}

/// `opened` once per `open`: a refused format (`unsupported`) or an unreadable file (`parserFailed`,
/// keys of the shell's intake messages) is `ok` false, a password prompt `ok` true, and a cancelled
/// prompt leaves nothing open (`closed`).
#[test]
fn opened_reports_each_open_and_a_password_prompt_counts_as_opened() {
    let (mut app, fake) = app();
    deliver(&app, "x", "docx", FORM.to_vec());
    deliver(&app, "y", "pdf", b"not a pdf".to_vec());
    deliver(&app, "z", "PDF", encrypted(FORM, "pw"));
    deliver(&app, "w", "pdf", encrypted(FORM, "pw"));
    app.open_host_documents();
    assert_eq!(fake.take_calls(), ["opened(x, false, Some(\"unsupported\"))", "opened(y, false, Some(\"parserFailed\"))", "opened(z, true, None)"]);
    assert!(app.toast.as_ref().is_some_and(|t| t.0.starts_with("Couldn't open Hợp đồng.pdf")), "said as for any file");
    app.submit_password(Some("wrong".into()));
    app.submit_password(Some("pw".into()));
    let id = app.views[0].id;
    assert!(app.is_host_doc(id), "unlocked, it is the server's document");
    app.open_host_documents();
    assert_eq!(fake.take_calls(), ["opened(w, true, None)"]);
    app.submit_password(None);
    assert_eq!(fake.take_calls(), ["closed(w)"]);
    assert_eq!(app.views.len(), 1);
}

/// A reload of an open document (after a conflict) takes its tab's place without `closed`, and an
/// answer to the earlier opening's save never touches it.
#[test]
fn a_reload_replaces_the_tab_and_ignores_the_earlier_openings_answers() {
    let (mut app, fake) = app();
    let first = open(&mut app, "d1");
    app.open_bytes("Local.pdf", None, FORM.to_vec()).unwrap();
    edit(&mut app, first, "Ada");
    tick(&mut app, &fake, 0.0);
    tick(&mut app, &fake, 3000.0);
    fake.take_calls();
    let second = open(&mut app, "d1");
    assert_eq!(app.views.len(), 2);
    assert_eq!(app.views[0].id, second, "in the earlier tab's place");
    assert_eq!(app.active, Some(0));
    assert!(app.session.get(first).is_none());
    fake.answer(saved());
    tick(&mut app, &fake, 3100.0);
    assert!(!dirty(&app, second));
    assert_eq!(fake.take_calls(), ["opened(d1, true, None)", "dirty(d1, false)"], "no closed for the reload");
    assert_eq!(app.host_version(second), Some(1), "the earlier opening's answer is not this one's");
    app.close_tab(0);
    assert_eq!(fake.take_calls(), ["closed(d1)"]);
}

/// Save As of a host document writes a local copy; the document stays the server's, unsaved.
#[test]
fn save_as_writes_a_local_copy_and_keeps_the_document_unsaved() {
    use crate::web_save::{BrowserSave, FileId, SaveDone, SaveOutcome, WebSaves};

    struct Picker(Rc<Cell<usize>>);
    impl BrowserSave for Picker {
        fn has_picker(&self) -> bool {
            true
        }
        fn pick_and_write(&self, _: &str, _: &[u8], done: SaveDone) {
            self.0.set(self.0.get() + 1);
            done(SaveOutcome::Saved { name: "Bản sao.pdf".into(), file: Some(1) });
        }
        fn write_to(&self, _: FileId, _: &str, _: &[u8], _: SaveDone) {
            panic!("Save never writes a host document locally");
        }
        fn download(&self, _: &str, _: &[u8]) -> Result<(), String> {
            Ok(())
        }
    }

    let (mut app, fake) = app();
    let picks = Rc::new(Cell::new(0));
    app.web_save = Some(WebSaves::new(Box::new(Picker(picks.clone()))));
    let id = open(&mut app, "d1");
    edit(&mut app, id, "Ada");
    app.save_active(SaveTarget::As);
    app.settle_web_saves();
    assert_eq!(picks.get(), 1);
    assert!(dirty(&app, id), "the server has not got it");
    assert_eq!(app.session.get(id).unwrap().name, "Hợp đồng.pdf");
    assert!(fake.saves().is_empty());
    app.save_active(SaveTarget::InPlace);
    assert_eq!(fake.saves()[0].2, SaveReason::Manual, "Save still goes to the shell");
    assert_eq!(picks.get(), 1);
}

/// Scripts stay off for a host document until the user allows them, as for any file.
#[test]
fn a_host_documents_scripts_stay_off_until_allowed() {
    let (mut app, _fake) = app();
    deliver(&app, "d1", "pdf", scripted());
    app.open_host_documents();
    let id = app.views[0].id;
    assert!(app.session.has_scripts(id));
    assert!(!app.session.scripts_allowed(id));
}

/// Without `erpHost` (the desktop, or the web without the shell) nothing of this runs.
#[test]
fn without_erp_host_saving_is_unchanged() {
    let mut app = PdfCraftApp::new();
    assert!(app.online.is_none());
    app.open_bytes("Local.pdf", None, FORM.to_vec()).unwrap();
    let id = app.views[0].id;
    edit(&mut app, id, "Ada");
    assert_eq!(app.online_step(), None);
    assert!(!app.is_host_doc(id));
    let dir = std::env::temp_dir().join(format!("pdfcraft-online-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("Local.pdf");
    app.save_override = Some(dest.to_string_lossy().into_owned());
    assert!(app.save_active(SaveTarget::InPlace), "written at once, as before");
    assert!(!dirty(&app, id));
    assert!(std::fs::read(&dest).unwrap().starts_with(b"%PDF"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_shells_answers_are_read_leniently() {
    assert_eq!(SaveResult::from_fields(Some("saved"), Some(3.0), None, None, None), SaveResult::Saved { version: Some(3) });
    assert_eq!(SaveResult::from_fields(Some("saved"), Some(f64::NAN), None, None, None), SaveResult::Saved { version: None });
    assert_eq!(SaveResult::from_fields(Some("conflict"), None, Some(9.0), None, None), SaveResult::Conflict { latest_version: Some(9) });
    assert_eq!(
        SaveResult::from_fields(Some("failed"), None, None, None, Some("Lỗi".into())),
        SaveResult::Failed { retryable: false, message: "Lỗi".into() }
    );
    assert_eq!(SaveResult::from_fields(Some("offline"), None, None, None, None), SaveResult::Offline);
    assert_eq!(SaveResult::from_fields(None, None, None, None, None), SaveResult::Offline, "unknown: kept unsaved");
    assert_eq!(SaveResult::from_fields(Some("saved"), Some(-1.0), None, None, None), SaveResult::Saved { version: None });
    assert_eq!(HostDocument::new("d".into(), Some(2.5), String::new(), "pdf".into(), vec![]).version, None);
}

/// form.pdf with an approval signature, so any change is a change after signing.
fn signed() -> Vec<u8> {
    let p12 = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../sign/tests/data/ec-p256.p12")).unwrap();
    let digital_id = pdfcraft_engine::sign::pkcs12::open(&p12, "test").unwrap();
    let mut s = pdfcraft_engine::Session::new().with_clock(|| 1_800_000_000);
    let doc = s.open("signed.pdf", None, Arc::new(FORM.to_vec()), None).unwrap();
    let opts = pdfcraft_engine::SignOptions { page: 0, rect: Some([20.0, 20.0, 140.0, 60.0]), ..Default::default() };
    s.sign(doc, &digital_id, opts).unwrap().as_ref().clone()
}

/// `bytes` protected with an open password (AES-256).
fn encrypted(bytes: &[u8], password: &str) -> Vec<u8> {
    let mut doc = pdfcraft_cos::Document::open(Arc::new(bytes.to_vec())).unwrap();
    doc.set_encryption(&pdfcraft_cos::NewEncryption {
        algorithm: pdfcraft_cos::Algorithm::Aes256,
        user_password: password,
        owner_password: "owner",
        permissions: -1,
        encrypt_metadata: true,
        seed: [2; 32],
    })
    .unwrap();
    pdfcraft_cos::write_full(&doc, &Default::default()).unwrap()
}

/// One page with a push button whose Mouse Up script fills in a field.
fn scripted() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R 5 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 6 0 R >> >> >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 300] >>",
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R] >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (greeting) /Rect [20 250 280 270] /P 3 0 R /F 4 >>",
        "<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 65536 /T (hello) /Rect [20 200 120 230] /P 3 0 R /F 4 /A << /S /JavaScript /JS (getField\\('greeting'\\).value = 'Hi';) >> >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}
