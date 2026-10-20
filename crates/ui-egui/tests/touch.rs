//! Phones and tablets: the phone-width layout and one-finger touch on the page.

use egui::{Event, Pos2, TouchDeviceId, TouchId, TouchPhase, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{Mode, PdfCraftApp, RightPanel};

/// `n` pages of 200×300, each saying "Page i pages PAGE".
fn fixture(n: usize) -> Vec<u8> {
    let mut objs: Vec<String> = vec!["<< /Type /Catalog /Pages 2 0 R >>".into()];
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {n} /MediaBox [0 0 200 300] >>", kids.join(" ")));
    objs.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    for i in 0..n {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>", 5 + 2 * i));
        let body = format!("BT /F1 18 Tf 20 150 Td (Page {} pages PAGE) Tj ET", i + 1);
        objs.push(format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let x = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn harness_at(width: f32, height: f32) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(vec2(width, height)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("a.pdf", None, fixture(3)).unwrap();
        app
    });
    h.run_steps(4);
    h
}

/// The zoomed page of [`page_harness`] at 60 frames a second with animation effects on, as a
/// phone runs it: kittest steps a quarter second and turns egui's animations off, and a finger
/// then has no velocity to fling with.
fn fling_harness() -> (Harness<'static, PdfCraftApp>, usize, Pos2) {
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_step_dt(1.0 / 60.0).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.animation_effects = pdfcraft_ui_egui::motion::AnimationEffects::On;
        app.open_bytes("a.pdf", None, fixture(3)).unwrap();
        app
    });
    h.ctx.all_styles_mut(|s| s.animation_time = 1.0 / 12.0);
    h.state_mut().set_option("zoom", "200").unwrap();
    h.state_mut().set_option("left", "closed").unwrap();
    h.state_mut().set_option("panel", "none").unwrap();
    h.run_steps(30);
    let view = h.state().active.unwrap();
    let page = h.state().views[view].page_screen_rect(0).unwrap();
    (h, view, page.center())
}

/// A finger dragged `step` points a frame for five frames, then lifted.
fn flick(h: &mut Harness<'static, PdfCraftApp>, from: Pos2, step: egui::Vec2) {
    finger_down(h, from);
    h.run_steps(1);
    for i in 1..=5 {
        finger_move(h, from + step * i as f32);
        h.run_steps(1);
    }
    finger_up(h, from + step * 5.0);
    h.run_steps(1);
}

/// A document zoomed in with the panels closed, and a point on its first line of text.
fn page_harness() -> (Harness<'static, PdfCraftApp>, usize, Pos2) {
    let mut h = harness_at(1400.0, 900.0);
    h.state_mut().set_option("zoom", "200").unwrap();
    h.state_mut().set_option("left", "closed").unwrap();
    h.state_mut().set_option("panel", "none").unwrap();
    h.run_steps(8);
    let view = h.state().active.unwrap();
    let page = h.state().views[view].page_screen_rect(0).unwrap();
    (h, view, egui::pos2(page.left() + page.width() * 0.11, page.top() + page.height() * 0.48))
}

// A finger as eframe's web runner reports it: pointer events plus `Event::Touch`.
fn touch(h: &Harness<'static, PdfCraftApp>, phase: TouchPhase, pos: Pos2) {
    h.event(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(0), phase, pos, force: None });
}

