"""Exercise the native notification repeat setting and concurrent hooks."""

import ctypes as C
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import wave

from smoke_native import ROOT, fingerprints, u, wait_for, window_for


def records(path):
    try:
        return [json.loads(line) for line in path.read_text(encoding='utf-8').splitlines()]
    except (ValueError, OSError):
        return []


def main():
    exe = ROOT / 'AgentVoice-Hojo-试用.exe'
    before = fingerprints()
    temp = Path(os.environ['LOCALAPPDATA']) / 'Temp/opencode'
    temp.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=temp, prefix='agent-voice-repeat-') as folder:
        home = Path(folder)
        args = [str(exe), '--home', folder]
        trace = home / 'playback.jsonl'
        env = dict(os.environ, AGENT_VOICE_PLAYBACK_TRACE=str(trace))
        cli = lambda *opts: json.loads(subprocess.run(
            [*args, *opts], env=env, stdin=subprocess.DEVNULL, capture_output=True,
            encoding='utf-8', check=True, timeout=10).stdout)
        config = cli('--export-config')
        assert config.pop('repeat_window_seconds') == 8  # Test upgrade from a config without the field.
        with socket.socket() as probe:
            probe.bind(('127.0.0.1', 0))
            config.update(api_enabled=True, api_port=probe.getsockname()[1])
        audio = home / 'silent.wav'
        with wave.open(str(audio), 'wb') as target:
            target.setparams((1, 2, 24000, 0, 'NONE', 'not compressed'))
            target.writeframes(b'\0\0' * 4800)
        config['profiles'][0].update(phrases={}, volume=0)
        for host in config['hosts']:
            for route in host['routes'].values():
                route['audio'] = str(audio)
        settings = home / '.agent-voice/config.json'
        settings.parent.mkdir()
        settings.write_text(json.dumps(config, ensure_ascii=False), encoding='utf-8')
        activity = settings.with_name('activity.jsonl')
        app = subprocess.Popen(args, env=env)
        try:
            hwnd = wait_for(lambda: window_for(app.pid))
            control = lambda id: u.GetDlgItem(hwnd, id)
            wait_for(lambda: control(211))
            u.SendMessageW(control(12), 0xf5, 0, 0)
            label = C.create_unicode_buffer(60)
            u.SendMessageW(control(428), 0x0d, len(label), C.addressof(label))
            assert label.value == '重复播报合并（秒）'
            edit = wait_for(lambda: control(408))
            value = C.create_unicode_buffer(16)
            u.SendMessageW(edit, 0x0d, len(value), C.addressof(value))
            assert value.value == '8'

            def save_window(seconds):
                buf = C.create_unicode_buffer(str(seconds))
                u.SendMessageW(edit, 0x0c, 0, C.addressof(buf))
                u.SendMessageW(control(2), 0xf5, 0, 0)
                assert json.loads(settings.read_text(encoding='utf-8'))['repeat_window_seconds'] == seconds

            save_window(12)
            hooks = [subprocess.Popen([*args, '--hook', 'opencode', 'done'], env=env,
                                      stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                      stderr=subprocess.PIPE) for _ in range(8)]
            for hook in hooks:
                stdout, stderr = hook.communicate(timeout=10)
                assert hook.returncode == 0 and stdout.strip() == b'{}', stderr
            wait_for(lambda: len(records(activity)) == 8)
            assert len(records(trace)) == 1
            assert sum(row['message'] == '已合并重复播报' for row in records(activity)) == 7

            def post(**extra):
                body = json.dumps(dict(host='opencode', event='done', **extra)).encode('utf-8')
                headers = (f'POST /v1/events HTTP/1.1\r\nHost: localhost\r\n'
                           f'Authorization: Bearer {config["api_token"]}\r\n'
                           f'Content-Length: {len(body)}\r\nConnection: close\r\n\r\n').encode()
                with socket.create_connection(('127.0.0.1', config['api_port']), timeout=5) as client:
                    client.sendall(headers + body)
                    with client.makefile('rb') as response:
                        assert response.readline().startswith(b'HTTP/1.1 202 ')

            post()  # Same notification, arriving via the GUI's HTTP listener.
            wait_for(lambda: len(records(activity)) == 9)
            assert len(records(trace)) == 1
            post(text='separate text override')
            assert cli('--emit', 'opencode', 'error')['queued']
            wait_for(lambda: len(records(activity)) == 11)
            assert len(records(trace)) == 3

            save_window(0)
            assert cli('--emit', 'opencode', 'done')['queued']
            assert cli('--emit', 'opencode', 'done')['queued']
            wait_for(lambda: len(records(activity)) == 13)
            assert len(records(trace)) == 5
        finally:
            if app.poll() is None:
                hwnd = window_for(app.pid)
                if hwnd:
                    u.PostMessageW(hwnd, 0x10, 0, 0)
                try:
                    app.wait(6)
                except subprocess.TimeoutExpired:
                    app.terminate()
                    app.wait(6)
    assert fingerprints() == before
    print('Eight concurrent hooks produced one playback; HTTP shares the gate, different alerts play, and the native setting can disable it.')


if __name__ == '__main__':
    main()
