# Vietnamese OCR feasibility — PDF-03

Assessment: 2026-10-23. Phase A complete; phase B is blocked by model character coverage, not implementation size. No product code, shipped assets, model downloads in the repository, or web behaviour changed.

## Decision

None of the evaluated ready-made recognizers meets the complete Vietnamese alphabet requirement **and** the phase B requirement of a permissively licensed model runnable through RTen without new C/C++ dependencies. Do not add `vi` to the current English recognizer: changing its alphabet cannot create trained output classes.

Recommend a Vietnamese-trained **ocrs CRNN** as the long-term Rust/RTen route, retaining the existing detector. This requires a separate model-training and validation effort before application integration. Tesseract `vie` is the quickest native prototype if C/C++ packaging becomes acceptable, but its inspected character set also needs correction before claiming complete coverage. Neither is a ready phase B option today. The Paddle Latin models are unsuitable without retraining, even if conversion succeeds.

This is a bounded evaluation of the named model families, not a claim that no Vietnamese ONNX model exists anywhere. No conversion or inference success is claimed for Paddle; the dictionary check rejects these candidates before that expense.

## Evidence and reproduction

Candidate material was downloaded only to `/home/phamt/erpcons-workspace/ocr-eval/`. It is not a build input, committed fixture, or published asset. No external artwork or fonts were imported. Sizes below distinguish downloaded bytes from upstream advertised model sizes; memory, implementation time and unmeasured speed figures are planning estimates.

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

## Option 1: PaddleOCR recognition exported to ONNX/RTen

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

## Option 2: Tesseract `vie` through a safe library binding

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

## Option 3: Fine-tune the ocrs recognition model

Training code declares MIT OR Apache-2.0 in [pyproject.toml](https://github.com/robertknight/ocrs-models/blob/068934f1725959b734edef025b13c46ddf784326/pyproject.toml). Existing recognition weights are **CC-BY-SA-4.0**, not permissive software-licensed weights (already allowed and attributed by this repository). A derivative checkpoint must preserve applicable attribution/share-alike obligations; do not label it Apache-2.0 just because the training code is. Training from scratch on contributor-owned/openly licensed data offers a route to new weights with a deliberate licence, subject to the data's terms.

The [training guide](https://github.com/robertknight/ocrs-models/blob/068934f1725959b734edef025b13c46ddf784326/docs/training.md) uses a 24-GB NVIDIA A10G and documents recognition training, fine-tuning and ONNX export. Its existing HierText input is about 3.6 GB compressed and is not a Vietnamese dataset. The current English classifier needs expanded output classes and new labeled examples; changing `ALPHABET` alone is invalid. Preserve existing class indices when transferring weights and initialize/retrain added classifier outputs.

Proposed data: 0.5–2 million synthetic line crops using proven open Vietnamese-capable fonts from craft-fonts, licensed Vietnamese source text and contributor-original layout/noise/blur/rotation generation, then thousands of consented or synthetic held-out page crops with rare uppercase tones and dot-below y deliberately represented. Split source text/fonts/pages before augmentation to prevent leakage. Audit all font, image and text licences; never use system/proprietary fonts or unverified public document scans. Synthetic and real-scan quality remain an open risk.

Planning estimate, not a training quote: 1–3 weeks for data/licence work, 24–100 GPU-hours on a 16–24 GB GPU for initial experiments, then 2–4 weeks for evaluation, hard cases and integration; several iterations may increase this. This shared 6-GB CPU machine is not an appropriate training host. No ready Vietnamese checkpoint or accuracy evidence was found in the inspected ocrs-models release. Require held-out CER and rare-character recall, not just an English benchmark.

Deployment would keep the existing small CRNN/detector architecture; estimate recognition weights in the tens of MB and 100–300 MiB worker memory plus images, to be measured after export. Native page speed should be in the same order as the current ocrs recognizer, but no trained Vietnamese model exists here to time. The runtime supports wasm in principle; this says nothing about whole-document peak RAM. This is the preferred future RTen route because it avoids an unrelated crop/model adapter, but it does not supply a runnable model for phase B now.

## Shared integration requirements and web decision

Current `crates/ocr/src/lib.rs` writes `/PCHelv` Helvetica/WinAnsi via `helvetica_width` and `win_ansi`; most Vietnamese letters would become `?`. Normalizing recognized text to NFC is necessary but does not solve encoding. Any later implementation must use the existing `pdfcraft-fonts` Unicode subset/CID embedding facilities (`cid_embed`, Type0/Identity-H with ToUnicode), correct glyph advances, per-word transforms and invisible rendering mode 3. Bundle no new fonts here. Retain the original image streams and run the entire page batch as one undoable engine transaction; expose language selection through automation as well as the desktop picker. Default `vi` only when the selected UI locale is Vietnamese and a valid model is available.

PDF-05's current `crates/engine/src/memory.rs` admits web documents against 1 GiB, with 256 MiB canvas cache and 128 MiB history allowances; desktop defaults to 4 GiB. These are existing document budgets, not free OCR worker reserves. OCR adds model buffers, rendered pages, detector tensors and potentially overlapping copies. There is no measured OCR peak, reservation/admission path, cancellable bounded worker or verified download allowance for any Vietnamese candidate. Keep the current translated web-unavailable message. Do not claim that a roughly 8–14 MB recognizer makes OCR fit PDF-05.

Before enabling a future model, measure native and wasm downloads, load time, peak memory and page latency with single-page sequential processing. Account for the detector too, reject work before allocation when the document plus OCR would exceed the budget, and test low-memory cancellation/recovery. For model acceptance, render the requested phrase, Nguyễn, đường, Ở and ữ, add Ỵ/ỵ and every tone/case, and test a rotated page. An initial held-out NFC character error rate ceiling of 2% on clean synthetic text is a proposed quality target, not a measured result; require exact recognition of the named regression strings and exact saved-text extraction of whatever was recognized. Check original image bytes and single-step Undo independently. Skip model/font-dependent tests explicitly when inputs are absent, as the current OCR tests do.

## Validation and limits

Performed dictionary set comparisons, checked the actual official v5 release dictionary, downloaded the two official Tesseract model files, computed their SHA-256 hashes and inspected their character inventories. Candidate files remain outside the repository. No inference, conversion, RAM profiling, screenshots, training or recognition quality tests were performed. No Rust package was touched, so release/debug Cargo tests and native/wasm clippy are not applicable to this documentation-only result. Phase B tests, translations, model attribution entries and Unicode-layer changes are deliberately not implemented because no evaluated ready model passes its prerequisite.

## Tóm tắt tiếng Việt

Chưa có phương án khảo sát nào đáp ứng đồng thời đủ chữ/dấu tiếng Việt, chạy bằng RTen và không thêm C/C++. Dictionary Latin PP-OCRv5 thiếu 90/146 ký tự cần kiểm tra; không thể sửa bằng cách đổi alphabet hay chuẩn hóa NFC. Tesseract `vie` gần đạt hơn nhưng thiếu Ỵ/ỵ và cần thư viện C/C++. Đề xuất huấn luyện mô hình ocrs tiếng Việt trong công việc riêng, có dữ liệu/phông hợp lệ và kiểm định dấu đầy đủ. Đợt này chỉ bàn giao khảo sát; giữ nguyên OCR desktop và thông báo chưa hỗ trợ trên web. Khi có mô hình đạt yêu cầu, phải dùng lớp chữ Unicode/ToUnicode, kiểm tra trích xuất, ảnh gốc và Undo một bước.
