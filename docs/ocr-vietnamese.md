# Vietnamese OCR feasibility — PDF-03

Assessment: 2026-10-23, phase B following the lead's licence decision. Earlier phase A findings below are historical evidence; the decision in this section supersedes their EasyOCR licensing gate.

## Decision and implementation

Use **EasyOCR `latin_g2` for native Vietnamese recognition**, alongside the existing ocrs detector. English keeps the ocrs recognizer. The lead explicitly accepts **Apache-2.0 (EasyOCR project licence; no separate weights licence published)**: the EasyOCR project publishes these weights itself in its own GitHub releases under its project licence. The user may veto this interpretation before release. VietOCR's separate weight licence remains unresolved and it is not integrated.

Original release asset: https://github.com/JaidedAI/EasyOCR/releases/download/v1.3/latin_g2.zip, SHA-256 `29f1920c493378da65a59793fb70e7e190504662b6bed57ab26f4067eb5f3769`. Converted RTen artifact: **15,402,880 bytes**, SHA-256 `921eb4282df957a236785877efe31b75756c1b65918cd647e4fb63ef5767e899`, left at `/home/phamt/erpcons-workspace/ocr-eval/out/text-recognition-vi.rten` for the lead to host. Neither checkpoint nor converted model is committed. `ATTRIBUTION.toml` records both hashes, the source, licence decision and pinned conversion source.

`tools/ocr-convert/convert.py` is an offline, documented Python export tool, outside CI and the product build. `cargo xtask models` obtains the converted artifact from **`PDFCRAFT_OCR_VI_MODEL_URL` (HTTPS)** and checks the repository's pinned SHA-256. With the variable unset it skips the optional download; a fresh installation lists Vietnamese as unavailable with a translated reason. Runtime model discovery uses `PDFCRAFT_MODELS` and the existing package locations, with no runtime downloads. Installing Vietnamese needs the existing detector plus `text-recognition-vi.rten`; it does not require the English recognizer. Corrupt or incompatible installed models produce a load/recognition error.

The recognizer preserves the exact 351-character latin_g2 classifier order (including the euro sign and combining dot), converts RGB/RGBA to grayscale, pads detected line bounds by 20% of line height (2–64 pixels, clipped to the image), resizes proportionally to height 64 with Catmull–Rom filtering, and normalizes pixels to [-1, 1]. Width is at least 16 and at most 4096; oversized lines return an actionable error rather than allocating without a limit. Greedy CTC collapses repeated labels, removes blank, retains case and aligns word boxes using emitted timestep spans. Output is normalized to NFC. This path handles rendered PDF page rotation; it does not implement automatic classification of sideways scanned writing or perspective correction.

The reference phrase's initial **C→c is a model error, not a decoding bug**: independent PyTorch and RTen decoders agreed on all 200 original lines. No forced capitalization, dictionary correction or lowercasing is applied. The decoder regression explicitly distinguishes C/c and repeated letters separated by blank. The prior measured corpus remains **1.063830% NFC CER**, 9.079 ms/line mean, 67.37 MiB recognition-only peak RSS. Those numbers describe isolated recognition, not whole-page OCR.

Desktop language selection includes **Tiếng Việt** and defaults to `vi` for a Vietnamese interface. English remains selectable if Vietnamese is not installed. Automation language schemas and all recognizer callers select the corresponding model; `ocr_status` reports availability and a reason per language. Invisible Vietnamese text uses the already bundled OFL Inter TrueType subset, Identity-H and ToUnicode, with real Inter advances. Each stamp gets a private font resource so repeated OCR preserves prior mappings. No additional font is shipped. The original image remains unchanged and the existing batch transaction remains one Undo step.

**Web OCR stays blocked.** The PDF-05 1-GiB document budget has no measured/reserved full-page OCR allowance, cancellable bounded worker or approved download budget. A 14.69-MiB recognizer and a 67-MiB recognition-only measurement do not establish page detector/render peak memory. Native recognition code and its added dependencies are excluded from wasm; the existing translated web-unavailable message is retained.

## Phase B validation (2026-10-23)

All Cargo checks ran through `bash ~/wsl-fork.sh pdfcraft` in the WSL clone, holding `/tmp/erpcons-build.lock`, with `CARGO_TARGET_DIR=target/gate`, release LTO disabled and 16 release codegen units. Logs are `/home/phamt/ci-logs/pdf03-*.log`. No workspace-wide gate or formatter write was run.

Model-present tests used `PDFCRAFT_MODELS=/home/phamt/erpcons-workspace/ocr-eval/out`. Font-dependent scans used a minimal `CRAFT_FONTS_DIR=/home/phamt/erpcons-workspace/ocr-eval/test-fonts` manifest referencing only the pinned, attributed non-CJK Noto Sans Regular described in phase A. No font was copied into this repository. The test prints a skip reason if either required input is missing; Unicode layer and decoder tests always run. Screenshots used `PDFCRAFT_SYSTEM_FONTS=0`.

The integrated ocrs detector + EasyOCR recognizer read **all five requested strings exactly**, including the initial uppercase C, on an upright image and a counter-rotated image with `/Rotate 90`: **0.000000 CER** in both release and debug. This is a clean synthetic acceptance result, not a replacement for the earlier 200-line, case-sensitive 1.063830% recognition-only measurement. The 5% acceptance ceiling allows detector-crop/raster differences and the known model-level C/c ambiguity; no capitalization correction is applied. The saved PDF extracts exactly the recognized characters in reading order (visual line separators are joined as spaces when comparing with `OcrPage::text`). Every one of the 146 inventory characters also round-trips in a model-independent test.

