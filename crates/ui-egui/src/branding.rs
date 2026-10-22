//! Host branding, applied after localisation so catalog keys stay stable.

use std::cell::Cell;
use std::collections::HashMap;

pub struct Branding {
    pub product_name: &'static str,
    pub publisher: &'static str,
    pub website: &'static str,
    text: HashMap<String, String>,
}

impl Branding {
    pub fn new(product_name: &'static str, publisher: &'static str, website: &'static str) -> Self {
        let mut text = HashMap::new();
        for language in &crate::i18n::LANGUAGES {
            let (entries, _) = crate::i18n::catalog::parse_entries(language.source, language.plural_forms());
            for entry in entries {
                for (value, rtl) in [(entry.source, false), (entry.translation, language.rtl)] {
                    for form in value.split('|').filter(|value| value.contains("PdfCraft")) {
                        let branded = form.replace("PdfCraft", product_name);
                        let (key, value) =
                            if rtl { (crate::bidi::display_rtl(form), crate::bidi::display_rtl(&branded)) } else { (form.to_owned(), branded) };
                        text.insert(key, value);
                    }
                }
            }
        }
        Self { product_name, publisher, website, text }
    }
}

/// ERPCons identity shared by native and web hosts.
pub fn erpcons() -> &'static Branding {
    static BRANDING: std::sync::OnceLock<Branding> = std::sync::OnceLock::new();
    BRANDING.get_or_init(|| Branding::new("PDF – ERPCons", "ERPCons", "https://office.erpcons.ai"))
}

thread_local! {
    static CURRENT: Cell<Option<&'static Branding>> = const { Cell::new(None) };
}

pub fn set_current(branding: Option<&'static Branding>) {
    CURRENT.set(branding);
}

pub(crate) fn text(value: &str) -> &str {
    CURRENT.get().and_then(|branding| branding.text.get(value)).map_or(value, String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_branding_preserves_unbranded_default_and_translates_product_names() {
        static DESKTOP: std::sync::OnceLock<Branding> = std::sync::OnceLock::new();
        let branding = DESKTOP.get_or_init(|| Branding::new("PDF – ERPCons", "ERPCons", "https://office.erpcons.ai"));
        let mut app = crate::PdfCraftApp::new();
        assert_eq!(app.product_name(), "PdfCraft");
        set_current(None);
        assert_eq!(crate::i18n::tr(crate::i18n::Lang::EN, "About PdfCraft"), "About PdfCraft");
        app.branding = Some(branding);
        set_current(app.branding);
        assert_eq!(app.product_name(), "PDF – ERPCons");
        for language in &crate::i18n::LANGUAGES {
            let lang = crate::i18n::Lang::from_code(language.code).unwrap();
            for key in ["About PdfCraft", "Welcome to PdfCraft", "PdfCraft {v} is available."] {
                let label = crate::i18n::tr(lang, key);
                assert!(label.contains("PDF – ERPCons"), "{}: {label}", language.code);
                assert!(!label.contains("PdfCraft"));
            }
        }
        set_current(None);
        assert_eq!(crate::i18n::tr(crate::i18n::Lang::EN, "About PdfCraft"), "About PdfCraft");
        assert_eq!(crate::PdfCraftApp::new().product_name(), "PdfCraft");
    }
}

#[cfg(test)]
mod web_tests {
    #[test]
    fn web_branding_uses_erpcons_product_and_title() {
        let branding = super::erpcons();
        assert_eq!(branding.product_name, "PDF – ERPCons");
        assert_eq!(branding.publisher, "ERPCons");
        assert_eq!(branding.website, "https://office.erpcons.ai");
        assert!(include_str!("../../../apps/pdfcraft-web/index.html").contains("<title>PDF – ERPCons</title>"));
        super::set_current(Some(branding));
        for (lang, expected) in [
            (crate::i18n::Lang::EN, "Welcome to PDF – ERPCons"),
            (crate::i18n::Lang::from_code("vi").unwrap(), "Chào mừng bạn đến với PDF – ERPCons"),
        ] {
            assert_eq!(crate::i18n::tr(lang, "Welcome to PdfCraft"), expected);
        }
        super::set_current(None);
    }
}
