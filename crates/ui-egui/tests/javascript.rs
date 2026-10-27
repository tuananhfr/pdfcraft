//! Acrobat JavaScript in the shell (egui_kittest): the console, Document JavaScripts, a button
//! script and Preferences ▸ JavaScript.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{Dialog, PdfCraftApp};

/// A form with a text field and a push button whose Mouse Up script fills it in.
fn form() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R 5 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 6 0 R >> >> >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 300] >>",
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R] >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (greeting) /Rect [20 250 280 270] /P 3 0 R /F 4 >>",
        "<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 65536 /T (hello) /Rect [20 200 120 230] /P 3 0 R /F 4 /A << /S /JavaScript /JS (getField\\('greeting'\\).value = 'Hello from JavaScript'; app.alert\\('Filled in'\\);) >> >>",
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

fn harness_with(allowed: bool) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("form.pdf", None, form()).unwrap();
        app
    });
    h.run_steps(4);
    if allowed {
        let id = h.state().active_ids().unwrap().1;
        h.state_mut().allow_scripts(id);
        h.run_steps(2);
    }
    h
}

/// The form, its scripts allowed (they are not by default).
fn harness() -> Harness<'static, PdfCraftApp> {
    harness_with(true)
}

fn value(h: &Harness<'static, PdfCraftApp>, name: &str) -> Vec<String> {
    let app = h.state();
    let id = app.active_ids().unwrap().1;
    app.session.get(id).unwrap().form.iter().find(|f| f.name == name).unwrap().value.clone()
}

#[test]
fn console_runs_scripts_against_the_document() {
    let mut h = harness();
    assert!(h.state_mut().execute("tools.js_console"));
    h.run_steps(2);
    h.get_by_label("JavaScript Console");
    h.state_mut().js_console.input = "getField('greeting').value = 'typed'; console.println(numFields); 6 * 7".into();
    h.get_by_label("Run").click();
    h.run_steps(2);
    assert_eq!(value(&h, "greeting"), ["typed"]);
    let log = h.state().js_console.log.clone();
    assert!(log.contains(&"2".to_string()) && log.contains(&"42".to_string()), "{log:?}");
    h.state_mut().js_console.input = "throw new Error('boom')".into();
    h.run_steps(4);
    h.get_by_label("Run").click();
    h.run_steps(2);
    assert!(h.state().js_console.log.last().unwrap().contains("boom"));
    h.run_steps(4);
    h.get_by_label("Clear").click();
    h.run_steps(2);
    assert!(h.state().js_console.log.is_empty(), "{:?} {:?}", h.state().js_console.log, h.state().dialog);
}

#[test]
fn button_scripts_run_and_alert() {
    let mut h = harness();
    let id = h.state().active_ids().unwrap().1;
    h.state_mut().run_button_script(id, "hello", "getField('greeting').value = 'Hello from JavaScript'; app.alert('Filled in');");
    h.run_steps(2);
    assert_eq!(value(&h, "greeting"), ["Hello from JavaScript"]);
    assert_eq!(h.state().toast.clone().unwrap().0, "Filled in");
}

#[test]
fn document_scripts_and_preferences() {
    let mut h = harness();
    assert!(h.state_mut().execute("tools.document_js"));
    h.run_steps(2);
    h.get_by_label("Document JavaScripts");
    h.state_mut().doc_js.name = "init".into();
    h.state_mut().doc_js.script = "function hi() { return 'hi'; }".into();
    h.get_by_label("Save").click();
    h.run_steps(2);
    let id = h.state().active_ids().unwrap().1;
    assert_eq!(h.state().session.get(id).unwrap().document_scripts()[0].0, "init");
    h.state_mut().dialog = Some(Dialog::Preferences);
    h.run_steps(2);
    // Below the fold in a 900-point window: the preferences scroll to it.
    let always = "Always allow JavaScript in PDFs (don't ask)";
    h.get_by_label(always).scroll_to_me();
    h.run_steps(4);
    assert!(!h.state().session.javascript(), "off by default");
    h.get_by_label(always).click();
    h.run_steps(2);
    assert!(h.state().session.javascript());
    let saved = h.state().persist();
    let mut fresh = PdfCraftApp::new();
    fresh.set_option("language", "en").unwrap();
    fresh.restore(&saved);
    assert!(fresh.session.javascript(), "the preference is remembered");
}

#[test]
fn javascript_stored_under_the_old_key_does_not_turn_always_allow_on() {
    let mut saved: serde_json::Value = serde_json::from_str(&PdfCraftApp::new().persist()).unwrap();
    let settings = saved.as_object_mut().unwrap();
    assert!(settings.remove("javascript_always").is_some(), "the preference is stored under its new key");
    // Enable Acrobat JavaScript was on by default, so a stored true says nothing about consent.
    settings.insert("javascript".into(), serde_json::Value::Bool(true));
    let mut fresh = PdfCraftApp::new();
    fresh.restore(&saved.to_string());
    assert!(!fresh.session.javascript());
}

const BAR: &str = "This PDF contains JavaScript. Its scripts have not been run.";

