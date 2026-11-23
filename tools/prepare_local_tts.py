"""Fetch one pinned, CPU-only Chinese TTS engine; no package installation.

Development/build helper only. End users get these files in the portable ZIP.
"""
from pathlib import Path
import hashlib
import json
import tarfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "resources/local-tts"
DOWNLOADS = ROOT / ".build/downloads"
PACKAGES = [
    ("vits-icefall-zh-aishell3.tar.bz2",
     "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-icefall-zh-aishell3.tar.bz2",
     "ab468db3a3308cdd861495e0db2f25d79418a0c00639f74944c7cdf5dd8c6ec1"),
    ("sherpa-onnx-v1.12.20-win-x64-shared.tar.bz2",
     "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.12.20/sherpa-onnx-v1.12.20-win-x64-shared.tar.bz2",
     "2aa9c11f13d6a20308afedcaa8c04a678a1aaebc206b7609e4df7d7910582f56"),
]


def download(url, path):
    request = urllib.request.Request(url, headers={"User-Agent": "AgentVoice-build"})
    temporary = path.with_suffix(path.suffix + ".part")
    with urllib.request.urlopen(request, timeout=45) as response, temporary.open("wb") as output:
        while chunk := response.read(1024 * 1024):
            output.write(chunk)
    temporary.replace(path)


def prepare():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    DOWNLOADS.mkdir(parents=True, exist_ok=True)
    for name, url, expected in PACKAGES:
        package = DOWNLOADS / name
        if not package.is_file():
            download(url, package)
        if hashlib.sha256(package.read_bytes()).hexdigest() != expected:
            raise ValueError(f"Checksum mismatch: {package}")
        if name.startswith("vits-"):
            prefix = "vits-icefall-zh-aishell3/"
            names = ["model.onnx", "lexicon.txt", "tokens.txt", "speakers.txt",
                     "number.fst", "date.fst", "phone.fst", "new_heteronym.fst"]
        else:
            prefix = "sherpa-onnx-v1.12.20-win-x64-shared/lib/"
            names = ["sherpa-onnx-c-api.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll"]
        with tarfile.open(package) as archive:
            for filename in names:
                member = archive.getmember(prefix + filename)
                if not member.isfile():
                    raise ValueError(f"Expected regular file: {member.name}")
                (OUTPUT / filename).write_bytes(archive.extractfile(member).read())
            if not name.startswith("vits-"):
                header = "sherpa-onnx-v1.12.20-win-x64-shared/include/sherpa-onnx/c-api/c-api.h"
                dependency = ROOT / ".build/dependencies"
                dependency.mkdir(parents=True, exist_ok=True)
                (dependency / "sherpa-onnx-1.12.20-c-api.h").write_bytes(archive.extractfile(header).read())

    licenses = {
        "LICENSE-sherpa-onnx.txt": "https://raw.githubusercontent.com/k2-fsa/sherpa-onnx/v1.12.20/LICENSE",
        "LICENSE-icefall.txt": "https://raw.githubusercontent.com/k2-fsa/icefall/master/LICENSE",
    }
    for name, url in licenses.items():
        path = OUTPUT / name
        if not path.is_file():
            download(url, path)
    # The wheel is only an upstream license source. Do not install Python
    # packages or ship the wheel/Python runtime to users.
    if not (OUTPUT / "LICENSE-onnxruntime.txt").is_file():
        wheel = DOWNLOADS / "onnxruntime-1.17.1-cp312-cp312-win_amd64.whl"
        if not wheel.is_file():
            download("https://files.pythonhosted.org/packages/81/e5/55699490257383a0a579b43eeea545680edc8f956f83fde1e16c5fd2359e/onnxruntime-1.17.1-cp312-cp312-win_amd64.whl", wheel)
        with zipfile.ZipFile(wheel) as zipped:
            for name in zipped.namelist():
                base = Path(name).name
                if base == "LICENSE":
                    (OUTPUT / "LICENSE-onnxruntime.txt").write_bytes(zipped.read(name))
                elif "ThirdPartyNotices" in base:
                    (OUTPUT / "NOTICE-onnxruntime-third-party.txt").write_bytes(zipped.read(name))
    # App-local Microsoft redistributables: no installer or administrator step.
    candidates = sorted(Path("C:/Program Files (x86)/Microsoft Visual Studio").glob(
        "*/BuildTools/VC/Redist/MSVC/*/x64/Microsoft.VC143.CRT"))
    if not candidates:
        candidates = sorted(Path("C:/Program Files/Microsoft Visual Studio").glob(
            "*/*/VC/Redist/MSVC/*/x64/Microsoft.VC143.CRT"))
    for name in ["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"]:
        if candidates:
            (OUTPUT / name).write_bytes((candidates[-1] / name).read_bytes())
        elif not (OUTPUT / name).is_file():
            raise ValueError(f"Build Tools redistributable not found: {name}")
    notice = (
        "Agent Voice local Chinese speech\n"
        "One model: icefall VITS AISHELL3, 174 speakers, CPU only.\n"
        "Model: https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models\n"
        "Dataset: https://www.openslr.org/93/\n"
        "Runtime: sherpa-onnx 1.12.20, ONNX Runtime (CPU).\n"
        "No Python, Node, CUDA or HTTP service is required at runtime.\n"
        "Microsoft VC runtime DLLs are distributed app-local with the native runtime.\n"
        "The large training/normalization archive rule.far is intentionally not shipped;\n"
        "the small number/date/phone FST files provide text normalization.\n"
    )
    (OUTPUT / "NOTICE.txt").write_text(notice, encoding="utf-8")
    files = {p.name: {"bytes": p.stat().st_size,
                       "sha256": hashlib.sha256(p.read_bytes()).hexdigest()}
             for p in sorted(OUTPUT.iterdir()) if p.is_file() and p.name != "manifest.json"}
    manifest = {"model": "icefall-vits-aishell3", "speakers": 174,
                "runtime": "sherpa-onnx-1.12.20-cpu-x64", "threads": 2,
                "files": files, "archives": {n: h for n, _, h in PACKAGES}}
    (OUTPUT / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"Prepared one offline model and CPU runtime: {sum(x['bytes'] for x in files.values()):,} bytes")


if __name__ == "__main__":
    prepare()
