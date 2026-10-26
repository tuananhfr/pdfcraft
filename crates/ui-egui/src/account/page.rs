//! The Account dialog, the tab strip's account button, Help ▸ Account and the paid licence expiry notice.

use egui::{Align2, Color32, RichText, Sense, TextWrapMode, vec2};

use super::{AccountError, AccountSnapshot, AccountView, LicenceView, SignedInView, date_text, error_text};
use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, i18n, icons, widgets};

/// The error colour of the password prompts.
const ERROR: Color32 = Color32::from_rgb(0xD1, 0x3B, 0x3B);

enum Action {
    SignIn,
    CancelSignIn,
    SignOut,
    SelectOrg(Option<String>),
    RetryActivation,
}

/// The Account dialog's content, above the dialog's Close button. True when there is nothing to
/// show (no account hook), so the dialog closes.
pub(crate) fn body(app: &mut PdfCraftApp, ui: &mut egui::Ui) -> bool {
    let Some(hook) = app.account_hook.clone() else { return true };
    let snapshot = app.account.snapshot.clone();
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!("Account")).font(theme::semibold(18.0)));
    ui.add_space(8.0);
    let mut action = None;
    match &snapshot.view {
        AccountView::Guest => {
            ui.add(
                egui::Label::new(RichText::new(tl!("Signing in is optional: every feature works without an account.")).color(t.text_muted)).wrap(),
            );
            ui.add_space(12.0);
            if ui.add_enabled_ui(!snapshot.busy, |ui| widgets::pill_button(ui, tl!("Sign in with PI ID"), true)).inner.clicked() {
                action = Some(Action::SignIn);
            }
        }
        AccountView::SigningIn => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tl!("Continue in your browser…"));
            });
            ui.add_space(10.0);
            if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
                action = Some(Action::CancelSignIn);
            }
        }
        AccountView::SignedIn(view) => {
            ui.add_enabled_ui(!snapshot.busy, |ui| signed_in(ui, &app.session, view, &t, &mut action));
        }
    }
    if snapshot.busy && snapshot.view != AccountView::SigningIn {
        ui.add_space(8.0);
        ui.spinner();
    }
    if let Some(text) = shown_error(&snapshot).and_then(error_text) {
        ui.add_space(10.0);
        ui.add(egui::Label::new(RichText::new(text).color(ERROR)).wrap());
    }
    // The web adapter opens its popup here: still inside the click's frame, as browsers require.
    match action {
        Some(Action::SignIn) => hook.sign_in(),
        Some(Action::CancelSignIn) => hook.cancel_sign_in(),
        Some(Action::SignOut) => hook.sign_out(),
        Some(Action::SelectOrg(org)) => hook.select_org(org),
        Some(Action::RetryActivation) => hook.retry_activation(),
        None => {}
    }
    false
}

/// The snapshot's error, unless the page already states that fact (one message per fact).
fn shown_error(snapshot: &AccountSnapshot) -> Option<AccountError> {
    let error = snapshot.error?;
    let AccountView::SignedIn(view) = &snapshot.view else { return Some(error) };
    let repeated = match error {
        AccountError::DeviceCap => view.licence == LicenceView::DeviceCapReached,
        AccountError::DeviceRevoked => view.licence == LicenceView::DeviceRevoked,
        AccountError::ReauthRequired => view.reauth_required,
        _ => false,
    };
    (!repeated).then_some(error)
}

fn signed_in(ui: &mut egui::Ui, session: &pdfcraft_engine::Session, view: &SignedInView, t: &Tokens, action: &mut Option<Action>) {
    let name = display_name(view);
    // Wrapping would split a long name or address mid-word in a phone-wide dialog.
    one_line_label(ui, RichText::new(&name).font(theme::semibold(15.0)));
    if let Some(email) = view.email.as_deref().filter(|email| !email.is_empty() && *email != name) {
        one_line_label(ui, RichText::new(email).color(t.text_muted));
    }
    if view.reauth_required {
        ui.add_space(8.0);
        ui.add(egui::Label::new(tl!("Your session has expired. Sign in again.")).wrap());
        ui.add_space(4.0);
        if widgets::pill_button(ui, tl!("Sign in again"), true).clicked() {
            *action = Some(Action::SignIn);
        }
    }
    widgets::section_title(ui, tl!("Organization"));
    let personal = std::iter::once((None, tl!("Personal").to_string()));
    let orgs = view.orgs.iter().map(|org| (Some(org.id.clone()), org.name.clone()));
    for (id, label) in personal.chain(orgs) {
        if one_line_radio(ui, view.active_org == id, &label).clicked() && view.active_org != id {
            *action = Some(Action::SelectOrg(id));
        }
    }
    widgets::section_title(ui, tl!("License"));
    for line in licence_lines(session, view.licence) {
        ui.add(egui::Label::new(line).wrap());
    }
    if view.licence == LicenceView::DeviceCapReached {
        ui.add_space(4.0);
        if widgets::pill_button(ui, tl!("Retry"), false).clicked() {
            *action = Some(Action::RetryActivation);
        }
    }
    if view.session_only && view.licence != LicenceView::SessionOnly {
        ui.add(egui::Label::new(RichText::new(tl!("You will need to sign in again the next time you open the app.")).color(t.text_muted)).wrap());
    }
    ui.add_space(8.0);
    let online = if view.online { tl!("Connected to PI ID") } else { tl!("Offline. Your account syncs when you reconnect.") };
    ui.add(egui::Label::new(RichText::new(online).color(t.text_muted)).wrap());
    ui.add_space(12.0);
    if widgets::pill_button(ui, tl!("Sign out"), false).clicked() {
        *action = Some(Action::SignOut);
    }
}

