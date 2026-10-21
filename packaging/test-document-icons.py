#!/usr/bin/env python3
"""Check desktop document icons and PDF handler registration (requires Pillow)."""
from pathlib import Path
import plistlib
import unittest
import xml.etree.ElementTree as ET

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent


class DocumentIcons(unittest.TestCase):
    def test_windows_document_artwork_and_sizes(self):
        sizes = {(s, s) for s in (16, 20, 24, 32, 40, 48, 64, 128, 256)}
        with Image.open(ROOT / "assets/app-icon/pdfcraft-1024.png") as source:
            with Image.open(ROOT / "assets/app-icon/pdfcraft-document.ico") as icon:
                self.assertEqual(icon.ico.sizes(), sizes)
                for size in sorted(sizes):
                    with self.subTest(size=size):
                        expected = source.convert("RGBA").resize(size, Image.Resampling.LANCZOS)
                        self.assertEqual(icon.ico.getimage(size).convert("RGBA").tobytes(), expected.tobytes())
        build = (ROOT / "apps/pdfcraft/build.rs").read_text()
        self.assertIn('set_icon_with_id("../../assets/app-icon/pdfcraft-document.ico", "2")', build)

    def test_macos_document_icons_are_packaged(self):
        with (ROOT / "packaging/macos/Info.plist.in").open("rb") as file:
            plist = plistlib.load(file)
        self.assertTrue(plist["CFBundleDocumentTypes"])
        for document in plist["CFBundleDocumentTypes"]:
            self.assertEqual(document["CFBundleTypeIconFile"], "pdfcraft")
        package = (ROOT / "packaging/macos/package.sh").read_text()
        self.assertIn('cp "$ROOT/assets/app-icon/pdfcraft.icns" "$APP/Contents/Resources/pdfcraft.icns"', package)

    def test_windows_pdf_candidate_registration(self):
        tree = ET.parse(ROOT / "packaging/windows/pdfcraft.wxs")
        rows = [node.attrib for node in tree.findall(".//{*}RegistryValue")]

        def value(key, name=None):
            matches = [r for r in rows if r.get("Root") == "HKLM" and r.get("Key") == key and r.get("Name") == name]
            self.assertEqual(len(matches), 1, (key, name))
            return matches[0]["Value"]

        self.assertEqual(value(r"Software\Classes\ERPCons.PDF.pdf\DefaultIcon"), '"[#PdfcraftExe]",-2')
        self.assertEqual(value(r"Software\Classes\.pdf\OpenWithProgids", "ERPCons.PDF.pdf"), "")
        self.assertEqual(value(r"Software\ERPCons\PDF\Capabilities\FileAssociations", ".pdf"), "ERPCons.PDF.pdf")
        self.assertEqual(value(r"Software\RegisteredApplications", "PDF – ERPCons"), r"Software\ERPCons\PDF\Capabilities")
        self.assertFalse(any(r.get("Key", "").endswith(r"\Classes\.pdf") and not r.get("Name") for r in rows))
        self.assertFalse(any("UserChoice" in r.get("Key", "") for r in rows))


if __name__ == "__main__":
    unittest.main()
