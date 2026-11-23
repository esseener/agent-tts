"""Check native control lifetimes, edit state and visible frames during clicks."""
import argparse
import ctypes as C
from ctypes import wintypes as W
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time

from PIL import ImageGrab
from smoke_native import CALLBACK, fingerprints, u, wait_for, window_for
from check_voice_studio import capture

ROOT = Path(__file__).resolve().parent.parent
u.EnumChildWindows.argtypes = [W.HWND, CALLBACK, W.LPARAM]
u.GetParent.argtypes = [W.HWND]
u.GetParent.restype = W.HWND
u.GetDlgCtrlID.argtypes = [W.HWND]
u.ClientToScreen.argtypes = [W.HWND, C.POINTER(W.POINT)]
u.IsWindowEnabled.argtypes = [W.HWND]
u.GetClientRect.argtypes = [W.HWND, C.POINTER(W.RECT)]
u.GetForegroundWindow.restype = W.HWND
u.SetWindowPos.argtypes = [W.HWND, W.HWND, C.c_int, C.c_int, C.c_int, C.c_int, W.UINT]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, default=ROOT / 'AgentVoice.exe')
    parser.add_argument('--observe', action='store_true', help='Record the old release without requiring the fix')
    args = parser.parse_args()
    exe = args.exe.resolve()
    original = fingerprints()
    checks, actions = {}, []
    temp = Path(os.environ['LOCALAPPDATA']) / 'Temp/opencode'
    with tempfile.TemporaryDirectory(prefix='agent-voice-redraw-', dir=temp) as folder:
        home = Path(folder)
        config = json.loads(subprocess.run([str(exe), '--home', folder, '--export-config'],
                            capture_output=True, encoding='utf-8', check=True).stdout)
        with socket.socket() as probe:
            probe.bind(('127.0.0.1', 0))
            config['api_port'] = probe.getsockname()[1]
        config['api_enabled'] = False
        path = home / '.agent-voice/config.json'
        path.parent.mkdir()
        path.write_text(json.dumps(config, ensure_ascii=False), encoding='utf-8')
        app = subprocess.Popen([str(exe), '--home', folder])
        stopped = threading.Event()
        frames = []
        low_frames = []
        sampler = None
        try:
            hwnd = wait_for(lambda: window_for(app.pid))
            wait_for(lambda: u.GetDlgItem(hwnd, 211))
            u.ShowWindow(hwnd, 9)
            u.SetForegroundWindow(hwnd)
            # Only this disposable QA window is raised above utility overlays.
            # Token Monitor can otherwise cover the sampled title region.
            u.SetWindowPos(hwnd, -1, 0, 0, 0, 0, 0x0003)

            def text(id):
                buf = C.create_unicode_buffer(8192)
                u.SendMessageW(u.GetDlgItem(hwnd, id), 0x0d, len(buf), C.addressof(buf))
                return buf.value

            def edit(id, value):
                buf = C.create_unicode_buffer(value)
                u.SendMessageW(u.GetDlgItem(hwnd, id), 0x0c, 0, C.addressof(buf))

            def handles():
                result = {}
                @CALLBACK
                def visit(child, _):
                    if u.GetParent(child) == hwnd:
                        result[u.GetDlgCtrlID(child)] = child
                    return True
                u.EnumChildWindows(hwnd, visit, 0)
                return result

            def action(name, fn):
                before = handles()
                started = time.perf_counter()
                fn()
                elapsed = (time.perf_counter() - started) * 1000
                # A control can be destroyed inside BM_CLICK. Let the owning
                # thread finish that notification and present a visible frame.
                time.sleep(.045)
                assert app.poll() is None, f'App exited after {name}: {app.returncode}'
                after = handles()
                changed = [id for id in before.keys() & after.keys() if before[id] != after[id]]
                actions.append({'action': name, 'recreated_shared_controls': changed,
                                'elapsed_ms': round(elapsed, 2)})

            def click(id):
                control = wait_for(lambda: u.GetDlgItem(hwnd, id))
                if args.observe:
                    # The old code destroys the button inside its own click
                    # handler; send the equivalent command to audit lifetimes.
                    u.SendMessageW(hwnd, 0x111, id, control)
                else:
                    u.SendMessageW(control, 0xf5, 0, 0)  # BM_CLICK

            def select(id, index):
                control = wait_for(lambda: u.GetDlgItem(hwnd, id))
                u.SendMessageW(control, 0x14e, index, 0)
                u.SendMessageW(hwnd, 0x111, id | (1 << 16), control)

            # The logo and version area never change during these operations.
            origin = W.POINT()
            u.ClientToScreen(hwnd, C.byref(origin))
            area = (origin.x + 24, origin.y + 8, origin.x + 500, origin.y + 73)
            expected = ImageGrab.grab(bbox=area).convert('L')
            mask = [i for i, value in enumerate(expected.tobytes()) if value > 160]
            assert len(mask) > 100, 'Reference header must be visible'

            def sample():
                worst = 1.0
                while not stopped.is_set():
                    shot = ImageGrab.grab(bbox=area)
                    image = shot.convert('L').tobytes()
                    coverage = sum(image[i] > 110 for i in mask) / len(mask)
                    frames.append(coverage)
                    if coverage < .8 and coverage < worst:
                        foreground = u.GetForegroundWindow()
                        owner = W.DWORD()
                        u.GetWindowThreadProcessId(foreground, C.byref(owner))
                        low_frames.append({'coverage':coverage,'foreground_pid':owner.value,'app_pid':app.pid})
                        shot.save(ROOT / 'docs/previews/redraw-low-header.png')
                        worst = coverage
                    stopped.wait(.008)

            sampler = threading.Thread(target=sample, daemon=True)
            sampler.start()
            for _ in range(3):
                action('current host tab', lambda: click(10))
            action('open profiles', lambda: click(11))
            if u.GetDlgItem(hwnd, 502):
                action('open phrase editor', lambda: click(316))
            wait_for(lambda: u.GetDlgItem(hwnd, 308))
            edit(308, '\r\n'.join(f'第{i}句测试台词' for i in range(40)))
            phrase = u.GetDlgItem(hwnd, 308)
            u.SendMessageW(phrase, 0xb1, 1, 7)  # EM_SETSEL
            u.SendMessageW(phrase, 0xb6, 0, 15)  # EM_LINESCROLL

            def edit_state():
                start, end = W.DWORD(), W.DWORD()
                control = u.GetDlgItem(hwnd, 308)
                u.SendMessageW(control, 0xb0, C.addressof(start), C.addressof(end))
                return [start.value, end.value, u.SendMessageW(control, 0xce, 0, 0)]

            state = edit_state()
            action('favorite voice with scrolled phrase', lambda: click(314))
            checks['phrase_caret_and_scroll_preserved'] = edit_state() == state
            action('voice filter', lambda: select(313, 3))
            action('all voices', lambda: select(313, 0))
            action('open mixer', lambda: click(317))
            wait_for(lambda: u.GetDlgItem(hwnd, 360))
            edit(350, '保留我的调音名字')
            for _ in range(4):
                action('current mixer tab', lambda: click(317))
                action('next fixed voice', lambda: click(312))
                action('favorite voice', lambda: click(314))
                action('apply tuning', lambda: select(340, 3))
                action('reset tuning', lambda: click(341))
            checks['tuning_name_preserved'] = text(350) == '保留我的调音名字'
            # A retained native widget must also recover from a disabled state.
            count = u.SendMessageW(u.GetDlgItem(hwnd, 301), 0x146, 0, 0)
            action('system voice', lambda: select(301, count - 1))
            disabled = not u.IsWindowEnabled(u.GetDlgItem(hwnd, 360))
            action('local voice again', lambda: select(301, 10))
            checks['mixer_enable_state_recovers'] = disabled and bool(u.IsWindowEnabled(u.GetDlgItem(hwnd, 360)))
            action('open interfaces', lambda: click(12))
            edit(400, 'draft-agent')
            edit(401, '尚未添加的宿主')
            for _ in range(3):
                action('current interfaces tab', lambda: click(12))
                action('scan from interfaces', lambda: click(13))
                action('start or stop API', lambda: click(404))
            action('stop API', lambda: click(404))
            checks['host_form_draft_preserved'] = text(400) == 'draft-agent' and text(401) == '尚未添加的宿主'
            for _ in range(8):
                action('switch to hosts', lambda: click(10))
                action('switch to mixer', lambda: click(11))
            stopped.set()
            sampler.join(5)
            checks['shared_controls_retained'] = not any(a['recreated_shared_controls'] for a in actions)
            checks['stable_header_frames'] = len(frames) > 0 and min(frames) > .95
            capture(hwnd, ROOT / 'docs/previews/native-redraw-normal.png')
            u.MoveWindow(hwnd, 80, 40, 960, 680, True)
            wait_for(lambda: u.GetDlgItem(hwnd, 353))
            # Allow queued WM_PAINT messages to reach the compositor.
            u.SendMessageW(hwnd, 0x000f, 0, 0)
            capture(hwnd, ROOT / 'docs/previews/native-redraw-compact.png')
            checks['compact_controls_visible'] = all(u.GetDlgItem(hwnd, id) for id in [2, 317, 351, 353, 310])
            click(2)
            u.PostMessageW(hwnd, 0x10, 0, 0)
            app.wait(8)
        finally:
            stopped.set()
            if sampler: sampler.join(5)
            if app.poll() is None:
                app.terminate()
                app.wait(8)
    checks['real_user_settings_unchanged'] = fingerprints() == original
    report = {'version': config['version'], 'actions': len(actions),
              'shared_control_recreations': sum(len(a['recreated_shared_controls']) for a in actions),
              'sampled_frames': len(frames), 'minimum_header_coverage': min(frames, default=0),
              'low_frame_diagnostics': low_frames,
              'checks': checks, 'details': actions}
    output = ROOT / ('docs/native-redraw-baseline.json' if args.observe else 'docs/native-redraw-verification.json')
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({key: value for key, value in report.items() if key != 'details'}, ensure_ascii=False), flush=True)
    if not args.observe:
        assert all(checks.values()), checks


if __name__ == '__main__':
    main()
