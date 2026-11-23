"""Exercise real audio levels, native sliders and shareable tuning presets."""
import argparse
from array import array
import copy
import ctypes as C
from ctypes import wintypes as W
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import wave

from smoke_native import CALLBACK, fingerprints, u, wait_for, window_for

ROOT = Path(__file__).resolve().parent.parent


def measure(path):
    with wave.open(str(path), 'rb') as wav:
        rate = wav.getframerate()
        values = [v / 32768 for v in array('h', wav.readframes(wav.getnframes()))]
    peak = max(map(abs, values), default=0)
    speech = [v for v in values if abs(v) > max(peak * .035, .00003)]
    rms = math.sqrt(sum(v*v for v in speech) / max(1, len(speech)))
    return {'speech_dbfs': 20 * math.log10(max(rms, 1e-9)),
            'peak_dbfs': 20 * math.log10(max(peak, 1e-9)), 'sample_rate': rate}


def capture(hwnd, path):
    from PIL import Image
    g = C.WinDLL('gdi32')
    u.GetWindowDC.argtypes = [W.HWND]; u.GetWindowDC.restype = W.HDC
    u.ReleaseDC.argtypes = [W.HWND, W.HDC]
    u.PrintWindow.argtypes = [W.HWND, W.HDC, W.UINT]
    g.CreateCompatibleDC.argtypes = [W.HDC]; g.CreateCompatibleDC.restype = W.HDC
    g.CreateCompatibleBitmap.argtypes = [W.HDC, C.c_int, C.c_int]; g.CreateCompatibleBitmap.restype = W.HBITMAP
    g.SelectObject.argtypes = [W.HDC, W.HANDLE]; g.SelectObject.restype = W.HANDLE
    g.DeleteObject.argtypes = [W.HANDLE]; g.DeleteDC.argtypes = [W.HDC]
    g.GetDIBits.argtypes = [W.HDC, W.HBITMAP, W.UINT, W.UINT, C.c_void_p, C.c_void_p, W.UINT]
    class Header(C.Structure):
        _fields_ = [('size', W.DWORD), ('width', W.LONG), ('height', W.LONG), ('planes', W.WORD),
                    ('bits', W.WORD), ('compression', W.DWORD), ('bytes', W.DWORD),
                    ('xppm', W.LONG), ('yppm', W.LONG), ('used', W.DWORD), ('important', W.DWORD)]
    rect = W.RECT(); u.GetWindowRect(hwnd, C.byref(rect))
    width, height = rect.right-rect.left, rect.bottom-rect.top
    header = Header(40, width, -height, 1, 32, 0, 0, 0, 0, 0, 0)
    screen = u.GetWindowDC(hwnd); memory = g.CreateCompatibleDC(screen)
    bitmap = g.CreateCompatibleBitmap(screen, width, height); prior = g.SelectObject(memory, bitmap)
    try:
        u.UpdateWindow(hwnd)
        assert u.PrintWindow(hwnd, memory, 0)
        pixels = C.create_string_buffer(width*height*4)
        g.GetDIBits(memory, bitmap, 0, height, pixels, C.byref(header), 0)
        Image.frombytes('RGB', (width, height), pixels.raw, 'raw', 'BGRX').save(path)
    finally:
        g.SelectObject(memory, prior); g.DeleteObject(bitmap); g.DeleteDC(memory); u.ReleaseDC(hwnd, screen)


def tuning_dialog(pid, title):
    found = []
    @CALLBACK
    def visit(hwnd, _):
        process = W.DWORD(); u.GetWindowThreadProcessId(hwnd, C.byref(process))
        text = C.create_unicode_buffer(512); u.GetWindowTextW(hwnd, text, 512)
        if process.value == pid and text.value == title and u.IsWindowVisible(hwnd): found.append(hwnd)
        return True
    u.EnumWindows(visit, 0)
    return found[0] if found else None


