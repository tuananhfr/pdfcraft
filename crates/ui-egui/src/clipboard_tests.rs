use std::cell::RefCell;
use std::rc::Rc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_render::{PageRenderer, RenderRequest, RequestKind};

use super::{Contents, HostClipboard, ReadInbox};
use crate::PdfCraftApp;

fn selection_app(allow_copy: bool) -> PdfCraftApp {
    let bytes = pdfcraft_engine::Session::new().create_from_text("Clipboard", "Selection text").unwrap();
    let mut pdf = pdfcraft_cos::Document::open(bytes).unwrap();
    pdf.set_encryption(&pdfcraft_cos::NewEncryption {
        algorithm: pdfcraft_cos::Algorithm::Aes256,
        user_password: "",
        owner_password: "owner",
        permissions: if allow_copy { -1 } else { -1 ^ 16 },
        encrypt_metadata: true,
        seed: [4; 32],
    })
    .unwrap();
    let mut app = PdfCraftApp::new();
    app.set_option("language", "en").unwrap();
    app.open_bytes("clipboard.pdf", None, pdfcraft_cos::write_full(&pdf, &Default::default()).unwrap()).unwrap();
    let doc = app.session.get(app.views[0].id).unwrap();
    assert_eq!(doc.allows_copying(), allow_copy);
    let mut renderer = PageRenderer::new(doc.bytes.clone(), Default::default());
    let text = renderer.render(RenderRequest { page: 0, kind: RequestKind::Text, scale: 1.0, ..Default::default() }).text.unwrap();
    app.views[0].texts.insert(0, text);
    assert!(app.views[0].select_all());
    app
}

#[test]
fn floating_copy_obeys_permissions_before_platform_or_host_writes() {
    for allow_copy in [false, true] {
        for destination in [0, 1, 2] {
            let writes = Rc::new(RefCell::new(Vec::<String>::new()));
            let captured = writes.clone();
            let mut app = selection_app(allow_copy);
            if destination == 1 {
                app.clipboard = Some(HostClipboard {
                    read: Box::new(|| {}),
                    write: Box::new(move |text| captured.borrow_mut().push(text.into())),
                    inbox: ReadInbox::default(),
                });
            }
            let captured = writes.clone();
            if destination == 2 {
                app.desktop_copy_text = Some(Box::new(move |text| captured.borrow_mut().push(text.into())));
            }
            let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_| app);
            h.run_steps(4);
            let selected = h.state().views[0].selected_text().unwrap();
            assert!(selected.contains("Selection text"));
            h.get_by_label("Copy").click();
            let mut platform_writes = Vec::new();
            for _ in 0..4 {
                h.step();
                for command in &h.output().platform_output.commands {
                    if let egui::OutputCommand::CopyText(text) = command {
                        platform_writes.push(text.clone());
                    }
                }
            }
            if allow_copy {
                let actual = if destination != 0 { writes.borrow().clone() } else { platform_writes };
                assert_eq!(actual, [selected]);
            } else {
                assert!(platform_writes.is_empty());
                assert!(writes.borrow().is_empty());
                assert_eq!(
                    h.state().toast.as_ref().map(|t| t.0.as_str()),
                    Some("The document's security settings don't allow copying text and images")
                );
            }
        }
    }
}

#[test]
fn clipboard_command_requests_host_read_and_creates_text_pdf() {
    let inbox = ReadInbox::default();
    let queue = inbox.clone();
    let mut app = PdfCraftApp::new();
    app.clipboard = Some(HostClipboard {
        read: Box::new(move || queue.lock().unwrap().push(Ok(Some(Contents::Text("Clipboard text".into()))))),
        write: Box::new(|_| {}),
        inbox,
    });
    assert!(app.execute("create.clipboard"));
    assert!(app.views.is_empty());
    app.poll_clipboard();
    assert_eq!(app.views.len(), 1);
    let doc = app.session.get(app.views[0].id).unwrap();
    assert!(doc.dirty);
    let mut renderer = PageRenderer::new(doc.bytes.clone(), Default::default());
    let text = renderer.render(RenderRequest { page: 0, kind: RequestKind::Text, scale: 1.0, ..Default::default() }).text.unwrap();
    assert!(text.plain_text().contains("Clipboard text"));
}

#[test]
fn clipboard_read_refusal_is_localized_and_creates_nothing() {
    let mut app = PdfCraftApp::new();
    let inbox = ReadInbox::default();
    app.clipboard = Some(HostClipboard { read: Box::new(|| {}), write: Box::new(|_| {}), inbox });
    app.clipboard.as_ref().unwrap().inbox.lock().unwrap().push(Err(()));
    crate::i18n::set_current(crate::i18n::Lang::from_pref("vi"));
    app.poll_clipboard();
    assert!(app.views.is_empty());
    assert_eq!(app.toast.as_ref().unwrap().0, "Trình duyệt không cho đọc bảng tạm — hãy dùng Ctrl+V");
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

#[test]
fn clipboard_encoded_png_creates_a_pdf() {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0, 0, 0, 255]).unwrap();
        writer.finish().unwrap();
    }
    let mut app = PdfCraftApp::new();
    let inbox = ReadInbox::default();
    inbox.lock().unwrap().push(Ok(Some(Contents::Image { name: "Clipboard.png".into(), bytes: png })));
    app.clipboard = Some(HostClipboard { read: Box::new(|| {}), write: Box::new(|_| {}), inbox });
    app.poll_clipboard();
    assert_eq!(app.views.len(), 1);
    assert!(app.session.get(app.views[0].id).unwrap().dirty);
}
