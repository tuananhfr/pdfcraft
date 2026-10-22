# pdfcraft-engine

The façade every frontend talks to. It holds the open documents, their edit history and the tool
catalogue; frontends never reach past it into the parsing, rendering or editing crates.

- **Layer:** L6. It depends on most of L1–L4 and on nothing above itself. **No UI toolkit** —
  `egui`, `winit`, `eframe` and `rfd` are forbidden below L7, and `xtask layers` enforces it.
- The crate denies `unwrap`/`expect`/`panic!`/`todo!`. Entry points that run untrusted-document
  code keep a last-resort `catch_unwind` guard so an escaped panic becomes an error and the user
  keeps their document.

Two frontends exist on top of it today: `pdfcraft-ui-egui` (desktop and web) and
`pdfcraft-automation` (headless tools, the CLI and MCP). Anything one can do, the other must be
able to do — see AGENTS.md §3.

## The editing model

Each open document holds a `pdfcraft_cos::Document`: the object graph plus a copy-on-write overlay.

1. An edit runs **on a clone**. On success the previous state is pushed onto the undo stack. Clones
   share all unchanged data, so a snapshot is cheap — this is what makes 100 levels of undo
   affordable on a 200 MB file.
2. After every edit the **working file** is produced by an incremental write: the original bytes
   plus one appended revision.
3. The view is refreshed from those bytes, so **what you see is exactly what Save will write**.
4. Saving rebases onto the written bytes, so the next save appends only new edits.

Consequences worth preserving: the original bytes of an opened file are never rewritten in place; a
full rewrite happens only where it must (redaction, PDF/A conversion, print imposition, a changed
password); and unknown data survives a round trip because nothing is reconstructed that wasn't
understood.

## API

```rust
let mut s = Session::new();
let id = s.open("report.pdf", Some(path), bytes, None)?;   // -> DocId

s.apply(id, Edit::RotatePages { pages: vec![0], degrees: 90 })?;
let label = s.undo(id)?;                                    // "Rotate pages"
s.redo(id)?;

let doc = s.get(id).ok_or("gone")?;                         // &Document: queries, never edits
doc.can_undo(); doc.editable(); doc.allows_printing();
doc.text_blocks(page); doc.page_boxes(page); doc.security_summary();

let report = s.save_report(id, false)?; // incremental output plus signatures_affected
let bytes = report.bytes;              // confirm affected signatures before writing
let bytes = s.save_full_bytes(id)?;   // full rewrite, garbage-collected
s.mark_saved(id, bytes.clone(), path)?;
```

`Edit` is one enum of 75 variants — every undoable change in the product, from
`RotatePages` to `AddAnnotation`, `SetFieldValue`, `ApplyRedactions` and `AddWatermark`. Adding a
feature means adding a variant and handling it in `apply`, which is what keeps undo, the dirty
flag, autosave and the automation tools consistent for free.

`Session` also owns the operations that produce a *new* document rather than editing one:
`create_blank`, `create_from_images`, `create_from_text`, `combine`, `extract`, `split`,
`split_by_size`, `reduced_bytes`, `optimized_bytes`, `print_pdf`, `export_data`, and the signing
family (`sign`, `sign_with_timestamp`, `timestamp_document`, `embed_ltv`).

Signed full rewrites are refused. `save_report(id, false)` refuses modifying incremental saves
under DocMDP P=1. An explicitly chosen copy uses `save_report(id, true)`, preserves the signed
revision and certification dictionaries, and does not claim certification compliance. The UI
confirms later changes for every signature; automation requires `acknowledgeSignatures: true`
and the CLI `--acknowledge-signatures`. No-op saves return an empty affected list. Recovery
drafts keep signed documents; a restored draft reports its signatures again when saved.

Opening admission lives in `memory`: wasm uses a 1 GiB budget; desktop defaults to 4 GiB and
accepts `PDFCRAFT_MEMORY_MIB` (64–16384). Above three quarters of the budget, opening uses light
read-only mode; above the budget, opening refuses without replacing existing documents or
recovery files. The bounded preflight estimates three input copies, declared pages, the largest
direct image and the cache/history reserves; compressed objects and indirect dimensions are not
decoded during preflight. Light mode keeps no decoded-image cache between renders, caps a page
raster and the canvas at 64 MiB, disables organize-grid thumbnails, and has no edit history.
Canvas rasters keep their per-kind allowances and share a 256 MiB total (least recently
requested first, on-screen pages never evicted); undo/redo share 512 MiB on desktop and 128 MiB
on the web, with source buffers charged once. Other UI textures (signature previews,
accessibility previews and the combine grid) are created elsewhere and are outside this total.
This admission estimate is not an allocator limit.

## Modules

| Module | What it holds |
|---|---|
| `commands` | The command registry: id, label, menu, shortcut, and a `Needs` precondition encoding document security. Menus, the ⌘K palette, shortcuts and the control channel all read it, so every surface agrees on what exists and when it is enabled |
| `catalog` | Acrobat's "All tools" information architecture as data — groups, sections, icons, hues, and an `Availability` of `Ready`, `Planned(milestone)` or `Provider`. The UI renders what is *not* built yet, honestly |
| `actions` | PDF action dispatch (go-to, URI, named, set-layer-state, JavaScript) |
| `links` | Link creation, properties, and finding URLs in page text |
| `memory` | The platform memory budget: opening admission (normal, light read-only, refused) and the canvas and history byte allowances |
| `js` | The bridge to `pdfcraft-js`. `forms` never depends on `js` directly; the engine injects it, so a build without JavaScript is possible |
| `xfa`, `ocr`, `compare`, `export` | Façades over the L3/L4 crates of the same names |

## Not done here

- **Rendering is still borrowed.** `pdfcraft-render` wraps `hayro` and inspects through `lopdf`
  (ADR-0004). The engine only sees `inspect() -> DocInfo`, `RenderPool` and `text::PageText`, and
  that API stays frozen while M2 replaces what is behind it.
- Opening, canvas raster and history budgets are enforced; other performance budgets remain unset.
- Some shipped features are still reachable only from the UI, not through an `automation` tool;
  `cargo xtask parity` lists them; closing that gap is tracked in the execution plan (D4).

`doc.text_edit_preview(page, block)` prepares a read-only page background with the selected
paragraph’s glyphs removed, preserving text positioning, images and colours. It shares the
working document’s render configuration and never enters the undo history.
