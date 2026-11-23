"""Create a portable ZIP with consistent names and verify every staged file."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import zipfile

parser=argparse.ArgumentParser()
parser.add_argument('--stage',type=Path,required=True)
parser.add_argument('--exe',type=Path,required=True)
parser.add_argument('--out',type=Path,required=True)
args=parser.parse_args()
recipes=json.loads(subprocess.run([str(args.exe),'--voice-selections'],capture_output=True,encoding='utf-8',check=True).stdout)
(args.stage/'VoiceLibrary/精选声音.json').write_text(json.dumps(recipes,ensure_ascii=False,indent=2),encoding='utf-8')
temporary=args.out.with_suffix('.zip.part')
files=sorted(p for p in args.stage.rglob('*') if p.is_file())
assert len([p for p in files if p.suffix=='.onnx'])==1
notice=args.stage/'resources/notifications'
manifest=json.loads((notice/'manifest.json').read_text(encoding='utf-8'))
assert len(manifest['clips'])==24
for clip in manifest['clips']:
    assert hashlib.sha256((notice/clip['file']).read_bytes()).hexdigest()==clip['sha256']
with zipfile.ZipFile(temporary,'w',compression=zipfile.ZIP_DEFLATED,compresslevel=6) as archive:
    for path in files:archive.write(path,path.relative_to(args.stage.parent).as_posix())
with zipfile.ZipFile(temporary) as archive:
    assert archive.testzip() is None
    for path in files:assert archive.read(path.relative_to(args.stage.parent).as_posix())==path.read_bytes()
temporary.replace(args.out)
print(f'Verified {len(files)} portable files; one model; ZIP {args.out.stat().st_size:,} bytes')
