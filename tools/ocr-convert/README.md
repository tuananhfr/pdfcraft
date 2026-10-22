# Vietnamese recognizer conversion (manual, outside CI)

Product inference uses only Rust RTen 0.26. This Python tool is a maintainer export step, never a product dependency or build step. Work outside the repository; keep the evaluation directory under 3 GB and check disk space first. No model or font binaries are committed.

Use Python 3.12, CPU-only PyTorch 2.5.1+cpu and torchvision 0.20.1+cpu (from the official PyTorch CPU index), ONNX 1.17.0, rten-convert 0.22.0 and packaging 26.1 in a disposable venv. These are the versions used for the pinned export. The converter version differs from the product runtime version. Disable pip's cache; remove the venv after exporting.

Obtain a clean checkout of https://github.com/JaidedAI/EasyOCR at `363afb184047ce452e436f4224f3098422df872e` and the official checkpoint ZIP from https://github.com/JaidedAI/EasyOCR/releases/download/v1.3/latin_g2.zip. The script verifies the the three imported source files against pinned hashes and ZIP/checkpoint hashes before loading with `weights_only=True`. The upstream architecture and exact alphabet are read from that checkout; no architecture code or model file is vendored here.

```sh
python /path/to/pdfcraft/tools/ocr-convert/convert.py \
  --source /path/to/EasyOCR --weights /path/to/latin_g2.zip --out /path/to/out
```

The only architecture adjustment replaces height-only adaptive average pooling with the equivalent mean to support variable-width ONNX export. Opset 17, batch 1, grayscale, height 64, dynamic width, float32. RTen supports all exported operators including bidirectional LSTMs. See `docs/ocr-vietnamese.md` for measured accuracy and limits.

Expected RTen file: **15,402,880 bytes**, SHA-256 **921eb4282df957a236785877efe31b75756c1b65918cd647e4fb63ef5767e899**. The script fails on a different hash; review and revalidate a changed export before changing the pin. Host that file over HTTPS and set `PDFCRAFT_OCR_VI_MODEL_URL` when running `cargo xtask models`. That command installs it as `text-recognition-vi.rten` and independently verifies the repository pin. Without the URL it skips this optional download. At runtime `PDFCRAFT_MODELS` selects the installed model directory; there is no runtime network access.

Weights: **Apache-2.0 (EasyOCR project licence; no separate weights licence published)**, per the lead's explicit licence decision. The original project publishes the checkpoint in its own GitHub release. The user may veto this interpretation before release. Full attribution, upstream ZIP hash and converted artifact hash are in `ATTRIBUTION.toml`; shipped model licence text is fetched beside the model.