#[test]
fn a_document_with_scripts_says_so_until_they_are_allowed() {
    let mut h = harness_with(false);
    let id = h.state().active_ids().unwrap().1;
    assert!(!h.state().session.scripts_allowed(id));
    h.get_by_label(BAR);
    h.get_by_label("Allow for this file").click();
    h.run_steps(3);
    assert!(h.state().session.scripts_allowed(id));
    assert!(h.query_by_label(BAR).is_none(), "the bar goes once they are allowed");
    h.state_mut().run_button_script(id, "hello", "getField('greeting').value = 'Hello from JavaScript';");
    h.run_steps(2);
    assert_eq!(value(&h, "greeting"), ["Hello from JavaScript"]);
}

#[test]
fn a_blocked_button_says_so_on_the_bar_and_brings_it_back() {
    let mut h = harness_with(false);
    let id = h.state().active_ids().unwrap().1;
    // The scripts bar is drawn first, above the form notice with its own ×.
    h.query_all_by_label("Dismiss").next().expect("the scripts bar closes").click();
    h.run_steps(2);
    assert!(h.query_by_label(BAR).is_none(), "closed with ×");
    h.state_mut().run_button_script(id, "hello", "getField('greeting').value = 'Hello from JavaScript';");
    h.run_steps(3);
    assert!(value(&h, "greeting").iter().all(|v| v.is_empty()), "the script did not run");
    assert!(!h.state().toast.as_ref().is_some_and(|t| t.0.contains("not allowed")), "no error toast: the bar says it");
    h.get_by_label("This PDF contains JavaScript. Its scripts have not been run, so the button or calculation you used did nothing.");
}

#[test]
fn a_document_without_scripts_has_no_bar() {
    let blank = pdfcraft_engine::Session::new().create_blank(200.0, 200.0, 1).unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("blank.pdf", None, blank.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.query_by_label(BAR).is_none());
    assert!(h.query_by_label("Allow for this file").is_none());
}

const BLOCKED: &str = "This PDF contains JavaScript. Its scripts have not been run, so the button or calculation you used did nothing.";

#[test]
fn allowing_one_copy_leaves_a_second_copy_and_a_reopened_tab_blocked() {
    let mut h = harness_with(false);
    h.state_mut().open_bytes("form copy.pdf", None, form()).unwrap();
    h.run_steps(3);
    let (first, copy) = (h.state().views[0].id, h.state().views[1].id);
    h.state_mut().allow_scripts(first);
    h.run_steps(2);
    assert!(h.state().session.scripts_allowed(first));
    assert!(!h.state().session.scripts_allowed(copy), "the same bytes in another tab are another document");
    h.state_mut().active = Some(0);
    assert!(h.state_mut().execute("file.close"));
    h.run_steps(3);
    assert!(h.state_mut().execute("file.reopen_closed"));
    h.run_steps(4);
    let app = h.state();
    let reopened = app.views.iter().map(|v| v.id).find(|&id| id != copy).expect("the closed tab is back");
    assert_ne!(reopened, first);
    assert!(!app.session.scripts_allowed(reopened), "reopened, it asks again");
}

#[test]
fn a_console_blocked_on_a_document_without_scripts_offers_to_allow_them() {
    let blank = pdfcraft_engine::Session::new().create_blank(200.0, 200.0, 1).unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("blank.pdf", None, blank.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.query_by_label(BAR).is_none() && h.query_by_label(BLOCKED).is_none(), "no scripts, no bar");
    assert!(h.state_mut().execute("tools.js_console"));
    h.run_steps(2);
    h.state_mut().js_console.input = "6 * 7".into();
    h.get_by_label("Run").click();
    h.run_steps(3);
    assert!(!h.state().js_console.log.iter().any(|l| l == "42"), "{:?}", h.state().js_console.log);
    h.state_mut().dialog = None;
    h.run_steps(2);
    h.get_by_label(BLOCKED);
    h.get_by_label("Allow for this file").click();
    h.run_steps(3);
    let id = h.state().views[0].id;
    assert!(h.state().session.scripts_allowed(id));
    assert!(h.state_mut().execute("tools.js_console"));
    h.run_steps(2);
    h.state_mut().js_console.input = "6 * 7".into();
    h.get_by_label("Run").click();
    h.run_steps(3);
    assert!(h.state().js_console.log.iter().any(|l| l == "42"), "{:?}", h.state().js_console.log);
}

#[test]
fn merge_data_files_into_a_spreadsheet() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-merge-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("report.csv");
    let mut app = PdfCraftApp::new();
    app.set_option("language", "en").unwrap();
    app.save_override = Some(out.to_string_lossy().into_owned());
    app.merge_data_files(vec![("form.pdf".into(), form())]);
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "greeting\n\n");
    assert!(app.toast.clone().unwrap().0.starts_with("Merged 1 file"));
    app.merge_data_files(vec![("x.fdf".into(), b"junk".to_vec())]);
    assert!(app.toast.clone().unwrap().0.starts_with("x.fdf:"));
}

#[test]
fn prepare_a_form_detects_fields_on_a_paper_form() {
    let paper = pdfcraft_engine::Session::new().create_from_text("t", "Name: ____________________\n\nPhone: ____________________").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("paper.pdf", None, paper.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("form.prepare"));
    h.run_steps(3);
    let app = h.state();
    let id = app.active_ids().unwrap().1;
    let names: Vec<String> = app.session.get(id).unwrap().form.iter().map(|f| f.name.clone()).collect();
    assert_eq!(names, ["Name", "Phone"]);
    assert_eq!(app.toast.clone().unwrap().0, "Detected 2 form fields");
}
