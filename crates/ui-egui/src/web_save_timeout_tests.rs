use std::cell::Cell;

use super::tests::{app, dirty, edit};
use super::*;

fn clock(app: &mut PdfCraftApp) -> Rc<Cell<f64>> {
    let now = Rc::new(Cell::new(100.0));
    let read = now.clone();
    app.web_save.as_mut().unwrap().clock = Rc::new(move || read.get());
    now
}

fn saved() -> SaveOutcome {
    SaveOutcome::Saved { name: "Late.pdf".into(), file: Some(7) }
}

#[test]
fn timeout_at_sixty_seconds_keeps_unsaved_and_offers_pending_bytes() {
    let (mut app, fake, id) = app(true);
    let now = clock(&mut app);
    edit(&mut app, "Ada");
    app.save_active(SaveTarget::InPlace);
    let pending_bytes = app.web_save.as_ref().unwrap().pending.first().unwrap().bytes.clone();
    now.set(159.999);
    app.settle_web_saves();
    assert_eq!(app.save_state(id), Some(crate::SaveState::Saving));
    now.set(160.0);
    assert!(app.settle_web_saves().is_empty());
    assert!(matches!(app.save_state(id), Some(crate::SaveState::Failed(_))));
    assert!(dirty(&app, id));
    let saves = app.web_save.as_ref().unwrap();
    assert!(!saves.is_pending(id));
    assert_eq!(saves.timed_out.first().unwrap().bytes, pending_bytes);
    app.download_timed_out_save(id);
    assert_eq!(fake.calls(), r#"[Pick("Report.pdf"), Download("Report.pdf")]"#);
    assert_eq!(fake.downloads(), vec![pending_bytes.as_ref().clone()]);
    assert!(app.web_save.as_ref().unwrap().timed_out.is_empty());
    assert!(dirty(&app, id));
    assert!(matches!(app.save_state(id), Some(crate::SaveState::Failed(_))));
}

#[test]
fn completion_before_deadline_wins_even_when_polled_after_deadline() {
    let (mut app, fake, id) = app(true);
    let now = clock(&mut app);
    edit(&mut app, "Ada");
    app.save_active(SaveTarget::InPlace);
    now.set(159.999);
    fake.answer(saved());
    now.set(170.0);
    assert_eq!(app.settle_web_saves().len(), 1);
    assert!(!dirty(&app, id));
    assert!(app.web_save.as_ref().unwrap().timed_out.is_empty());
    assert_eq!(app.save_state(id), None);
}

#[test]
fn late_completion_cannot_mark_saved_rename_or_close_the_document() {
    let (mut app, fake, id) = app(true);
    let now = clock(&mut app);
    edit(&mut app, "Ada");
    app.request_close_tab(0);
    app.resolve_close(&egui::Context::default(), Some(true));
    now.set(160.0);
    app.settle_web_saves();
    now.set(161.0);
    fake.answer(saved());
    assert!(app.settle_web_saves().is_empty());
    assert_eq!(app.views.len(), 1);
    assert!(dirty(&app, id));
    assert_eq!(app.session.get(id).unwrap().name, "Report.pdf");
    assert!(matches!(app.save_state(id), Some(crate::SaveState::Failed(_))));
    assert_eq!(fake.calls(), r#"[Pick("Report.pdf")]"#);
}

#[test]
fn completion_at_deadline_without_an_intervening_poll_times_out() {
    let (mut app, fake, id) = app(true);
    let now = clock(&mut app);
    edit(&mut app, "Ada");
    app.save_active(SaveTarget::InPlace);
    now.set(160.0);
    fake.answer(saved());
    assert!(app.settle_web_saves().is_empty());
    assert!(dirty(&app, id));
    assert!(matches!(app.save_state(id), Some(crate::SaveState::Failed(_))));
    assert_eq!(app.web_save.as_ref().unwrap().timed_out.len(), 1);
}

#[test]
fn retries_download_instead_of_writing_to_an_uncertain_destination() {
    let (mut app, fake, id) = app(true);
    let now = clock(&mut app);
    edit(&mut app, "Ada");
    app.save_active(SaveTarget::InPlace);
    fake.answer(saved());
    app.settle_web_saves();
    edit(&mut app, "Grace");
    app.save_active(SaveTarget::InPlace);
    now.set(160.0);
    app.settle_web_saves();
    assert!(dirty(&app, id));
    assert!(app.save_active(SaveTarget::InPlace));
    fake.answer(saved());
    assert!(app.settle_web_saves().is_empty());
    assert_eq!(app.session.get(id).unwrap().name, "Late.pdf");
    edit(&mut app, "Lin");
    assert!(app.save_active(SaveTarget::As));
    assert_eq!(fake.calls(), r#"[Pick("Report.pdf"), WriteTo(7), Download("Late.pdf"), Download("Late.pdf")]"#);
}

#[test]
fn closing_a_timed_out_document_discards_its_download_offer() {
    let (mut app, fake, id) = app(true);
    let now = clock(&mut app);
    edit(&mut app, "Ada");
    app.save_active(SaveTarget::InPlace);
    now.set(160.0);
    app.settle_web_saves();
    app.close_tab(0);
    fake.answer(saved());
    assert!(app.settle_web_saves().is_empty());
    let saves = app.web_save.as_ref().unwrap();
    assert!(saves.timed_out.is_empty());
    assert!(!saves.destinations.contains_key(&id));
}

#[test]
fn failed_fallback_download_keeps_the_offer_and_unsaved_work() {
    let (mut app, fake, id) = app(true);
    let now = clock(&mut app);
    edit(&mut app, "Ada");
    app.save_active(SaveTarget::InPlace);
    now.set(160.0);
    app.settle_web_saves();
    fake.fail_download();
    app.download_timed_out_save(id);
    assert!(dirty(&app, id));
    assert_eq!(app.web_save.as_ref().unwrap().timed_out.len(), 1);
    assert!(matches!(app.save_state(id), Some(crate::SaveState::Failed(_))));
}

#[test]
fn timeout_download_button_works_at_desktop_and_phone_widths() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    for size in [egui::vec2(1366.0, 768.0), egui::vec2(390.0, 844.0)] {
        let (mut app, fake, id) = app(true);
        let now = clock(&mut app);
        app.set_option("language", "vi").unwrap();
        edit(&mut app, "Ada");
        app.save_active(SaveTarget::InPlace);
        now.set(160.0);
        app.settle_web_saves();
        let mut h = Harness::builder().with_size(size).build_eframe(move |_cc| app);
        h.run_steps(4);
        h.get_by_label("Tải xuống").click();
        h.run_steps(2);
        assert_eq!(fake.calls(), r#"[Pick("Report.pdf"), Download("Report.pdf")]"#);
        assert!(dirty(h.state(), id));
        assert!(h.state().web_save.as_ref().unwrap().timed_out.is_empty());
    }
}

#[test]
fn timeout_labels_are_localized_in_every_catalog() {
    use crate::i18n::{LANGUAGES, Lang, tr};

    let vi = Lang::from_code("vi").unwrap();
    assert_eq!(tr(vi, "Save failed"), "Lưu không thành công");
    assert_eq!(tr(vi, "Download"), "Tải xuống");
    for info in &LANGUAGES {
        if info.code == "en" {
            continue;
        }
        let lang = Lang::from_code(info.code).unwrap();
        for key in ["Save failed", "Download"] {
            assert_ne!(tr(lang, key), key, "{}: {key}", info.code);
        }
    }
}
