# ERPCons PDF app icons

Contributor-original artwork by Tuan Anh PHAM for ERPCons, licensed MIT OR Apache-2.0.
Source: github.com/tuananhfr/erpcons-office, commit 133af85, public/brand/office/.
The owner approved the artwork on 2026-10-21. The lead derived the sizes, ICO and ICNS with Pillow.

The hicolor PNGs use `vn.erpcons.pdf`; the native host uses the 256 px PNG or `pdfcraft-1024.png`.
Windows embeds `pdfcraft.ico`; macOS packages `pdfcraft.icns`.
Windows PDF files use the same ERPCons artwork in `pdfcraft-document.ico` (resource 2).
Regenerate it from `pdfcraft-1024.png` with `bash packaging/icons.sh` (Python 3 with Pillow).
It retains the document icon sizes: 16, 20, 24, 32, 40, 48, 64, 128 and 256 px.
The obsolete Lucide-derived document SVG has been removed. See `LICENSE-document.txt`.
macOS document types use the packaged `pdfcraft.icns` app icon.