| Check | Release | Debug |
| --- | --- | --- |
| Engine `--test ocr_vietnamese --test ocr_edit ocr` | 3 Vietnamese + 4 existing OCR tests pass | Same 7 pass |
| OCR `--lib vietnamese` | 2 pass | 2 pass |
| OCR `--lib reads_rendered_text` | 1 English regression passes | 1 passes |
| OCR `--lib rejects_hostile_image_shapes_without_panicking` | 1 passes | 1 passes |
| Automation `--test automation ocr` | 5 pass | 5 pass |
| UI `--lib ocr_ui::tests` | 3 pass | 3 pass |
| UI `--test ocr` | 5 pass | 5 pass |
| UI `--lib bundled_catalogs_are_consistent` | 1 passes | 1 passes |
| Edit `--lib inter_added_text` | 1 passes | 1 passes |
| xtask `assets::tests` | 13 pass | 13 pass |

Native `cargo clippy --locked --release -p pdfcraft-ocr -p pdfcraft-edit -p pdfcraft-engine -p pdfcraft-automation -p pdfcraft-ui-egui -p xtask --all-targets -- -D warnings` passed (``Finished `release` profile [optimized] target(s) in 20.23s``). `cargo clippy --locked -p pdfcraft-web --target wasm32-unknown-unknown -- -D warnings` passed (``Finished `dev` profile [optimized + debuginfo] target(s) in 7.23s``).

`cargo xtask assets --write` and `cargo xtask assets` passed: `assets: ok (262 repository assets, 6 bundled by dependencies, 23 fetched at build time)`. `cargo xtask layers` passed: `OK: 35 crates, no layering violations.` The asset gate initially found two stale pre-existing demo hashes. Both files match their declared contributor-original source at commit `64e6398abb4280d99fadf69ea21fbaab5ffcc8d1`; only their manifest hashes were corrected, with no demo content changes. `ATTRIBUTION.md` was regenerated by xtask.

Delivery checks: `cargo xtask models` with the optional URL unset fetched/verified the existing models and reported `models: text-recognition-vi.rten unavailable (PDFCRAFT_OCR_VI_MODEL_URL is unset)`. Supplying the upstream ZIP URL as the converted-model URL deliberately failed SHA-256 validation and installed no Vietnamese model. A separate process with `PDFCRAFT_MODELS` pointing to an absent directory reported Vietnamese unavailable and skipped its model-dependent automation test. The UI still offered the language, displayed the Vietnamese installation reason and disabled recognition.

Regression sensitivity was verified by actually restoring the old WinAnsi OCR application path: `vietnamese_ocr_every_precomposed_letter_round_trips` failed with question marks in extracted text, then passed after restoring Unicode embedding. Restoring the original recognition entry point made `rejects_hostile_image_shapes_without_panicking` fail with `attempt to multiply with overflow`; the bounded guarded implementation passed after restoration. Both temporary mutations were restored, and final native clippy passed afterward.

`pdfcraft-cli run --script` reopened the saved synthetic PDF, rendered both pages and extracted all five strings with their original accents. Both page PNGs have SHA-256 `1602fb99fd228d609852a97ab29b44601c4762cc5a777192dd9bd6dfa959602c`. The model-backed test separately checks unchanged compressed image streams, unchanged image references/geometry, pixel equality, rotated word placement, repeated subset stamps and one-step batch Undo/Redo. The picker and unavailable-model screenshots were visually inspected; all artifacts remain outside the repo at `/home/phamt/erpcons-workspace/ocr-eval/artifacts/`.

The conversion script was executed and reproduced the pinned RTen hash exactly. Its disposable venv was removed; the complete evaluation directory is about 244 MiB, below 3 GB, and `/mnt/g` remained above 8 GB free before downloads. No native installer was built for this scoped task. The lead still needs to host the converted bytes over HTTPS, set `PDFCRAFT_OCR_VI_MODEL_URL` for packaging and smoke-test installers. Existing packaging takes the whole fetched-model folder on Windows, Linux and macOS. The live converted-model hosting URL cannot be checked until that hosting step exists. Web OCR remains disabled pending a measured full-page RAM/download design.

## Evidence and reproduction

Candidate material was downloaded only to `/home/phamt/erpcons-workspace/ocr-eval/`. It is not a build input, committed fixture, or published asset. No external artwork or fonts were imported into the repository. Sizes below distinguish downloaded bytes from upstream advertised model sizes. Figures explicitly marked measured come from this experiment; other memory, implementation-time and speed figures are planning estimates.

The required set contains 146 code points: 12 vowel bases in both cases, each with no tone or one of five tones, plus Đ/đ. This covers ordinary ASCII vowels as well as ă â ê ô ơ ư, all tones, and both cases. Other ASCII consonants are present in the inspected dictionaries. Reproduce the coverage check with Python's standard library:

```python
import unicodedata
from pathlib import Path
required = set("Đđ")
for base in "aăâeêioôơuưyAĂÂEÊIOÔƠUƯY":
    for tone in ("", "\u0301", "\u0300", "\u0309", "\u0303", "\u0323"):
        required.update(unicodedata.normalize("NFC", base + tone))
assert len(required) == 146
for name in ("latin_dict.txt", "ppocrv5_latin_dict.txt"):
    entries = Path(name).read_text(encoding="utf-8").splitlines()
    missing = sorted(required - set(entries))
    print(name, len(entries), len(missing), "".join(missing))
```

Dictionary order is significant for CTC decoding. An NFC pass only composes characters actually emitted; it cannot restore a tone missing from the model's classes. Neither Latin dictionary contains combining marks, so decomposed output is not an escape hatch.

## Second pass: EasyOCR and VietOCR (empirical)

This pass corrects the first pass's incomplete candidate selection. EasyOCR and VietOCR both have complete Vietnamese character inventories. EasyOCR also **runs successfully on the fork's RTen 0.26.0**; neither character coverage nor unsupported operators is a reason to reject it. At the end of that pass, the remaining shipping gate was the checkpoint's redistribution licence; the lead has since resolved that gate as recorded above. VietOCR's separate weight-hosting licence is likewise unverified. These are findings about the evidence inspected, not claims that either author prohibits commercial use.

### Provenance and character inventories

Inspected immutable source revisions:

