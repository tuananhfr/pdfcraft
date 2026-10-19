# ERPCons changes

This branch (`erpcons`) is the ERPCons Văn phòng fork of [PdfCraft](https://github.com/storytold/pdfcraft), based on PdfCraft by the ArtCraft team, licensed MIT OR Apache-2.0 (see `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). Changes from upstream `main` (Apache-2.0 §4b):

- Removed the ArtCraft logos and marks (`docs/brand/` and their ATTRIBUTION entries) and every link to the ArtCraft Discord and getartcraft.com, as the ArtCraft brand terms require for forks: title-bar Discord button, home-screen community card, About dialog, Help menu entries, CLI and MCP server info, README. The README screenshot was regenerated without the mark.
- About now reads "ERPCons Văn phòng — dựa trên PdfCraft của nhóm ArtCraft. MIT OR Apache-2.0."; Help ▸ "PdfCraft on GitHub" and the releases page point at this fork.
- Help ▸ "Check for updates…" is not in the web build's menu.
- Dropped the translations of the removed strings.
- Web draft autosave for the ERPCons Văn phòng shell (`crates/ui-egui/src/drafts.rs`, `apps/pdfcraft-web/src/main.rs`): unsaved documents are handed to `window.erpDrafts` (format `pdf`, the working bytes) after 2.5 s idle / 30 s of continuous edits and when the page is hidden, removed once saved, undone to saved or closed, and drafts the shell restores reopen unsaved under their key. Off without `window.erpDrafts` and in `?host=parent` mode; the desktop recovery store is unchanged.
