"""Verify the one-model portable deployment, real CPU synthesis and idle release."""
import argparse
from array import array
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import tempfile
import time
import wave
import zipfile

import psutil
from smoke_native import VERSION, fingerprints, u, wait_for, window_for

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, default=ROOT / 'AgentVoice.exe')
    args = parser.parse_args()
    exe = args.exe.resolve()
    original = fingerprints()
    resources = ROOT / 'resources/kokoro'
    manifest = json.loads((resources / 'manifest.json').read_text(encoding='utf-8'))
    assert len(list(resources.glob('*.onnx'))) == 1
    assert (resources / 'model.int8.onnx').stat().st_size == 114203914
    for name, info in manifest['files'].items():
        assert hashlib.sha256((resources / name).read_bytes()).hexdigest() == info['sha256'], name

    with tempfile.TemporaryDirectory(prefix='agent-voice-local-', dir=Path(os.environ['LOCALAPPDATA']) / 'Temp/opencode') as folder:
        portable = Path(folder) / '中文 便携应用 🎵'
        portable.mkdir()
        target = portable / 'AgentVoice.exe'
        shutil.copy2(exe, target)
        shutil.copytree(resources, portable / 'resources/kokoro')
        shutil.copytree(ROOT / 'resources/notifications', portable / 'resources/notifications')
        home = Path(folder) / 'home'
        # PATH has neither Python/Node nor the development dependency directory.
        env = dict(os.environ, PATH=str(Path(os.environ['WINDIR']) / 'System32'),
                   HTTP_PROXY='http://127.0.0.1:9', HTTPS_PROXY='http://127.0.0.1:9')

        def cli(*args):
            result = subprocess.run([str(target), '--home', str(home), *args], cwd=portable,
                                    env=env, capture_output=True, encoding='utf-8', timeout=180)
            assert result.returncode == 0, result.stderr
            return json.loads(result.stdout)

        voices = cli('--voices')
        assert len([key for key, _ in voices if key.startswith('local:kokoro-v1.1:')]) == 103
        config = cli('--export-config')
        assert config['profiles'][0]['voice'] == 'local:kokoro-v1.1:zf_001'
        cli('--use-local-voices')
        config = cli('--export-config')
        config['profiles'][0]['phrases']['done'] = ['任务已经完成。', '代码检查通过了。']
        settings = home / '.agent-voice/config.json'
        settings.write_text(json.dumps(config, ensure_ascii=False), encoding='utf-8')

        app = subprocess.Popen([str(target), '--home', str(home), '--benchmark-local', '--idle-check'],
                               cwd=portable, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, encoding='utf-8')
        process = psutil.Process(app.pid)
        peak_rss = 0
        connections = []
        samples = []
        started = time.monotonic()
        try:
            while app.poll() is None:
                try:
                    memory = process.memory_info()
                    peak_rss = max(peak_rss, memory.rss)
                    cpu = process.cpu_times()
                    samples.append((time.monotonic() - started, memory.rss, cpu.user + cpu.system))
                    if len(samples) % 20 == 0:
                        connections.extend(process.net_connections(kind='inet'))
                except psutil.NoSuchProcess:
                    break
                if time.monotonic() - started > 240:
                    raise AssertionError('Local benchmark/idle cleanup timed out')
                time.sleep(.1)
            stdout, stderr = app.communicate(timeout=3)
            assert app.returncode == 0, stderr
        finally:
            if app.poll() is None:
                app.terminate()
                app.wait()
        benchmark = json.loads(stdout)
        print('Portable synthesis measurement:', json.dumps(benchmark, ensure_ascii=False), flush=True)
        print('Memory MB (peak / after idle):', round(peak_rss / 1024**2, 1), round(samples[-1][1] / 1024**2, 1), flush=True)
        # First access to newly copied DLLs/model can include Windows file
        # scanning. Report that cost separately from steady-state synthesis.
        # Natural-voice generation is deliberately measured independently of
        # cached playback. This model is heavier; do not imply a 2s guarantee.
        assert max(benchmark['synthesis_ms']) < 60000
        assert not connections, 'Local synthesis opens no network connections'
        assert samples[-1][1] < peak_rss * .85, 'Model/runtime memory released after idle timeout'
        idle_start = next(s for s in samples if s[0] > samples[-1][0] - 20)
        idle_end = next(s for s in samples if s[0] > samples[-1][0] - 12)
        idle_cpu_percent = 100 * (idle_end[2] - idle_start[2]) / (idle_end[0] - idle_start[0])
        assert idle_cpu_percent < 3, f'Idle CPU stays low: {idle_cpu_percent}'
        with wave.open(str(home / 'local-benchmark.wav'), 'rb') as audio:
            audio_seconds = audio.getnframes() / audio.getframerate()
            assert audio.getnframes() > 1000
            assert audio.getsampwidth() == 2
            assert audio.getframerate() == 24000, 'Native output, not upsampled 8 kHz'
            assert max(abs(x) for x in array('h', audio.readframes(audio.getnframes()))) > 100, 'Generated waveform contains speech'

        prepared = cli('--warm-cache')
        assert prepared['prepared'] >= 5
        replay = cli('--warm-cache')
        assert replay['elapsed_ms'] < 200, 'Pre-generated phrases require no model load'
        files_before = {p.name: p.read_bytes() for p in (home / '.agent-voice/cache').glob('*.wav')}
        config['profiles'][0]['name'] = '只改名字'
        settings.write_text(json.dumps(config, ensure_ascii=False), encoding='utf-8')
        renamed = cli('--warm-cache')
        assert renamed['elapsed_ms'] < 200
        assert files_before == {p.name: p.read_bytes() for p in (home / '.agent-voice/cache').glob('*.wav')}
        # A different speaker and pitch/rate are real generated audio.
        config['profiles'][0].update(voice='local:kokoro-v1.1:zm_010', pitch=0, rate=10, volume=80)
        settings.write_text(json.dumps(config, ensure_ascii=False), encoding='utf-8')
        cli('--render', 'default', '--text', 'Codex 已完成任务，等待你确认。', '--out', str(home / 'different.wav'))
        assert (home / 'different.wav').stat().st_size > 1000
        result = subprocess.run([str(target), '--home', str(home), '--play-event', 'chatgpt', 'done'],
                                cwd=portable, env=env, capture_output=True, encoding='utf-8', timeout=90)
        assert result.returncode == 0, result.stderr
        activity = [json.loads(line) for line in (home / '.agent-voice/activity.jsonl').read_text(encoding='utf-8').splitlines()]
        assert activity[-1]['ok'] and activity[-1]['message'].startswith('已播放：')

        app = subprocess.Popen([str(target), '--home', str(home)], cwd=portable, env=env)
        try:
            hwnd = wait_for(lambda: window_for(app.pid))
            wait_for(lambda: u.GetDlgItem(hwnd, 211))
            u.SendMessageW(hwnd, 0x111, 11, u.GetDlgItem(hwnd, 11))
            wait_for(lambda: u.GetDlgItem(hwnd, 312))
            u.SendMessageW(hwnd, 0x111, 312, u.GetDlgItem(hwnd, 312))
            u.SendMessageW(hwnd, 0x111, 2, u.GetDlgItem(hwnd, 2))
            selected = json.loads(settings.read_text(encoding='utf-8'))['profiles'][0]
            choices = cli('--voice-selections')
            assert selected['voice'] in [s['voice'] for s in choices], 'Next selection is a fixed shared-model voice'
            assert selected['rate'] == selected['pitch'] == 0
            u.PostMessageW(hwnd, 0x10, 0, 0)
            app.wait(5)
            assert app.returncode == 0
        finally:
            if app.poll() is None:
                app.terminate()
                app.wait()

    package = ROOT / f'dist/AgentVoice-{VERSION}.zip'
    with zipfile.ZipFile(package) as zipped:
        for name in [*manifest['files'], 'manifest.json']:
            assert zipped.read(f'AgentVoice-{VERSION}/resources/kokoro/' + name) == (resources / name).read_bytes()
        assert zipped.read(f'AgentVoice-{VERSION}/AgentVoice.exe') == exe.read_bytes()
    assert fingerprints() == original
    report = {
        'model': 'Kokoro v1.1-zh INT8', 'model_bytes': 114203914, 'speakers': 103,
        'resource_bytes': sum(p.stat().st_size for p in resources.rglob('*') if p.is_file()),
        'exe_bytes': exe.stat().st_size, 'zip_bytes': package.stat().st_size,
        'cpu_threads': 2, 'synthesis_ms': benchmark['synthesis_ms'], 'sample_audio_seconds': audio_seconds,
        'median_warm_synthesis_ms': statistics.median(benchmark['synthesis_ms'][1:]),
        'sample_rate': 24000,
        'warm_realtime_factor': round(statistics.median(benchmark['synthesis_ms'][1:]) / 1000 / audio_seconds,3),
        'peak_rss_mb': round(peak_rss / 1024**2, 1), 'idle_rss_mb': round(samples[-1][1] / 1024**2, 1),
        'idle_cpu_percent_one_core': round(idle_cpu_percent, 2), 'idle_release_seconds': 30,
        'prepared_phrase_count': prepared['prepared'], 'cached_prepare_ms': replay['elapsed_ms'],
        'no_network_connections': True, 'portable_unicode_path': True, 'fixed_voice_gui': True,
        'package_matches': True, 'real_user_settings_unchanged': True,
        'sha256': hashlib.sha256(exe.read_bytes()).hexdigest(),
    }
    (ROOT / 'docs/local-tts-verification.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