fn finger_down(h: &Harness<'static, PdfCraftApp>, pos: Pos2) {
    h.event(Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    touch(h, TouchPhase::Start, pos);
}

fn finger_move(h: &Harness<'static, PdfCraftApp>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    touch(h, TouchPhase::Move, pos);
}

fn finger_up(h: &Harness<'static, PdfCraftApp>, pos: Pos2) {
    h.event(Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.event(Event::PointerGone);
    touch(h, TouchPhase::End, pos);
}

fn page_top(h: &Harness<'static, PdfCraftApp>, view: usize) -> f32 {
    h.state().views[view].page_screen_rect(0).unwrap().top()
}

#[test]
fn one_finger_drag_on_the_page_scrolls_without_selecting() {
    let (mut h, view, start) = page_harness();
    let before = page_top(&h, view);
    finger_down(&h, start);
    h.run_steps(1);
    finger_move(&h, start + vec2(0.0, -80.0));
    h.run_steps(1);
    finger_move(&h, start + vec2(0.0, -160.0));
    h.run_steps(1);
    assert!(h.state().views[view].selected_text().is_none(), "a touch drag must not select text");
    finger_up(&h, start + vec2(0.0, -160.0));
    h.run_steps(3);
    let after = page_top(&h, view);
    assert!(after < before - 100.0, "the page follows the finger: {before} -> {after}");
    assert!(h.state().views[view].selected_text().is_none());
}

#[test]
fn a_quick_flick_keeps_the_page_moving_until_it_slows_to_a_stop() {
    let (mut h, view, centre) = fling_harness();
    let start = page_top(&h, view);
    flick(&mut h, centre, vec2(0.0, -24.0));
    let lifted = page_top(&h, view);
    assert!(lifted < start - 100.0, "the page followed the finger: {start} -> {lifted}");
    h.run_steps(10);
    let gliding = page_top(&h, view);
    assert!(gliding < lifted - 30.0, "it keeps going after the finger lifts: {lifted} -> {gliding}");
    h.run_steps(120);
    let stopped = page_top(&h, view);
    assert!(stopped < gliding, "and slows down rather than stopping dead");
    h.run_steps(10);
    assert_eq!(page_top(&h, view), stopped, "then it stops");
}

#[test]
fn a_slow_drag_does_not_fling_and_a_new_touch_stops_a_fling() {
    let (mut h, view, centre) = fling_harness();
    flick(&mut h, centre, vec2(0.0, -3.0));
    let lifted = page_top(&h, view);
    h.run_steps(10);
    assert_eq!(page_top(&h, view), lifted, "a slow release leaves the page where the finger did");

    flick(&mut h, centre, vec2(0.0, -24.0));
    h.run_steps(3);
    finger_down(&h, centre);
    h.run_steps(2);
    let caught = page_top(&h, view);
    h.run_steps(10);
    assert_eq!(page_top(&h, view), caught, "a finger on the page catches it");
    finger_up(&h, centre);
    h.run_steps(10);
    assert_eq!(page_top(&h, view), caught);
}

#[test]
fn a_quick_swipe_still_turns_one_page_with_flinging_on() {
    let (mut h, view, _) = fling_harness();
    h.state_mut().set_option("layout", "single").unwrap();
    h.state_mut().set_option("zoom", "100").unwrap();
    h.run_steps(10);
    let centre = h.state().views[view].page_screen_rect(0).unwrap().center();
    flick(&mut h, centre, vec2(-48.0, 1.0));
    h.run_steps(60);
    assert_eq!(h.state().views[view].current, 1, "one page, and the glide doesn't scroll the next");
    let shown = |h: &Harness<'static, PdfCraftApp>| h.state().views[view].page_screen_rect(1).map(|r| r.top());
    let top = shown(&h);
    assert!(top.is_some());
    h.run_steps(10);
    assert_eq!(shown(&h), top);
}

#[test]
fn a_mouse_drag_still_selects_text_after_the_screen_was_touched() {
    let (mut h, view, start) = page_harness();
    // A tap: the device has a touch screen now, as a convertible laptop does.
    finger_down(&h, start + vec2(0.0, 200.0));
    h.run_steps(1);
    finger_up(&h, start + vec2(0.0, 200.0));
    h.run_steps(2);
    let before = page_top(&h, view);
    h.hover_at(start);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    h.hover_at(start + vec2(100.0, 0.0));
    h.run_steps(2);
    assert!(h.state().views[view].selected_text().is_some_and(|s| !s.is_empty()), "the mouse selects text");
    h.event(Event::PointerButton {
        pos: start + vec2(100.0, 0.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    h.run_steps(2);
    assert_eq!(page_top(&h, view), before, "a mouse drag never pans with the Select tool");
}

#[test]
fn a_long_press_selects_the_word_and_a_drag_extends_it() {
    let (mut h, view, start) = page_harness();
    let before = page_top(&h, view);
    finger_down(&h, start);
    // egui turns a press held 0.8 s without moving into a long touch (4 frames of 0.25 s).
    h.run_steps(5);
    assert_eq!(h.state().views[view].selected_text().as_deref(), Some("Page"), "the long-pressed word");
    assert!(!egui::Popup::is_any_open(&h.ctx), "the selection replaces the page's context menu");
    finger_move(&h, start + vec2(240.0, 0.0));
    h.run_steps(2);
    let extended = h.state().views[view].selected_text().unwrap_or_default();
    assert!(extended.starts_with("Page 1 pa"), "dragging on extends the selection: {extended:?}");
    finger_up(&h, start + vec2(240.0, 0.0));
    h.run_steps(2);
    assert_eq!(page_top(&h, view), before, "selecting does not pan");
    assert!(h.state().views[view].selected_text().is_some_and(|s| s.starts_with("Page 1 pa")));
}

#[test]
fn a_swipe_turns_the_page_in_single_page_view() {
    let mut h = harness_at(1400.0, 900.0);
    h.state_mut().set_option("left", "closed").unwrap();
    h.state_mut().set_option("panel", "none").unwrap();
    h.state_mut().set_option("layout", "single").unwrap();
    h.state_mut().set_option("zoom", "100").unwrap();
    h.run_steps(6);
    let view = h.state().active.unwrap();
    assert_eq!(h.state().views[view].current, 0);
    let centre = h.state().views[view].page_screen_rect(0).unwrap().center();
    let swipe = |h: &mut Harness<'static, PdfCraftApp>, dx: f32| {
        finger_down(h, centre);
        h.run_steps(1);
        finger_move(h, centre + vec2(dx / 2.0, 4.0));
        h.run_steps(1);
        finger_move(h, centre + vec2(dx, 6.0));
        h.run_steps(1);
        finger_up(h, centre + vec2(dx, 6.0));
        h.run_steps(4);
    };
    swipe(&mut h, -240.0);
    assert_eq!(h.state().views[view].current, 1, "a swipe to the left shows the next page");
    swipe(&mut h, 240.0);
    assert_eq!(h.state().views[view].current, 0, "a swipe to the right goes back");
    swipe(&mut h, -30.0);
    assert_eq!(h.state().views[view].current, 0, "a short drag is not a swipe");
}

#[test]
fn a_phone_width_window_folds_the_side_panels_once() {
    let mut h = harness_at(1400.0, 900.0);
    h.state_mut().right = Some(RightPanel::Pages);
    h.run_steps(2);
    assert!(h.state().left_open && !h.state().is_narrow());
    h.set_size(vec2(390.0, 844.0));
    h.run_steps(3);
    assert!(h.state().is_narrow());
    assert_eq!((h.state().left_open, h.state().right), (false, None), "the page gets the window");
    assert!(!h.state().right_panel_closed, "folding is not the user closing the panel");
    let saved: serde_json::Value = serde_json::from_str(&h.state().persist()).unwrap();
    assert_eq!((saved["docking_left_open"].as_bool(), saved["docking_right"].as_str()), (Some(true), Some("Pages")), "settings keep the wide layout");
    // A panel opened on the phone is the user's choice and stays.
    h.state_mut().choose_right_panel(Some(RightPanel::Bookmarks));
    h.run_steps(3);
    assert_eq!(h.state().right, Some(RightPanel::Bookmarks));
    h.set_size(vec2(1400.0, 900.0));
    h.run_steps(3);
    assert!(!h.state().is_narrow());
    assert_eq!((h.state().left_open, h.state().right), (true, Some(RightPanel::Bookmarks)));
}

#[test]
fn a_phone_width_window_fits_the_page_width_and_widening_gives_the_zoom_back() {
    let mut h = harness_at(1400.0, 900.0);
    h.state_mut().set_option("zoom", "150").unwrap();
    h.run_steps(3);
    let view = h.state().active.unwrap();
    let zoom = |h: &Harness<'static, PdfCraftApp>| (h.state().views[view].fit, h.state().views[view].zoom);
    assert_eq!(zoom(&h), (pdfcraft_ui_egui::canvas::Fit::None, 1.5));
    h.set_size(vec2(390.0, 844.0));
    h.run_steps(3);
    assert_eq!(zoom(&h).0, pdfcraft_ui_egui::canvas::Fit::Width, "the page fits the phone's width");
    let page = h.state().views[view].page_screen_rect(0).unwrap();
    let canvas = h.state().views[view].viewport_rect();
    assert!(page.left() >= canvas.left() && page.right() <= canvas.right(), "nothing is cut at the sides: {page:?} in {canvas:?}");
    assert!(page.width() >= 0.85 * canvas.width(), "the page fills the canvas's width: {page:?} in {canvas:?}");
    // A zoom picked on the phone stands while it stays narrow.
    h.state_mut().set_option("zoom", "300").unwrap();
    h.run_steps(3);
    h.set_size(vec2(420.0, 844.0));
    h.run_steps(3);
    assert_eq!(zoom(&h), (pdfcraft_ui_egui::canvas::Fit::None, 3.0));
    h.set_size(vec2(1400.0, 900.0));
    h.run_steps(3);
    assert_eq!(zoom(&h), (pdfcraft_ui_egui::canvas::Fit::None, 1.5), "the wide zoom is back");
}

#[test]
fn a_tablet_mode_bar_keeps_every_mode_clear_of_the_search() {
    for (lang, width) in [("vi", 768.0), ("en", 768.0), ("vi", 1024.0), ("vi", 1366.0)] {
        let mut h = harness_at(width, 1024.0);
        h.state_mut().set_option("language", lang).unwrap();
        h.run_steps(4);
        let tr = |s: &'static str| pdfcraft_ui_egui::i18n::tr(pdfcraft_ui_egui::i18n::Lang::from_pref(lang), s);
        // The mode bar is the row under the tab strip; "All tools" also heads the Tools pane.
        let in_bar = |label: &str| {
            let found: Vec<egui::Rect> = h.get_all_by_label(label).map(|n| n.rect()).filter(|r| r.center().y > 38.0 && r.center().y < 86.0).collect();
            assert_eq!(found.len(), 1, "{lang} {width}: {label} once in the mode bar");
            found[0]
        };
        let find = in_bar(tr("Find tools and commands"));
        for mode in ["All tools", "Read", "Edit", "Convert", "E-Sign"] {
            let r = in_bar(tr(mode));
            assert!(r.right() <= find.left(), "{lang} {width}: {mode} {r:?} runs under the search {find:?}");
        }
        assert!(find.right() <= width);
    }
}

#[test]
fn the_phone_mode_bar_folds_the_modes_into_a_picker() {
    let mut h = harness_at(390.0, 844.0);
    assert!(h.query_by_label("E-Sign").is_none(), "only the current mode shows");
    h.get_by_label("All tools").click();
    h.run_steps(2);
    h.get_by_label("Read").click();
    h.run_steps(2);
    assert_eq!(h.state().mode, Mode::Read);
    // Everything else on the bars stays reachable.
    for label in ["Menu", "Find tools and commands", "Keyboard shortcuts"] {
        h.get_by_label(label);
    }
    h.set_size(vec2(1400.0, 900.0));
    h.run_steps(3);
    h.get_by_label("E-Sign");
}