fn fits_on_one_line(ui: &egui::Ui, text: impl Into<egui::WidgetText>, reserved: f32) -> bool {
    let galley = text.into().into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, egui::FontSelection::Default);
    reserved + galley.size().x <= ui.available_width()
}

/// A label on one line, elided with the full text on hover when it does not fit.
fn one_line_label(ui: &mut egui::Ui, text: RichText) {
    let fits = fits_on_one_line(ui, text.clone(), 0.0);
    let full = text.text().to_owned();
    // egui's own elision tooltip is wider than a phone; this one wraps within the window.
    let response = ui.add(egui::Label::new(text).truncate().show_tooltip_when_elided(false));
    if !fits {
        response.on_hover_ui(|ui| widgets::fitted_tip(ui, &full));
    }
}

/// [`widgets::radio`] on one line, elided with the full text on hover when it does not fit.
fn one_line_radio(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    let fits = fits_on_one_line(ui, label, ui.spacing().icon_width + ui.spacing().icon_spacing);
    let response = ui
        .scope(|ui| {
            ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
            ui.add(widgets::radio(selected, label))
        })
        .inner;
    if fits { response } else { response.on_hover_ui(|ui| widgets::fitted_tip(ui, label)) }
}

fn licence_lines(session: &pdfcraft_engine::Session, licence: LicenceView) -> Vec<String> {
    let dated = |template: &str, at: i64| i18n::fmt(template, &[("date", &date_text(session, at))]);
    match licence {
        LicenceView::Free | LicenceView::Online { pro: false } => vec![tl!("Plan: Free").into()],
        LicenceView::Active { expires_at, pro: false } => {
            vec![tl!("Plan: Free").into(), dated(tl!("This device is activated until {date}."), expires_at)]
        }
        LicenceView::Active { expires_at, pro: true } => vec![dated(tl!("Paid features are valid until {date}."), expires_at)],
        LicenceView::Online { pro: true } => vec![tl!("Paid features are active.").into()],
        LicenceView::Inactive { expires_at } => vec![dated(tl!("Your license expired on {date}. Free features keep working."), expires_at)],
        LicenceView::DeviceRevoked => vec![tl!("This device was removed from your account. Free features keep working.").into()],
        LicenceView::DeviceCapReached => vec![tl!("Your account has reached its device limit, so this device is not activated.").into()],
        LicenceView::SessionOnly => vec![tl!("Signed in for this session only: this computer has no credential store.").into()],
    }
}

fn display_name(view: &SignedInView) -> String {
    match (view.name.trim(), view.email.as_deref()) {
        ("", Some(email)) if !email.is_empty() => email.to_string(),
        ("", _) => tl!("PI ID account").to_string(),
        (name, _) => name.to_string(),
    }
}

fn initial(view: &SignedInView) -> Option<String> {
    crate::account::avatar_letter(&view.name, view.email.as_deref())
}

/// The tab strip's account button: the user's initial when signed in, a person otherwise. It opens
/// the Account dialog.
pub(crate) fn strip_button(app: &mut PdfCraftApp, ui: &mut egui::Ui, size: f32) {
    if app.account_hook.is_none() {
        return;
    }
    let (tip, letter) = match &app.account.snapshot.view {
        AccountView::SignedIn(view) => (display_name(view), initial(view)),
        AccountView::SigningIn => (tl!("Signing in…").to_string(), None),
        AccountView::Guest => (tl!("Sign in").to_string(), None),
    };
    let clicked = match letter {
        None => icons::button(ui, "user-round", size, false, &tip).clicked(),
        Some(letter) => {
            let t = Tokens::get(ui.ctx());
            let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, t.radius, t.hover);
            }
            let radius = (size / 2.0 - 4.0).max(6.0);
            ui.painter().circle_filled(rect.center(), radius, t.accent);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, letter, theme::semibold(11.0), Color32::WHITE);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &tip));
            theme::hand(resp).on_hover_text(&tip).clicked()
        }
    };
    if clicked {
        app.show_account();
    }
}

/// Help ▸ Account, so the dialog stays reachable when a narrow tab strip leaves the button out.
pub(crate) fn help_menu_item(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    if app.account_hook.is_none() {
        return;
    }
    ui.separator();
    if widgets::menu_item(ui, tl!("Account"), "").clicked() {
        app.show_account();
        ui.close();
    }
}

/// The lease expiry notice under the mode bar, shown only while paid features hang on that lease
/// (lease.md §8). Closing it tells the adapter, which records the stage as shown.
pub(crate) fn warning_notice(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    let Some(hook) = app.account_hook.clone() else { return };
    let AccountView::SignedIn(view) = &app.account.snapshot.view else { return };
    let (Some(warning), LicenceView::Active { pro: true, .. }) = (view.warning, view.licence) else { return };
    if app.account.dismissed == Some(warning) {
        return;
    }
    let text = i18n::fmt(
        tl!("Paid features expire on {date}. Connect to the internet to renew them."),
        &[("date", &date_text(&app.session, warning.expires_at))],
    );
    let t = Tokens::get(ui.ctx());
    let (mut close, mut open) = (false, false);
    egui::Panel::top("account_warning").resizable(false).frame(egui::Frame::NONE).show(ui, |ui| {
        crate::canvas::notice_bar(ui, &t, "clock", t.accent_text, text, |ui| {
            close = icons::button(ui, "x", 22.0, false, tl!("Dismiss")).clicked();
            open = widgets::pill_button(ui, tl!("Account"), false).clicked();
        });
    });
    if close {
        app.account.dismissed = Some(warning);
        hook.warning_shown(warning.days);
    }
    if open {
        app.show_account();
    }
}
