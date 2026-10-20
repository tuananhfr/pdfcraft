//! Content and protection that a flowing Word export cannot preserve.

use crate::Document;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LossItem {
    FormFields(usize),
    Annotations(usize),
    DigitalSignatures,
    EncryptionPermissions,
    LayoutMayChange,
}

/// Word export losses; a clean document needs no confirmation for reflow alone.
pub fn export_losses(doc: &Document) -> Vec<LossItem> {
    let mut losses = Vec::new();
    if !doc.form.is_empty() {
        losses.push(LossItem::FormFields(doc.form.len()));
    }
    if !doc.info.annotations.is_empty() {
        losses.push(LossItem::Annotations(doc.info.annotations.len()));
    }
    if !doc.signatures.is_empty() || doc.form.iter().any(|field| field.kind == pdfcraft_forms::FieldKind::Signature) {
        losses.push(LossItem::DigitalSignatures);
    }
    if doc.info.encrypted || doc.security_summary().is_some() {
        losses.push(LossItem::EncryptionPermissions);
    }
    if !losses.is_empty() {
        losses.push(LossItem::LayoutMayChange);
    }
    losses
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use std::sync::Arc;

    fn fixture(field: Option<&str>, annotation: bool) -> Vec<u8> {
        let form = if field.is_some() { "/AcroForm << /Fields [4 0 R] >>" } else { "" };
        let annots = match (field.is_some(), annotation) {
            (true, true) => "/Annots [4 0 R 5 0 R]",
            (true, false) => "/Annots [4 0 R]",
            (false, true) => "/Annots [5 0 R]",
            (false, false) => "",
        };
        let objects = [
            format!("<< /Type /Catalog /Pages 2 0 R {form} >>"),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] {annots} >>"),
            field.map_or_else(|| "null".into(), |kind| {
                format!("<< /Type /Annot /Subtype /Widget /FT /{kind} /T (field) /Rect [20 20 80 40] /P 3 0 R >>")
            }),
            "<< /Type /Annot /Subtype /Text /Rect [100 100 120 120] /Contents (Review) >>".into(),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        bytes
    }

    fn losses(bytes: Vec<u8>) -> Vec<LossItem> {
        let mut session = Session::new();
        let id = session.open("doc.pdf", None, std::sync::Arc::new(bytes), None).unwrap();
        export_losses(session.get(id).unwrap())
    }

    #[test]
    fn clean_text_has_no_export_losses() {
        let bytes = Session::new().create_from_text("Clean", "Only text").unwrap().to_vec();
        assert!(losses(bytes).is_empty());
    }

    #[test]
    fn form_field_is_counted_without_counting_its_widget_as_a_comment() {
        assert_eq!(losses(fixture(Some("Tx"), false)), [LossItem::FormFields(1), LossItem::LayoutMayChange]);
    }

    #[test]
    fn annotation_is_counted() {
        assert_eq!(losses(fixture(None, true)), [LossItem::Annotations(1), LossItem::LayoutMayChange]);
    }

    #[test]
    fn unsigned_signature_field_warns_about_signatures() {
        assert_eq!(
            losses(fixture(Some("Sig"), false)),
            [LossItem::FormFields(1), LossItem::DigitalSignatures, LossItem::LayoutMayChange]
        );
    }

    #[test]
    fn encryption_and_permissions_are_reported_once() {
        let mut cos = pdfcraft_cos::Document::open(Arc::new(fixture(None, false))).unwrap();
        cos.set_encryption(&pdfcraft_cos::NewEncryption {
            algorithm: pdfcraft_cos::Algorithm::Aes256,
            user_password: "",
            owner_password: "owner",
            permissions: -1,
            encrypt_metadata: true,
            seed: [4; 32],
        })
        .unwrap();
        let bytes = pdfcraft_cos::write_full(&cos, &Default::default()).unwrap();
        assert_eq!(losses(bytes), [LossItem::EncryptionPermissions, LossItem::LayoutMayChange]);
    }

    #[test]
    fn losses_combine_in_a_stable_order_without_duplicate_layout_warning() {
        assert_eq!(
            losses(fixture(Some("Sig"), true)),
            [LossItem::FormFields(1), LossItem::Annotations(1), LossItem::DigitalSignatures, LossItem::LayoutMayChange]
        );
    }
}
