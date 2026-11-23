"""Verify the local Hojo trial and render same-line Hojo/Kokoro WAVs."""

import argparse
from array import array
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import wave

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / 'docs/voice-samples-hojo'
TEXT = '好啦，任务完成了，来看看结果吧。'
HOJO_PREFIX = 'local:hojo-40m:'


def sha256(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def check_audio(path):
    with wave.open(str(path), 'rb') as audio:
        rate = audio.getframerate()
        assert (audio.getnchannels(), audio.getsampwidth(), rate) == (1, 2, 24000), path
        frames = audio.getnframes()
        samples = array('h', audio.readframes(frames))
    assert len(samples) == frames and frames >= rate // 2, f'Incomplete or short audio: {path}'
    peak = max(abs(min(samples)), abs(max(samples)))
    assert 327 < peak < 32767, f'Silent or clipped audio: {path}'
    return {'sample_rate': rate, 'seconds': round(frames / rate, 3), 'sha256': sha256(path)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--exe', type=Path, default=ROOT / 'AgentVoice-Hojo-试用.exe')
    args = parser.parse_args()
    exe = args.exe.resolve(strict=True)
    models = ROOT / 'resources/hojo'
    manifest = json.loads((models / 'manifest.json').read_text(encoding='utf-8'))
    for name, expected in manifest['files'].items():
        path = models / name
        assert path.stat().st_size == expected['bytes'] and sha256(path) == expected['sha256'], name

    temp = Path(os.environ['LOCALAPPDATA']) / 'Temp/opencode'
    temp.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='agent-voice-hojo-', dir=temp) as folder:
        home = Path(folder)

        def cli(*arguments, data=None):
            result = subprocess.run([str(exe), '--home', folder, *arguments], input=data,
                                    encoding='utf-8', capture_output=True, timeout=90)
            if result.returncode:
                raise RuntimeError(f'{arguments[0]} failed: {result.stderr.strip()}')
            return result.stdout

        voices = dict(json.loads(cli('--voices')))
        trial = {key for key in voices if key.startswith(HOJO_PREFIX)}
        assert trial == {HOJO_PREFIX + speaker for speaker in manifest['voices']}
        selections = json.loads(cli('--voice-selections'))
        assert len(selections) == 6 and all(s['voice'].startswith('local:kokoro-') for s in selections)
        profile = json.loads(cli('--export-config'))['profiles'][0]
        assert profile['voice'] == 'local:kokoro-v1.1:zf_001', 'Trial changed the default voice'
        settings = dict(profile, rate=0, pitch=0, volume=100, mixer=selections[0]['mixer'])

        samples = []
        for gender, baseline, gender_label in [('f', selections[0], '女声'), ('m', selections[1], '男声')]:
            choices = [(baseline['voice'], baseline['title'], 'Kokoro v1.1-zh INT8')]
            choices.extend(
                (HOJO_PREFIX + speaker, f'Hojo {gender_label} {speaker.rsplit("_", 1)[1]}', manifest['model'])
                for speaker in manifest['voices'] if f'_{gender}_' in speaker
            )
            for voice, title, model in choices:
                path = home / f'{voice.rsplit(":", 1)[1]}.wav'
                data = json.dumps([dict(settings, voice=voice), TEXT], ensure_ascii=False)
                cli('--render-json', str(path), data=data)
                samples.append(dict(voice=voice, title=title, model=model, text=TEXT,
                                    file=path.name, **check_audio(path)))

        assert len({sample['sha256'] for sample in samples}) == 6, 'Duplicate or missing audio'
        assert not (home / '.agent-voice').exists(), 'Read-only checks wrote settings or cache'
        report = dict(version=cli('--version').strip(), model_files_verified=len(manifest['files']),
                      kokoro_default_unchanged=True, isolated_home_unchanged=True, samples=samples)
        OUTPUT.mkdir(parents=True, exist_ok=True)
        for sample in samples:
            shutil.copyfile(home / sample['file'], OUTPUT / sample['file'])
        (OUTPUT / 'samples.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
        print(f'Validated {len(samples)} voice samples: {OUTPUT}')


if __name__ == '__main__':
    main()
