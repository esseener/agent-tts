"""Prepare one pinned Kokoro v1.1-zh INT8 model and a native CPU runtime.

Build helper only. The portable application never invokes Python/downloads.
Keep the legacy resource directory intact for already running older releases.
"""
import hashlib
import json
from pathlib import Path
import shutil
import tarfile

from prepare_local_tts import ROOT, DOWNLOADS, PACKAGES, download

ARCHIVE = 'kokoro-int8-multi-lang-v1_1.tar.bz2'
SHA = '259c9c8bc906d6078e4e0570421885dabb68b539c2f970fcbb890a58eeaf3e9c'
OUTPUT = ROOT / 'resources/kokoro'


def prepare():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    DOWNLOADS.mkdir(parents=True, exist_ok=True)
    package = DOWNLOADS / ARCHIVE
    if not package.is_file():
        download('https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/' + ARCHIVE, package)
    assert hashlib.sha256(package.read_bytes()).hexdigest() == SHA, 'Kokoro archive checksum mismatch'
    prefix = 'kokoro-int8-multi-lang-v1_1/'
    names = ['model.int8.onnx','voices.bin','tokens.txt','lexicon-us-en.txt','lexicon-zh.txt',
             'phone-zh.fst','date-zh.fst','number-zh.fst','LICENSE']
    with tarfile.open(package) as archive:
        for member in archive.getmembers():
            if not member.name.startswith(prefix) or not member.isfile():
                continue
            relative = member.name[len(prefix):]
            if relative not in names and not relative.startswith(('dict/','espeak-ng-data/')):
                continue
            target = OUTPUT / ('LICENSE-kokoro.txt' if relative == 'LICENSE' else relative)
            assert target.resolve().is_relative_to(OUTPUT.resolve())
            target.parent.mkdir(parents=True,exist_ok=True)
            target.write_bytes(archive.extractfile(member).read())
    assert hashlib.sha256((OUTPUT/'model.int8.onnx').read_bytes()).hexdigest() == '05d793700b0b507f760c183c0e39bdd6a97b43d32fdc7bf7b54a5179f1b11222'
    runtime_name, url, checksum = PACKAGES[1]
    package = DOWNLOADS / runtime_name
    if not package.is_file():download(url,package)
    assert hashlib.sha256(package.read_bytes()).hexdigest() == checksum
    with tarfile.open(package) as archive:
        for name in ['sherpa-onnx-c-api.dll','onnxruntime.dll','onnxruntime_providers_shared.dll']:
            (OUTPUT/name).write_bytes(archive.extractfile('sherpa-onnx-v1.12.20-win-x64-shared/lib/'+name).read())
    # Reuse already prepared files when possible; a clean build does not need
    # to download or prepare the retired voice model.
    legacy = ROOT/'resources/local-tts'
    candidates = sorted(Path('C:/Program Files (x86)/Microsoft Visual Studio').glob('*/BuildTools/VC/Redist/MSVC/*/x64/Microsoft.VC143.CRT'))
    if not candidates:candidates=sorted(Path('C:/Program Files/Microsoft Visual Studio').glob('*/*/VC/Redist/MSVC/*/x64/Microsoft.VC143.CRT'))
    for name in ['msvcp140.dll','vcruntime140.dll','vcruntime140_1.dll']:
        source=legacy/name if (legacy/name).is_file() else candidates[-1]/name if candidates else None
        if source is None:raise RuntimeError('Visual Studio C++ redistributable missing: '+name)
        shutil.copy2(source,OUTPUT/name)
    licenses = {
        'LICENSE-sherpa-onnx.txt':'https://raw.githubusercontent.com/k2-fsa/sherpa-onnx/v1.12.20/LICENSE',
        'LICENSE-onnxruntime.txt':'https://raw.githubusercontent.com/microsoft/onnxruntime/v1.17.1/LICENSE',
        'NOTICE-onnxruntime-third-party.txt':'https://raw.githubusercontent.com/microsoft/onnxruntime/v1.17.1/ThirdPartyNotices.txt',
        'LICENSE-espeak-ng.txt':'https://raw.githubusercontent.com/espeak-ng/espeak-ng/master/COPYING',
        'LICENSE-cppjieba.txt':'https://raw.githubusercontent.com/yanyiwu/cppjieba/master/LICENSE',
    }
    for name,url in licenses.items():
        if not (OUTPUT/name).is_file():
            if (legacy/name).is_file():shutil.copy2(legacy/name,OUTPUT/name)
            else:download(url,OUTPUT/name)
    (OUTPUT/'NOTICE.txt').write_text(
        'Agent Voice offline speech: Kokoro-82M v1.1-zh (2025-02-26), INT8, 24 kHz.\n'
        'One ONNX model; 100 Chinese voices and 3 English voices share its weights.\n'
        'Model: https://huggingface.co/hexgrad/Kokoro-82M-v1.1-zh (Apache-2.0)\n'
        'Export: https://huggingface.co/csukuangfj/kokoro-int8-multi-lang-v1_1\n'
        'Runtime: sherpa-onnx 1.12.20, ONNX Runtime CPU, 2 inference threads.\n'
        'Front-end includes cppjieba dictionaries and eSpeak NG data (see licenses).\n'
        'No Python, Node, CUDA, online service or per-voice model download required.\n',encoding='utf-8')
    files = {p.relative_to(OUTPUT).as_posix():dict(bytes=p.stat().st_size,sha256=hashlib.sha256(p.read_bytes()).hexdigest())
             for p in sorted(OUTPUT.rglob('*')) if p.is_file() and p.name != 'manifest.json'}
    manifest = dict(model='kokoro-v1.1-zh-int8',parameters=82000000,sample_rate=24000,speakers=103,
                    chinese_speakers=100,runtime='sherpa-onnx-1.12.20-cpu-x64',threads=2,files=files,
                    archives={ARCHIVE:SHA,runtime_name:checksum})
    (OUTPUT/'manifest.json').write_text(json.dumps(manifest,indent=2),encoding='utf-8')
    print(json.dumps(dict(resource_bytes=sum(f['bytes'] for f in files.values()),files=len(files),model_bytes=(OUTPUT/'model.int8.onnx').stat().st_size)))


if __name__=='__main__':prepare()