def filename_field(dialog):
    # Explorer-style dialogs may nest the file-name edit inside a ComboBoxEx.
    u.EnumChildWindows.argtypes = [W.HWND, CALLBACK, W.LPARAM]
    edits = []
    @CALLBACK
    def visit(hwnd, _):
        name = C.create_unicode_buffer(128); u.GetClassNameW(hwnd, name, 128)
        if name.value == 'Edit' and u.IsWindowVisible(hwnd): edits.append(hwnd)
        return True
    u.EnumChildWindows(dialog, visit, 0)
    return u.GetDlgItem(dialog, 1148) or (edits[-1] if edits else None)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, default=ROOT / 'AgentVoice.exe')
    parser.add_argument('--all-voices', action='store_true')
    parser.add_argument('--reuse-audio', action='store_true', help='Reuse the last completed audio audit while verifying UI changes')
    args = parser.parse_args()
    exe = args.exe.resolve()
    audio_source = hashlib.sha256((ROOT / 'native/mixer.rs').read_bytes() + (ROOT / 'native/local_tts.rs').read_bytes()).hexdigest()
    original = fingerprints()
    with tempfile.TemporaryDirectory(prefix='voice-studio-', dir=Path(os.environ['LOCALAPPDATA']) / 'Temp/opencode') as folder:
        home = Path(folder)
        def cli(*args, data=None, timeout=90):
            result = subprocess.run([str(exe), '--home', str(home), *args], input=data,
                                    capture_output=True, encoding='utf-8', timeout=timeout)
            assert result.returncode == 0, result.stderr
            return json.loads(result.stdout)
        config = cli('--export-config')
        catalog = cli('--voice-catalog')
        voices = catalog['voices']
        assert len(voices) == len({v['name'] for v in voices}) == 103
        assert sum(v['gender'] == 'male' for v in voices) == 45
        preset_library = cli('--mixer-presets')
        assert len(preset_library) == 7
        chosen = [] if args.reuse_audio else (range(103) if args.all_voices else [3,59,9,75,21,11])
        batch = []
        for number in chosen:
            for raw in [True, False]:
                profile = copy.deepcopy(config['profiles'][0]); profile['voice'] = f'local:kokoro-v1.1:{voices[number]["speaker"]}'
                if raw: profile['mixer'].update(normalize=False, compression=0)
                batch.append((profile, '任务已经完成，请检查结果。', str(home / f'{number}-{raw}.wav')))
        if batch: cli('--render-batch-json', data=json.dumps(batch, ensure_ascii=False), timeout=3600 if args.all_voices else 600)
        previous = json.loads((ROOT / 'docs/voice-loudness-verification.json').read_text(encoding='utf-8')) if args.reuse_audio else None
        if previous:
            assert previous.get('audio_source_sha256') == audio_source, 'Audio source changed; run the audio audit again'
        levels = previous['levels'] if previous else []
        for number in chosen:
            raw = measure(home / f'{number}-True.wav'); processed = measure(home / f'{number}-False.wav')
            assert processed['peak_dbfs'] <= -.99, (number, processed)
            assert -21 <= processed['speech_dbfs'] <= -16, (number, processed)
            levels.append({'id': number, 'name': voices[number]['name'], 'raw': raw, 'processed': processed})
        print('Real audio leveling:', json.dumps({'speakers': len(levels),
              'quietest_original_dbfs': min(v['raw']['speech_dbfs'] for v in levels),
              'processed_range_dbfs': [min(v['processed']['speech_dbfs'] for v in levels), max(v['processed']['speech_dbfs'] for v in levels)]}), flush=True)
        (ROOT / 'docs/voice-loudness-verification.json').write_text(json.dumps({'tested_speakers':len(levels),'audio_source_sha256':audio_source,'levels':levels},ensure_ascii=False,indent=2),encoding='utf-8')
        # Max gain with all EQ bands boosted must remain bounded on actual model audio.
        boosted = copy.deepcopy(config['profiles'][0]); boosted['mixer'].update(gain_db=18, low_db=12, presence_db=12, high_db=12, compression=100)
        cli('--render-batch-json', data=json.dumps([(boosted, '这是一条响亮清晰的提醒。', str(home / 'boosted.wav'))]))
        assert measure(home / 'boosted.wav')['peak_dbfs'] <= -.99

        # Load an old config without mixer fields to exercise compatible defaults.
        config['profiles'][0].pop('mixer'); config['profiles'][0]['phrases']['done'] = ['保留我的台词。']
        settings = home / '.agent-voice/config.json'; settings.parent.mkdir(parents=True, exist_ok=True)
        settings.write_text(json.dumps(config, ensure_ascii=False), encoding='utf-8')
        app = subprocess.Popen([str(exe), '--home', str(home)])
        try:
            hwnd = wait_for(lambda: window_for(app.pid))
            wait_for(lambda: u.GetDlgItem(hwnd, 211))
            def button(id):
                u.SendMessageW(hwnd, 0x111, id, wait_for(lambda: u.GetDlgItem(hwnd, id)))
            def choose(id, index):
                u.SendMessageW(u.GetDlgItem(hwnd, id), 0x14e, index, 0)
                u.SendMessageW(hwnd, 0x111, id | (1 << 16), u.GetDlgItem(hwnd, id))
            def text(id):
                buffer = C.create_unicode_buffer(2048)
                control = u.GetDlgItem(hwnd, id)
                if id in [301,313,340]:
                    selected = u.SendMessageW(control, 0x147, 0, 0)
                    u.SendMessageW(control, 0x148, selected, C.addressof(buffer))
                else:
                    u.SendMessageW(control, 0x0d, 2048, C.addressof(buffer))
                return buffer.value
            def edit(id, value):
                buffer = C.create_unicode_buffer(value)
                u.SendMessageW(u.GetDlgItem(hwnd, id), 0x0c, 0, C.addressof(buffer))
            button(11)
            wait_for(lambda: u.GetDlgItem(hwnd,312))
            button(316)
            assert 'zf_001' in text(301) and '女声' in text(301)
            choose(313, 2); choose(301, 1)
            assert '女声' in text(301)
            button(314); choose(313, 4)
            assert u.SendMessageW(u.GetDlgItem(hwnd, 301), 0x146, 0, 0) == 1
            button(317)
            assert text(368) == '自动响度补偿'
            choose(340, 1)
            assert text(362) == '0' and text(364) == '15'
            assert not u.GetDlgItem(hwnd,365) and not u.GetDlgItem(hwnd,366), 'No unsupported model variation controls'
            slider = u.GetDlgItem(hwnd, 1360)
            u.SendMessageW(slider, 0x405, 1, 6)
            u.SendMessageW(hwnd, 0x114, 5, slider)
            assert text(360) == '6' and text(340) == '自定义参数', {'gain':text(360),'preset':text(340),'slider':u.SendMessageW(slider,0x400,0,0)}
            edit(350, '我喜欢的提醒'); button(351)
            saved = json.loads(settings.read_text(encoding='utf-8'))
            assert saved['saved_tunings'][-1]['mixer']['gain_db'] == 6
            assert saved['saved_tunings'][-1]['name'] == '我喜欢的提醒'
            assert saved['profiles'][0]['mixer']['gain_db'] == 6
            assert saved['profiles'][0]['phrases']['done'] == ['保留我的台词。']
            button(341)
            choose(340, 8)
            assert text(360) == '6'
            # Export through the actual native Save dialog, then import it.
            u.PostMessageW(hwnd, 0x111, 353, u.GetDlgItem(hwnd, 353))
            dialog = wait_for(lambda: tuning_dialog(app.pid, '导出当前调音'), timeout=20)
            export = home / '我的调音.json'
            filename = C.create_unicode_buffer(str(export))
            u.SendMessageW(wait_for(lambda: filename_field(dialog)), 0x0c, 0, C.addressof(filename))
            u.PostMessageW(dialog, 0x111, 1, u.GetDlgItem(dialog, 1))
            wait_for(lambda: export.is_file())
            document = json.loads(export.read_text(encoding='utf-8'))
            assert document['presets'][0]['mixer']['gain_db'] == 6
            document['presets'][0]['name'] = '朋友分享的调音'
            export.write_text(json.dumps(document, ensure_ascii=False), encoding='utf-8')
            u.PostMessageW(hwnd, 0x111, 352, u.GetDlgItem(hwnd, 352))
            dialog = wait_for(lambda: tuning_dialog(app.pid, '导入调音预设'), timeout=20)
            filename = C.create_unicode_buffer(str(export))
            u.SendMessageW(wait_for(lambda: filename_field(dialog)), 0x0c, 0, C.addressof(filename))
            u.PostMessageW(dialog, 0x111, 1, u.GetDlgItem(dialog, 1))
            wait_for(lambda: '已导入' in text(90))
            button(2)
            saved = json.loads(settings.read_text(encoding='utf-8'))
            assert saved['saved_tunings'][-1]['name'] == '朋友分享的调音'
            button(310); wait_for(lambda: text(90).startswith('已播放：'), timeout=90)
            assert '峰值' in text(90)
            previews = ROOT / 'docs/previews'
            capture(hwnd, previews / 'native-mixer.png')
            u.MoveWindow(hwnd, 60, 40, 960, 680, True)
            capture(hwnd, previews / 'native-mixer-compact.png')
            u.MoveWindow(hwnd, 60, 40, 1156, 829, True)
            button(316)
            assert text(308) == '保留我的台词。'
            capture(hwnd, previews / 'native-voice-library.png')
            u.PostMessageW(hwnd, 0x10, 0, 0); app.wait(5)
            assert app.returncode == 0
        finally:
            if app.poll() is None: app.terminate(); app.wait()
    assert fingerprints() == original
    raw_range = [v['raw']['speech_dbfs'] for v in levels]
    processed_range = [v['processed']['speech_dbfs'] for v in levels]
    report = {'model':'Kokoro v1.1-zh INT8','named_voices': 103, 'female': 58, 'male': 45, 'built_in_tunings': 7,
              'tested_speakers': len(levels), 'raw_loudness_spread_db': max(raw_range)-min(raw_range),
              'processed_loudness_spread_db': max(processed_range)-min(processed_range),
              'max_gain_test_db': 18, 'sample_peak_ceiling_dbfs': -1,
              'native_sliders': True, 'favorites_and_gender_filter': True,
              'native_preset_export_import': True, 'legacy_config_compatible': True,
              'real_user_settings_unchanged': True, 'levels': levels}
    (ROOT / 'docs/voice-studio-verification.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({k:v for k,v in report.items() if k != 'levels'}, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
