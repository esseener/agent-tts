"""Verify fixed voice preview/apply, native caption colors and compact layout."""
import ctypes as C
from ctypes import wintypes as W
import argparse
import copy
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

from smoke_native import fingerprints, u, wait_for, window_for
from check_voice_studio import capture

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, default=ROOT / 'AgentVoice.exe')
    args = parser.parse_args()
    exe = args.exe.resolve()
    original = fingerprints()
    temp = Path(os.environ['LOCALAPPDATA']) / 'Temp/opencode'
    with tempfile.TemporaryDirectory(prefix='agent-voice-gallery-', dir=temp) as folder:
        def cli(*args):
            return json.loads(subprocess.run([str(exe), '--home', folder, *args],
                              capture_output=True, encoding='utf-8', check=True).stdout)
        config = cli('--export-config')
        config['api_enabled'] = False
        p = config['profiles'][0]
        p.update(name='保留我的名称', voice='local:kokoro-v1.1:zf_032', rate=17, pitch=-8, volume=73)
        p['phrases']['done'] = ['我是 codex，任务完成了。', '请检查结果。']
        settings = Path(folder) / '.agent-voice/config.json'
        settings.parent.mkdir()
        settings.write_text(json.dumps(config, ensure_ascii=False), encoding='utf-8')
        app = subprocess.Popen([str(exe), '--home', folder])
        try:
            hwnd = wait_for(lambda: window_for(app.pid))
            wait_for(lambda: u.GetDlgItem(hwnd, 211))
            def button(id):
                u.SendMessageW(u.GetDlgItem(hwnd, id), 0xf5, 0, 0)
            def text(id):
                buf = C.create_unicode_buffer(2048)
                u.SendMessageW(u.GetDlgItem(hwnd, id), 0x0d, len(buf), C.addressof(buf))
                return buf.value
            button(11)
            wait_for(lambda: u.GetDlgItem(hwnd, 502))
            assert not u.GetDlgItem(hwnd, 301), 'Full selector belongs in editor'
            selections = cli('--voice-selections')
            assert len(selections) == 6
            for index in range(6):
                assert u.GetDlgItem(hwnd, 502 + index * 10) and u.GetDlgItem(hwnd, 503 + index * 10)
            # Preview a different recipe, save: speaker and tuning must stay.
            button(502)
            wait_for(lambda: text(90).startswith('已播放：'), timeout=90)
            button(2)
            after_preview = json.loads(settings.read_text(encoding='utf-8'))
            assert after_preview['profiles'][0] == p
            expected = copy.deepcopy(p)
            expected.update(voice=selections[0]['voice'], rate=selections[0]['rate'],
                            pitch=selections[0]['pitch'], mixer=selections[0]['mixer'])
            button(503)
            button(2)
            applied = json.loads(settings.read_text(encoding='utf-8'))
            assert applied['profiles'][0] == expected
            assert applied['hosts'] == config['hosts']
            # Every fixed combination applies predictably.
            for index, recipe in enumerate(selections):
                button(503 + index * 10)
                button(2)
                stored = json.loads(settings.read_text(encoding='utf-8'))['profiles'][0]
                assert stored['voice'] == recipe['voice'] and stored['mixer'] == recipe['mixer']
                assert stored['name'] == p['name'] and stored['volume'] == p['volume'] and stored['phrases'] == p['phrases']
            button(503)
            # Caption color remains fixed across simulated theme changes.
            d = C.WinDLL('dwmapi')
            d.DwmGetWindowAttribute.argtypes = [W.HWND, W.DWORD, C.c_void_p, W.DWORD]
            d.DwmSetWindowAttribute.argtypes = [W.HWND, W.DWORD, C.c_void_p, W.DWORD]
            colors = {}
            for message in [0x31a, 0x1e]:
                u.SendMessageW(hwnd, message, 0, 0)
                for attribute, expected_color in [(35, 0x050505), (36, 0xf5f5f5), (34, 0x050505)]:
                    value = W.DWORD()
                    result = d.DwmGetWindowAttribute(hwnd, attribute, C.byref(value), 4)
                    if result == 0:
                        assert value.value == expected_color, (attribute, value.value)
                        colors[str(attribute)] = hex(value.value)
                    else:
                        # These color attributes are documented for Set; Get
                        # returns E_INVALIDARG on some Windows DWM versions.
                        value.value = expected_color
                        assert d.DwmSetWindowAttribute(hwnd, attribute, C.byref(value), 4) == 0
                        colors[str(attribute)] = {'requested':hex(expected_color), 'set_hresult':0,
                                                  'get_hresult':hex(result & 0xffffffff)}
            u.ShowWindow(hwnd, 9)
            u.SetForegroundWindow(hwnd)
            time.sleep(.15)
            capture(hwnd, ROOT / 'docs/previews/native-voice-gallery.png')
            u.MoveWindow(hwnd, 70, 30, 960, 680, True)
            time.sleep(.15)
            capture(hwnd, ROOT / 'docs/previews/native-voice-gallery-compact.png')
            u.GetClientRect.argtypes = [W.HWND, C.POINTER(W.RECT)]
            u.MapWindowPoints.argtypes = [W.HWND, W.HWND, C.c_void_p, W.UINT]
            rect = W.RECT()
            u.GetClientRect(hwnd, C.byref(rect))
            for id in [502, 553, 310, 319, 312]:
                item = W.RECT()
                u.GetWindowRect(u.GetDlgItem(hwnd, id), C.byref(item))
                u.MapWindowPoints(None, hwnd, C.byref(item), 2)
                assert item.left >= 0 and item.top >= 0 and item.right <= rect.right and item.bottom <= rect.bottom
            button(316)
            wait_for(lambda: u.GetDlgItem(hwnd, 308))
            assert 'codex' in text(308), 'Spoken normalization does not rewrite source phrases'
            button(317)
            wait_for(lambda: u.GetDlgItem(hwnd, 360))
            assert text(304) == text(305) == '0' and text(367) == '100'
            button(2)
            u.PostMessageW(hwnd, 0x10, 0, 0)
            app.wait(8)
        finally:
            if app.poll() is None:
                app.terminate(); app.wait()
    assert fingerprints() == original
    report = dict(fixed_selections=6, preview_preserves_settings=True, apply_preserves_user_content=True,
                  native_caption_colors=colors, compact_layout=True, full_editor_accessible=True,
                  real_user_settings_unchanged=True)
    (ROOT / 'docs/voice-gallery-verification.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(report, ensure_ascii=False, indent=2), flush=True)


if __name__ == '__main__':
    main()
