"""Bake six voices x four state clips. Build-time synthesis, never event-time."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import wave

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / 'resources/notifications'
TEXTS = {'done': '任务完成了。', 'subagent': '子任务完成了。',
         'error': '出了点问题，请检查。', 'ask': '需要你确认一下。'}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, default=ROOT / 'AgentVoice.exe')
    parser.add_argument('--reuse', action='store_true')
    args = parser.parse_args()
    OUTPUT.mkdir(parents=True, exist_ok=True)
    exe = args.exe.resolve()
    if not exe.is_file():
        subprocess.run(['cargo','build','--release','--locked'],cwd=ROOT,check=True)
        exe = ROOT/'.build/rust/x86_64-pc-windows-msvc/release/AgentVoice.exe'
    recipes = json.loads(subprocess.run([str(exe), '--voice-selections'], capture_output=True, encoding='utf-8', check=True).stdout)
    config = json.loads(subprocess.run([str(exe), '--export-config'], capture_output=True, encoding='utf-8', check=True).stdout)
    batch, records = [], []
    for recipe in recipes:
        profile = dict(config['profiles'][0], voice=recipe['voice'], mixer=recipe['mixer'], rate=0, pitch=0, volume=100)
        speaker = recipe['voice'].rsplit(':', 1)[1]
        for event, text in TEXTS.items():
            path = OUTPUT / f'{speaker}-{event}.wav'
            if not args.reuse or not path.exists():
                batch.append((profile, text, str(path)))
            records.append(dict(voice=recipe['voice'], event=event, text=text, file=path.name))
    if batch:
        result = subprocess.run([str(exe), '--render-batch-json'], input=json.dumps(batch, ensure_ascii=False),
            capture_output=True, encoding='utf-8', timeout=900)
        assert result.returncode == 0, result.stderr
    for record in records:
        path = OUTPUT / record['file']
        with wave.open(str(path), 'rb') as wav:
            rate = wav.getframerate()
            assert rate == 24000 and wav.getnchannels() == 1 and wav.getsampwidth() == 2
            pcm = wav.readframes(wav.getnframes())
        values = struct.unpack('<' + 'h' * (len(pcm)//2), pcm)
        peak = max(map(abs, values))
        threshold = max(8, peak * .002)
        first = next(i for i, v in enumerate(values) if abs(v) > threshold)
        # Remove only initial digital padding, keep 60ms before quiet onset.
        start = max(0, first - int(rate * .06))
        with wave.open(str(path), 'wb') as wav:
            wav.setparams((1, 2, rate, 0, 'NONE', 'not compressed'))
            wav.writeframes(pcm[start*2:])
        record.update(bytes=path.stat().st_size, sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
            sample_rate=rate, seconds=round((len(values)-start)/rate, 3), onset_ms=round((first-start)*1000/rate, 2))
    manifest = dict(format='agent-voice-notifications-v1', model='Kokoro v1.1-zh INT8',
        synthesis='Build time only. Event playback never loads the model.', clips=records)
    (OUTPUT/'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps(dict(clips=len(records),bytes=sum(r['bytes'] for r in records),max_onset_ms=max(r['onset_ms'] for r in records)),ensure_ascii=False),flush=True)


if __name__ == '__main__': main()
