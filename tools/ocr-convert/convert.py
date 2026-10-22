"""Offline conversion of the pinned EasyOCR latin_g2 checkpoint. See README.md."""
import argparse
import hashlib
import importlib
import pathlib
import subprocess
import sys
import types
import zipfile

import torch

REVISION = "363afb184047ce452e436f4224f3098422df872e"
ZIP_SHA256 = "29f1920c493378da65a59793fb70e7e190504662b6bed57ab26f4067eb5f3769"
WEIGHTS_SHA256 = "aaa95be1c4a9cb3496879bed7c520886ce1164f89e026f0c54488394e74e8c55"
RTEN_SHA256 = "921eb4282df957a236785877efe31b75756c1b65918cd647e4fb63ef5767e899"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=pathlib.Path, required=True, help="EasyOCR checkout at the pinned revision")
    parser.add_argument("--weights", type=pathlib.Path, required=True, help="official latin_g2.zip")
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()
    sources = {
        "easyocr/config.py": "9c36d6c674fe32b05bda27192211ff6b63bf22f00b8439d40d33b35e3e2c1f8d",
        "easyocr/model/modules.py": "6e22171d7b63ca993a4db4070c0e1b69498e700f4eb0cab5ae290115e7b55a4d",
        "easyocr/model/vgg_model.py": "946c718a6a907a1c5ff26750fd5b29ad5a3b7eb850253d94a3cbd7e11ffbb137",
    }
    for relative, expected in sources.items():
        if digest(args.source / relative) != expected:
            raise ValueError(f"source differs from EasyOCR {REVISION}: {relative}")
    if digest(args.weights) != ZIP_SHA256:
        raise ValueError("upstream ZIP SHA-256 mismatch")
    args.out.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(args.weights) as archive:
        checkpoint = archive.read("latin_g2.pth")
    if hashlib.sha256(checkpoint).hexdigest() != WEIGHTS_SHA256:
        raise ValueError("checkpoint SHA-256 mismatch")
    weights = args.out / "latin_g2.pth"
    weights.write_bytes(checkpoint)

    # Import only recognition modules, avoiding EasyOCR's optional detector dependencies.
    package = types.ModuleType("easy_model")
    package.__path__ = [str(args.source / "easyocr" / "model")]
    sys.modules["easy_model"] = package
    model_module = importlib.import_module("easy_model.vgg_model")
    config = types.ModuleType("easy_config")
    config.__file__ = str(args.source / "easyocr" / "config.py")
    exec(compile((args.source / "easyocr" / "config.py").read_text(), "config.py", "exec"), config.__dict__)
    alphabet = config.recognition_models["gen2"]["latin_g2"]["characters"]

    def forward(self, image, text):
        features = self.FeatureExtraction(image)
        # Mean over feature height is equivalent to AdaptiveAvgPool2d((None, 1))
        # and exports with dynamic width in the legacy ONNX exporter.
        features = features.permute(0, 3, 1, 2).mean(dim=3, keepdim=True).squeeze(3)
        return self.Prediction(self.SequenceModeling(features).contiguous())

    model_module.Model.forward = forward
    torch.set_num_threads(1)
    model = model_module.Model(1, 256, 256, len(alphabet) + 1)
    state = torch.load(weights, map_location="cpu", weights_only=True)
    model.load_state_dict({key.removeprefix("module."): value for key, value in state.items()})
    model.eval()

    class Export(torch.nn.Module):
        def __init__(self, model):
            super().__init__()
            self.m = model

        def forward(self, image):
            return self.m(image, None)

    onnx = args.out / "latin_g2.onnx"
    torch.onnx.export(
        Export(model), torch.zeros(1, 1, 64, 512), onnx,
        input_names=["image"], output_names=["logits"],
        dynamic_axes={"image": {3: "width"}, "logits": {1: "time"}}, opset_version=17,
    )
    subprocess.run(["rten-convert", str(onnx)], check=True)
    converted = args.out / "latin_g2.rten"
    print(f"{digest(converted)}  {converted} ({converted.stat().st_size} bytes)")
    if digest(converted) != RTEN_SHA256:
        raise ValueError("export differs from pinned artifact; do not host without review and revalidation")


if __name__ == "__main__":
    main()