- [EasyOCR `363afb184047ce452e436f4224f3098422df872e`](https://github.com/JaidedAI/EasyOCR/tree/363afb184047ce452e436f4224f3098422df872e). `easyocr/config.py` includes `vi` in `latin_lang_list`; `easyocr/easyocr.py` selects `recognition_models['gen2']['latin_g2']` for that family. This is **latin_g2**, not the English-only `english_g2`, and not the older `latin_g1`.
- [VietOCR `fe8c3a7fc714aec57ab81cec844eb3adf0c1636c`](https://github.com/pbcquoc/vietocr/tree/fe8c3a7fc714aec57ab81cec844eb3adf0c1636c). Both `config/vgg-seq2seq.yml` and `config/vgg-transformer.yml` inherit `config/base.yml`'s Vietnamese vocabulary.

| Candidate | Dictionary | Missing required characters | Code licence | Weight-file licence evidence |
| --- | --- | --- | --- | --- |
| EasyOCR `latin_g2` | `characters` in the pinned `easyocr/config.py`; CTC blank is an additional class | **None: 146/146**, including Đ/đ, Ỵ/ỵ, all tone/case combinations | Apache-2.0; verified in the pinned source and v1.3 tag | **Apache-2.0 (EasyOCR project licence; no separate weights licence published)**, accepted by the lead after this pass; user may veto before release |
| VietOCR `vgg_seq2seq` | 229-character `vocab` in the pinned `config/base.yml`, plus decoding control tokens | **None: 146/146** | Apache-2.0 | Separately hosted `.pth`; no weight licence declaration found; **unverified** |
| VietOCR `vgg_transformer` | Same vocabulary | **None: 146/146** | Apache-2.0 | Same unresolved issue |

The coverage comparison uses the generated NFC set described above, including the 12 ordinary ASCII vowel characters. It has 146 total characters, or 134 non-ASCII characters **including** Đ/đ. This explicit construction avoids an ambiguous count of “134 letters plus Đ/đ.” The missing-character lists for all three candidates are empty. EasyOCR's alphabet contains a combining dot and one euro sign; all 351 characters are distinct. Preserve the exact classifier order rather than sorting it.

Weight licensing was checked separately from code licensing. The [official EasyOCR model hub](https://www.jaided.ai/easyocr/modelhub/) links to the [v1.3 release](https://github.com/JaidedAI/EasyOCR/releases/tag/v1.3). Its `latin_g2.zip` contains exactly one file, `latin_g2.pth`, without a licence or model card. The release notes describe generation-two models but do not state terms for the checkpoint. The [project LICENSE at that tag](https://github.com/JaidedAI/EasyOCR/blob/v1.3/LICENSE) is Apache-2.0: this is evidence in favour of permissive availability, but this assessment does **not** assume that licensing code automatically licenses separately distributed learned parameters. No primary-source statement explicitly applying it to this weight file was located. A third-party mirror's Apache label cannot resolve the original author's grant. This was the pass-2 scope question, not a finding that the weights have a non-commercial licence. The lead subsequently accepted the project licence for its own release weights; phase B uses that explicit decision. The company's hosted-service terms are not used to assign restrictions to its GitHub weights either.

For VietOCR, the [README licence statement](https://github.com/pbcquoc/vietocr/blob/fe8c3a7fc714aec57ab81cec844eb3adf0c1636c/README.md#license) explicitly describes the library. The two pinned configurations point to `https://vocr.vn/data/vietocr/vgg_seq2seq.pth` and `https://vocr.vn/data/vietocr/vgg_transformer.pth`, outside the source repository. [Upstream issue #140](https://github.com/pbcquoc/vietocr/issues/140) asks exactly whether those weights may be redistributed commercially and under which licence; it had **no replies** when inspected. No issue, email or other message was sent as part of this task. A model-specific author statement or release/model card covering the exact checkpoint would resolve the gate. Do not create an `ATTRIBUTION.toml` entry claiming Apache-2.0 for unverified weights.

### Download and export measurements

All experiments live in `/home/phamt/erpcons-workspace/ocr-eval/`, outside both PdfCraft working trees. `/mnt/g` had 57 GB free before the initial candidate download, 56 GB before installing/exporting, 50 GB after inference and 40 GB at the final check, all above the required 8 GB. The evaluation directory peaked at approximately 1.2 GB, below 3 GB; after removing the throw-away venv and bootstrap script it contains 175,311,636 bytes (about 167 MiB). No CUDA packages or persistent pip cache were installed. No model, font, rendered fixture or screenshot was added to the repository.

| File | Bytes | SHA-256 / acquisition |
| --- | ---: | --- |
| EasyOCR original `latin_g2.zip` | 14,284,936 | `29f1920c493378da65a59793fb70e7e190504662b6bed57ab26f4067eb5f3769` |
| Extracted `latin_g2.pth` | 15,406,141 | `aaa95be1c4a9cb3496879bed7c520886ce1164f89e026f0c54488394e74e8c55` |
| Exported `latin_g2.onnx` | 15,399,219 | `984f6d55005940f2e69489cd1770ddb1d3c39b7ed0b02f5764eb02e9f2413efe` |
| Converted `latin_g2.rten` | 15,402,880 | `921eb4282df957a236785877efe31b75756c1b65918cd647e4fb63ef5767e899` |
| Exact EasyOCR alphabet, UTF-8 without newline | 712 | `f6f61866a083d7b06e4fa0af37cccbd2229ceaf382ee5c438a322dbb7381d4db` |
| VietOCR `vgg_seq2seq.pth` | 89,575,371 | Official server HEAD `Content-Length`; **not downloaded**, no local hash claimed |
| VietOCR `vgg_transformer.pth` | 151,815,373 | Official server HEAD `Content-Length`; **not downloaded**, no local hash claimed |

The EasyOCR checkpoint's MD5 is `469869130aad1a34e8f9086f4262bc59`, matching the pinned upstream config. Load used `torch.load(..., weights_only=True, map_location='cpu')`, stripping the `module.` prefix. The model is VGG with 1 input channel, 256 output channels, two 256-hidden bidirectional LSTM stages and a classifier with 352 classes (351 alphabet entries plus blank). No detector, dictionary correction, beam search, quantization or language-model postprocessing was used.

Throw-away Python 3.12 environment: PyTorch `2.5.1+cpu`, torchvision `0.20.1+cpu`, ONNX `1.17.0`, rten-convert `0.22.0`, Pillow `12.3.0`, NumPy `2.5.2`. The available Python converter releases stop at 0.22.0; installation of a nonexistent 0.26.0 converter failed, then 0.22.0 was used. **The Rust inference runtime is exactly 0.26.0**, not 0.22.0. `scratch/Cargo.lock` pins its dependencies.

ONNX export used opset 17, float32 input `[1,1,64,width]`, dynamic width and dynamic output time. Direct export initially failed on `AdaptiveAvgPool2d((None,1))` with dynamic dimensions. In the scratch copy only, replaced:

```python
self.AdaptiveAvgPool(visual_feature.permute(0, 3, 1, 2))
```

with the mathematically equivalent:

```python
visual_feature.permute(0, 3, 1, 2).mean(dim=3, keepdim=True)
```

The pooled dimension is the convolutional feature height. This changes no trained parameters. Input remains normalized grayscale `(pixel / 255 - 0.5) / 0.5`. Export calls the upstream model with `text=None`. `rten-convert latin_g2.onnx` produced the `.rten` file successfully. The ONNX operator inventory is:

`Add, Concat, Constant, ConstantOfShape, Conv, Gather, LSTM, MatMul, MaxPool, ReduceMean, Relu, Reshape, Shape, Squeeze, Transpose, Unsqueeze`.

**RTen 0.26 supported every operator exercised on all 200 variable-width inputs**, including the bidirectional LSTMs. No missing operator or runtime failure occurred. Exact greedy decoded strings agreed with a separate PyTorch reference on **200/200 lines**. This establishes decoding agreement on this corpus; it is not a tensor-level error bound or a guarantee for every possible shape.

### Synthetic corpus and Rust inference results

Rendered **200** contributor-generated text lines, 1,222 NFC source characters:

1. The five requested strings, verbatim: `Cộng hòa xã hội chủ nghĩa Việt Nam`, `Nguyễn`, `đường`, `Ở`, `ữ`.
2. 146 separate `b<char>n` strings, one per required character, in Unicode order. These are deliberately dictionary-free and cover every tone and case, including Ỵ/ỵ.
3. 49 dictionary-free lines, each three random words of length 3–6 from the sorted required set, using Python `random.Random(20261023)`.

Font: **Noto Sans Regular**, the non-CJK Latin family, from craft-fonts reference `b7dfde98c6d9d5db761c405732f9ef6d58bec192/fonts/noto-sans/NotoSans-Regular.ttf`. Author: The Noto Project Authors; OFL-1.1, verified against its adjacent `OFL.txt` and craft-fonts attribution. Its original-source pin is [notofonts/notofonts.github.io `28b15b4b43b7bed62b5cf6e6b0b5ff5846270535`](https://github.com/notofonts/notofonts.github.io/tree/28b15b4b43b7bed62b5cf6e6b0b5ff5846270535/fonts/NotoSans/unhinted/ttf). SHA-256: `f3961a9cde016d41a4879aecda1474d3a36d6bf54fa0e4643de029cc2248b0e8`. Its cmap was checked for all 146 characters. No Source family, Noto CJK or Adobe-authored font was used.

Rendering: Pillow grayscale, black text on white, font size 40 pixels, tight ink bounds plus 8 pixels padding on every side; resize proportionally to height 64 with bicubic resampling, width at least 16, then normalize to float32. Saved PNGs and little-endian `.f32` tensors remain outside the repo. The manifest SHA-256 is `e4d4eeeba1f74ed12abe149dc5bf348ceb5e27d45914bf49cf6247afada3b245`.

The scratch Rust binary loads `.rten` once, reads the tensors, calls `Model::run_one`, takes the class argmax at each timestep, collapses repeated classes and removes blank. It writes source/prediction/elapsed milliseconds per line. CER is total Levenshtein edits divided by total source Unicode scalar count after **NFC on both strings**, including spaces and preserving case. No lowercasing, punctuation removal or word dictionary was used.

| Subset | Lines | NFC source characters | Edits | CER | Exact lines |
| --- | ---: | ---: | ---: | ---: | ---: |
| Requested strings | 5 | 47 | 1 | 2.127660% | 4/5 |
| Exhaustive character contexts | 146 | 438 | 2 | 0.456621% | 144/146 |
| Random dictionary-free lines | 49 | 737 | 10 | 1.356852% | 40/49 |
| **All** | **200** | **1,222** | **13** | **1.063830%** | **188/200** |

Requested-string outputs:

| Source | RTen output |
| --- | --- |
| `Cộng hòa xã hội chủ nghĩa Việt Nam` | `cộng hòa xã hội chủ nghĩa Việt Nam` |
| `Nguyễn` | `Nguyễn` |
| `đường` | `đường` |
| `Ở` | `Ở` |
| `ữ` | `ữ` |

The exhaustive-context errors were `bín` → `bÍn` and `bỉn` → `bỈn`, both case confusions.

This passes the first pass's proposed **2% aggregate CER** ceiling for clean synthetic lines, chosen because even clean OCR occasionally confuses similar shapes. It does **not** pass an exact-match requirement for all five named strings. Do not silently lowercase the expected phrase or claim perfect Vietnamese recognition. Real scans, additional fonts, blur, rotated pages, detector crops and unusual tone/case contexts are still untested; the clean single-font result is a feasibility signal, not release accuracy evidence.

CPU: Intel Core Ultra 9 275HX, 8 logical CPUs exposed to WSL, no GPU. Rust release build, RTen defaults (multithreaded; process used about 304% CPU). A single timed recognition-only process reported:

- Load: **491.32 ms**.
- Mean: **9.079 ms/line**, median **6.767 ms**, p95 **17.707 ms**; first inference included, I/O excluded from each line timer.
- `/usr/bin/time -v` peak RSS: **68,984 KiB = 67.37 MiB**; wall time **2.33 s**, exit status **0**, no swaps.
- Input widths range from 53 to 746 pixels with the rendered lines; this is neither a fixed-width throughput benchmark nor a full-page measurement. The document, page renderer, ocrs detector, extraction and Undo are absent from the process. Shared-machine scheduling and the default worker pool make timings approximate; do not interpret 67 MiB as the application or wasm memory requirement.

### VietOCR export path and costs (not executed)

VietOCR was **not exported, converted or run** because its checkpoint licence does not pass the task's prerequisite. Its CER, RTen operator support, peak RAM and CPU milliseconds are **not measured**. The sizes above are HTTP metadata, not local model measurements. Do not present the project's GPU benchmark as this experiment's result.

The pinned source provides a credible export path, with no new native dependency in the eventual product:

- `vgg_seq2seq`: export the VGG CNN plus bidirectional GRU encoder once; return encoded memory and initial decoder hidden state. Export one attention/embedding/GRU/linear decoder step separately. In Rust, start at SOS (1), run the decoder on the previous token and state, take argmax, retain the new hidden state, and stop at EOS (2) or a hard output-token cap. Keep encoder memory across steps. The pinned decoder already exposes `forward_encoder` / `forward_decoder`; verify memory layout rather than assuming Transformer-compatible shapes.
- `vgg_transformer`: export CNN plus `forward_encoder`, and a second graph for `forward_decoder(tgt, memory)`. Grow a bounded token prefix, make the causal mask for its length, take the final-position argmax and stop at EOS. Upstream does not supply an incremental KV cache, so this straightforward export recomputes prefix attention. Dynamic target lengths, positional-encoding slices, masks and PyTorch attention lowering need explicit ONNX/RTen validation. A single traced full Python generation loop is unsuitable because its stopping length would be frozen.
- Shared preprocessing: RGB, height 32, upstream bounded width 32–512 and its normalization/padding policy; vocab control-token indices must follow `Vocab`, followed by NFC normalization. An RTen operator inventory cannot be claimed before export; GRU, attention primitives and shape/mask operations are the areas to exercise. Use Rust error returns, checked dimensions and a decode cap, not an unbounded loop.

The [upstream README benchmark](https://github.com/pbcquoc/vietocr/blob/fe8c3a7fc714aec57ab81cec844eb3adf0c1636c/README.md) reports full-sequence precision 0.8701/12 ms for seq2seq and 0.8800/86 ms for Transformer on a GTX 1080 Ti, described as the 10m dataset. This is Vietnamese-oriented evidence but neither a reproducible benchmark on these synthetic lines nor CPU/RTen performance. For planning only, reserve roughly 250–600 MiB per seq2seq recognizer worker and 400–1,000 MiB for Transformer plus decode buffers, excluding page/detector memory. Tentative CPU envelopes are 50–500 ms and 200–3,000 ms per line respectively; these are deliberately broad **unmeasured estimates**, not acceptance numbers. Measure before choosing either. Allow 2–4 engineer-weeks for two-graph export, bounded Rust decoding and integration after licence verification; Transformer conversion may expose additional work.

### Recommendation, web budget and phase B gate

**Prefer EasyOCR `latin_g2` over training a new ocrs model if its weight redistribution licence is confirmed.** It has complete coverage, a roughly 15.4-MB RTen recognizer, measured low clean-text CER and demonstrated execution on exactly RTen 0.26, with no C/C++ product dependency. This supersedes the first pass's recommendation to start with new model training. Keep ocrs detection and supply an EasyOCR-specific grayscale crop/CTC adapter. A 1–3 engineer-week integration estimate remains reasonable for word placement, cancellation, model packaging, automation/UI, Unicode text embedding and regression coverage; export has now been demonstrated rather than assumed.

**Do not proceed with phase B in this change:** permissive redistribution of the exact weights has not been proven from a primary-source weight grant. `AGENTS.md` §1.2/§1.5 requires provable licensing before adding or fetching shipped assets. EasyOCR's project Apache licence may cover its release checkpoint, but this report leaves that scope uncertainty explicit for the lead rather than recording an unsupported licence in `ATTRIBUTION.toml`. VietOCR needs the same confirmation and still needs empirical export/inference. Neither is rejected for missing characters. No product code or asset attribution changed, and no request was sent upstream.

For web, the 15.4-MB recognizer and 67-MiB native recognition-only peak make EasyOCR a plausible future candidate. They **do not establish the PDF-05 whole-document budget**: wasm operator execution, model download/load peak, detector/page allocations, worker copies, memory admission and cancellation remain unmeasured. Keep the existing translated web-unavailable message. There is no demonstrated safe wasm OCR budget in this pass.

Once the licence gate is resolved, phase B still needs Unicode Type0/Identity-H plus ToUnicode text (Helvetica/WinAnsi is inadequate), per-language model selection, the Vietnamese desktop default and catalogues, the existing automation path, saved extraction equality, image preservation and one-step batch Undo. No rotated-page recognition, saved-PDF round trip, Undo, UI screenshot or wasm execution was tested here; those belong to that implementation and are not implied by line recognition success.

### Local reproduction and checks

Scratch files retained for inspection: `export_easy.py`, `easy_model/`, `render_lines.py`, `check_easy_torch.py`, `metrics.py`, `scratch/{Cargo.toml,Cargo.lock,src/main.rs}`, `lines/manifest.tsv`, `easy-results-direct.tsv` and source/licence snapshots in `/home/phamt/erpcons-workspace/ocr-eval/`. They are not application code or release inputs. `easy-results-direct.tsv` preserves the measured run. The throw-away Python environment is removed after evaluation; reinstall pinned packages to rerun export/reference. The retained model, inputs and release Rust binary can be used without Python.

```sh
# Build runs in WSL and holds the required machine-wide lock.
bash ~/wsl-fork.sh pdfcraft "flock -w 7200 /tmp/erpcons-build.lock env CARGO_TARGET_DIR=/home/phamt/erpcons-workspace/ocr-eval/target CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 cargo build --release --manifest-path /home/phamt/erpcons-workspace/ocr-eval/scratch/Cargo.toml > ~/ci-logs/ocr-eval-rust-build.log 2>&1"
/usr/bin/time -v ~/erpcons-workspace/ocr-eval/target/release/ocr-eval > ~/ci-logs/ocr-eval-rten-direct.log 2>&1
python3 ~/erpcons-workspace/ocr-eval/metrics.py > ~/ci-logs/ocr-eval-metrics.log
```

Actual result lines:

```text
    Finished `release` profile [optimized] target(s) in 34.29s
completed_lines=200
all errors 13 characters 1222 CER 1.063830% exact_lines 188 / 200
torch_rten_equal_lines 200 200
```

The initial dynamic-pooling export failure and unavailable converter-version install are recorded above and resolved in the successful experiment. Logs are under `/home/phamt/ci-logs/ocr-eval-*.log`. The only repository changes are this document and an `ERPCONS-CHANGES.md` bullet; product release/debug tests and native/wasm clippy are not applicable to a documentation-only change. No product tests were added and no skipped product test is counted as a pass.

## First-pass option 1: PaddleOCR recognition exported to ONNX/RTen

Code: [PaddleOCR Apache-2.0 licence](https://github.com/PaddlePaddle/PaddleOCR/blob/dab3fe35379033fdcb2d0e9572fac0b36c9a9ebf/LICENSE). The official [Latin PP-OCRv5 weights release](https://huggingface.co/PaddlePaddle/latin_PP-OCRv5_mobile_rec/tree/ab2cd5cc5fa6309be2e5acdfe66eca2c2c127d57) declares Apache-2.0 independently of the code. Do not infer the licence of arbitrary third-party ONNX exports from that declaration. A future export needs a reproducible original-source conversion, pinned weights, dictionary, licence and output checksums.

Inspected PaddleOCR source commit: `dab3fe35379033fdcb2d0e9572fac0b36c9a9ebf`.

| Dictionary | Entries | Missing of 146 | SHA-256 |
| --- | ---: | ---: | --- |
| `ppocr/utils/dict/latin_dict.txt` | 185 | 102 | `8e6d4e3629788c35c31f7e530287d6147b549bb7a265bd6708bb281134429e2c` |
| `ppocr/utils/dict/ppocrv5_latin_dict.txt` | 836 | 90 | `ccbcc45730b3fbbd9050c5bc74db6a99067141ef1035e3d14889a84a6b9b1aff` |

The source [v5 training configuration](https://github.com/PaddlePaddle/PaddleOCR/blob/dab3fe35379033fdcb2d0e9572fac0b36c9a9ebf/configs/rec/PP-OCRv5/multi_language/latin_PP-OCRv5_mobile_rec.yml) explicitly selects the v5 Latin dictionary. Independently checked the official weights release's `config.json`: `PostProcess.character_dict` contains 836 entries and has exactly the same 90 missing characters. Thus this is not merely a mismatched repository dictionary.

Missing in the older Latin dictionary (all characters listed literally, in Unicode order):

`ÃÙĂăĨĩŨũƠơƯưẠạẢảẤấẦầẨẩẪẫẬậẮắẰằẲẳẴẵẶặẸẹẺẻẼẽẾếỀềỂểỄễỆệỈỉỊịỌọỎỏỐốỒồỔổỖỗỘộỚớỜờỞởỠỡỢợỤụỦủỨứỪừỬửỮữỰựỲỳỴỵỶỷỸỹ`

Missing in the v5 Latin dictionary:

`ẠạẢảẤấẦầẨẩẪẫẬậẮắẰằẲẳẴẵẶặẸẹẺẻẼẽẾếỀềỂểỄễỆệỈỉỊịỌọỎỏỐốỒồỔổỖỗỘộỚớỜờỞởỠỡỢợỤụỦủỨứỪừỬửỮữỰựỲỳỴỵỶỷỸỹ`

The older multilingual Latin configuration is PP-OCRv3; the inspected official model table offers general/English v4 models, not a separate Vietnamese-capable v4 Latin model. As a cross-check, `ppocr/utils/ppocr_keys_v1.txt` (the general dictionary used by the older Chinese family) lacks 123 required characters; `ppocr/utils/dict/ppocrv5_dict.txt` lacks 108. A larger Chinese/multilingual label is not evidence of Vietnamese coverage.

Size and accuracy: the upstream [recognition table](https://github.com/PaddlePaddle/PaddleOCR/blob/dab3fe35379033fdcb2d0e9572fac0b36c9a9ebf/docs/version3.x/module_usage/text_recognition.en.md) lists Latin v5 at 14 MB and 84.7% average recognition accuracy. The official Hugging Face tree lists approximately 7.97 MB of parameters; these are different distributions, not measured `.rten` sizes. The average is not a Vietnamese benchmark. On the requested strings, missing ộ, ễ, ườ, Ở and ữ already make exact recognition impossible. No local CER or page benchmark was run.

That table reports CPU recognition latency around 21.20 ms in normal mode and 5.32 ms in accelerated mode on upstream benchmark hardware. Those are crop/model measurements, not full-page timings or an RTen promise. Budget roughly 100–300 MiB for a recognizer worker plus detector/page buffers as an initial measurement envelope, not an observed peak. Long lines, batch size and detector resolution can substantially change it.

Runtime/integration: [RTen](https://github.com/robertknight/rten) is MIT OR Apache-2.0 and supports Rust native and wasm inference, but an ONNX export alone does not establish operator/shape compatibility with this fork's RTen 0.26. The existing ocrs pipeline supplies grayscale text lines; Paddle expects BGR/RGB three-channel, height-48 crops with its own normalization, padding, layout and CTC class order. A separate recognition adapter must preserve word boxes and spaces. Allow approximately 1–3 engineer-weeks for export verification, preprocessing, decoding, bounded allocations and integration after a suitable trained model exists. Unsupported operators would need implementation within scope; no operator failure was observed because no conversion was attempted. Character coverage rejects these weights first.

Wasm is architecturally possible with compatible operators and bounded crops; it is not demonstrated for this export. A small recognition download is encouraging but does not establish a safe whole-page memory budget. Verdict: reject these published weights for full Vietnamese; retraining is required.

## First-pass option 2: Tesseract `vie` through a safe library binding

Code: [Tesseract Apache-2.0](https://github.com/tesseract-ocr/tesseract/blob/main/LICENSE), with Leptonica's BSD-2-Clause dependency and a Rust wrapper such as [leptess](https://github.com/houqp/leptess) (MIT). The application would call a safe wrapper; handwritten unsafe FFI would violate repository policy. Audit the chosen binding/version and transitive native libraries before adopting it. No GPL/AGPL code or non-commercial weights are needed.

Official traineddata repositories carry their own Apache-2.0 licence. Downloaded and inspected:

| Repository and commit | `vie.traineddata` bytes | SHA-256 |
| --- | ---: | --- |
| [tessdata_fast](https://github.com/tesseract-ocr/tessdata_fast/tree/87416418657359cb625c412a48b6e1d6d41c29bd) | 531,275 | `79df64caf7bcfb2a27df5042ecb6121e196eada34da774956995747636d5bfa1` |
| [tessdata_best](https://github.com/tesseract-ocr/tessdata_best/tree/e12c65a915945e4c28e237a9b52bc4a8f39a0cec) | 12,435,550 | `b6b49293d95d0b6dbd8780174627e82c75be957b6f4ed9862155540d6b00bb45` |

For a reproducible static coverage check, read the traineddata little-endian component count and signed 64-bit offset table, then component 21 (`lstm-unicharset`), ending at the next present component. Compare the first whitespace-separated token of each unicharset entry with the required set above. Both files have 270 entries and lack **Ỵ and ỵ**; neither has combining-mark tokens to reconstruct them. Their remaining 144 required characters are present. This is a character-inventory inspection, not a successful recognition test. The task examples are representable here, but the complete alphabet contract is not met.

Accuracy evidence: [Tesseract's official comparison](https://tesseract-ocr.github.io/tessdoc/Data-Files.html) describes fast as the faster integer network and best as the slower, more accurate floating-point network on Google's evaluation data; this is not a Vietnamese CER guarantee. Both use LSTM-only engine mode 1. Test Vietnamese administrative scans, tone confusion and rotation independently before shipping. Missing Ỵ/ỵ requires additional training even if common examples pass.

Planning envelope: 50–200 MiB additional worker RAM plus page images, seconds to tens of seconds per 300-dpi text page on a desktop CPU, with best generally slower. These figures are unmeasured sizing estimates, not acceptance evidence. A 300-dpi A4 RGBA page alone is about 33 MiB; grayscale and preprocessing copies add to it. Serial page processing and cancellation are necessary.

Native packaging is substantial: Windows MSVC-compatible libtesseract/Leptonica libraries and dependent DLLs (or static builds), Linux matching distribution libraries or bundled runtime, macOS arm64/x86_64 builds, install-name/rpath handling and signing. The traineddata path must be resolved in each installer layout. Handle initialization failures without losing the open document. Allow 2–4 engineer-weeks for three-platform packaging and engine/UI/text-layer integration, excluding alphabet retraining and quality calibration.

No wasm path is proposed: this option is a native binding with C/C++ dependencies, outside phase B's conditions. Recommendation: useful fallback prototype under a separately approved native-backend task; not an implementation candidate for this task.

## First-pass option 3: Fine-tune the ocrs recognition model

Training code declares MIT OR Apache-2.0 in [pyproject.toml](https://github.com/robertknight/ocrs-models/blob/068934f1725959b734edef025b13c46ddf784326/pyproject.toml). Existing recognition weights are **CC-BY-SA-4.0**, not permissive software-licensed weights (already allowed and attributed by this repository). A derivative checkpoint must preserve applicable attribution/share-alike obligations; do not label it Apache-2.0 just because the training code is. Training from scratch on contributor-owned/openly licensed data offers a route to new weights with a deliberate licence, subject to the data's terms.

The [training guide](https://github.com/robertknight/ocrs-models/blob/068934f1725959b734edef025b13c46ddf784326/docs/training.md) uses a 24-GB NVIDIA A10G and documents recognition training, fine-tuning and ONNX export. Its existing HierText input is about 3.6 GB compressed and is not a Vietnamese dataset. The current English classifier needs expanded output classes and new labeled examples; changing `ALPHABET` alone is invalid. Preserve existing class indices when transferring weights and initialize/retrain added classifier outputs.

Proposed data: 0.5–2 million synthetic line crops using proven open Vietnamese-capable fonts from craft-fonts, licensed Vietnamese source text and contributor-original layout/noise/blur/rotation generation, then thousands of consented or synthetic held-out page crops with rare uppercase tones and dot-below y deliberately represented. Split source text/fonts/pages before augmentation to prevent leakage. Audit all font, image and text licences; never use system/proprietary fonts or unverified public document scans. Synthetic and real-scan quality remain an open risk.

Planning estimate, not a training quote: 1–3 weeks for data/licence work, 24–100 GPU-hours on a 16–24 GB GPU for initial experiments, then 2–4 weeks for evaluation, hard cases and integration; several iterations may increase this. This shared 6-GB CPU machine is not an appropriate training host. No ready Vietnamese checkpoint or accuracy evidence was found in the inspected ocrs-models release. Require held-out CER and rare-character recall, not just an English benchmark.

Deployment would keep the existing small CRNN/detector architecture; estimate recognition weights in the tens of MB and 100–300 MiB worker memory plus images, to be measured after export. Native page speed should be in the same order as the current ocrs recognizer, but no trained Vietnamese model exists here to time. The runtime supports wasm in principle; this says nothing about whole-document peak RAM. This was the first pass's preferred future RTen route. The second-pass EasyOCR measurements now make a ready recognizer preferable if its weight licence is verified; training remains an alternative when that cannot be resolved.

## Shared integration requirements and web decision

Current `crates/ocr/src/lib.rs` writes `/PCHelv` Helvetica/WinAnsi via `helvetica_width` and `win_ansi`; most Vietnamese letters would become `?`. Normalizing recognized text to NFC is necessary but does not solve encoding. Any later implementation must use the existing `pdfcraft-fonts` Unicode subset/CID embedding facilities (`cid_embed`, Type0/Identity-H with ToUnicode), correct glyph advances, per-word transforms and invisible rendering mode 3. Bundle no new fonts here. Retain the original image streams and run the entire page batch as one undoable engine transaction; expose language selection through automation as well as the desktop picker. Default `vi` when the selected UI locale is Vietnamese; show the unavailable reason until its optional model is installed.

PDF-05's current `crates/engine/src/memory.rs` admits web documents against 1 GiB, with 256 MiB canvas cache and 128 MiB history allowances; desktop defaults to 4 GiB. These are existing document budgets, not free OCR worker reserves. OCR adds model buffers, rendered pages, detector tensors and potentially overlapping copies. There is no measured OCR peak, reservation/admission path, cancellable bounded worker or verified download allowance for any Vietnamese candidate. Keep the current translated web-unavailable message. Do not claim that a roughly 8–14 MB recognizer makes OCR fit PDF-05.

Before enabling a future model, measure native and wasm downloads, load time, peak memory and page latency with single-page sequential processing. Account for the detector too, reject work before allocation when the document plus OCR would exceed the budget, and test low-memory cancellation/recovery. For model acceptance, render the requested phrase, Nguyễn, đường, Ở and ữ, add Ỵ/ỵ and every tone/case, and test a rotated page. An initial held-out NFC character error rate ceiling of 2% on clean synthetic text is a proposed quality target, not a measured result; require exact recognition of the named regression strings and exact saved-text extraction of whatever was recognized. Check original image bytes and single-step Undo independently. Skip model/font-dependent tests explicitly when inputs are absent, as the current OCR tests do.

## Validation and limits

First pass: dictionary set comparisons, official v5 release dictionary check, SHA-256 hashes and character-inventory inspection of two official Tesseract files. Second pass: complete EasyOCR/VietOCR vocabulary comparisons; EasyOCR export, conversion and 200-line inference in Rust RTen 0.26 with PyTorch decoding agreement, CER, timing and process peak-RSS measurement. Detailed commands, actual result lines, sources and remaining limits are in the second-pass section above.

Phase A results above precede the lead-approved licence interpretation. Phase B results are recorded in the validation section above; native release/debug checks and wasm clippy pass.

## Tóm tắt phase A (lịch sử, trước quyết định giấy phép của lead)

Đợt khảo sát thực nghiệm thứ hai tìm được **EasyOCR `latin_g2` đủ 146/146 ký tự** và chạy thành công bằng Rust RTen 0.26. Với 200 dòng tổng hợp dùng Noto Sans (không phải Noto CJK), CER sau NFC là **1,063830%**, trung bình **9,079 ms/dòng**, RAM đỉnh riêng bộ nhận dạng khoảng **67 MiB**. Kết quả giải mã khớp PyTorch 200/200 dòng; câu yêu cầu bị đổi chữ C đầu thành c, bốn chuỗi còn lại đúng hoàn toàn. VietOCR cũng đủ bộ ký tự nhưng giấy phép weights tải ngoài chưa rõ, chưa chạy thử mô hình này.

Đề xuất ưu tiên EasyOCR khi xác minh được quyền phân phối weights. Apache-2.0 của mã nguồn chưa được coi là bằng chứng riêng cho checkpoint trong đợt này; cần làm rõ phạm vi giấy phép trước khi thêm tài sản theo AGENTS.md. Vì vậy chưa làm phase B, không đổi OCR desktop/web. Các số đo dòng chữ chưa chứng minh ngân sách RAM toàn trang trên web; vẫn giữ thông báo chưa hỗ trợ. Khi tích hợp phải có lớp chữ Unicode/ToUnicode, kiểm tra trích xuất, trang xoay, ảnh gốc và Undo một bước.

## Tóm tắt triển khai phase B

Theo quyết định giấy phép của kỹ sư phụ trách, dùng EasyOCR `latin_g2` cho OCR tiếng Việt trên bản máy tính; giữ bộ phát hiện ocrs và bộ nhận dạng tiếng Anh hiện tại. Mô hình được tải tùy chọn qua `PDFCRAFT_OCR_VI_MODEL_URL`, kiểm tra SHA-256, không đưa vào Git. Khi chưa cài mô hình, giao diện báo rõ bằng ngôn ngữ đã chọn. Lớp chữ ẩn dùng Inter nhúng theo tập con và ToUnicode để giữ nguyên dấu khi tìm kiếm, sao chép và lưu. Kết quả chuẩn hóa NFC, không tự đổi hoa/thường; lỗi C→c trong khảo sát là lỗi nhận dạng của mô hình. Web vẫn chưa bật OCR vì chưa chứng minh được ngân sách RAM toàn trang. Người dùng có thể phủ quyết cách hiểu giấy phép weights trước khi phát hành.

Đã kiểm tra release và debug: nhận dạng đúng toàn bộ năm chuỗi yêu cầu trên hai trang (có trang xoay), CER 0%; chữ Unicode lưu/mở lại nguyên dấu, ảnh gốc không đổi và batch hoàn tác một bước. Clippy native/wasm, kiểm tra tài sản và phân lớp đều đạt. Đã xem ảnh giao diện khi có và khi thiếu mô hình. Lead cần đưa tệp `.rten` lên HTTPS, cấu hình URL cho đóng gói và kiểm tra bộ cài trước khi phát hành.
