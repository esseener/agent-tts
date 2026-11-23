"""Keep old windows alive with a recoverable visible-draft snapshot, then open."""
import ctypes as C
from ctypes import wintypes as W
from datetime import datetime
import json
from pathlib import Path
import subprocess

from smoke_native import CALLBACK, fingerprints, u, wait_for, window_for, VERSION

ROOT = Path(__file__).resolve().parent.parent
u.EnumChildWindows.argtypes = [W.HWND, CALLBACK, W.LPARAM]
u.GetDlgCtrlID.argtypes = [W.HWND]


def text(hwnd):
    count = u.SendMessageW(hwnd, 0x0e, 0, 0)
    buf = C.create_unicode_buffer(min(count + 1, 40001))
    u.SendMessageW(hwnd, 0x0d, len(buf), C.addressof(buf))
    return buf.value


def main():
    original = fingerprints()
    previous = []
    @CALLBACK
    def visit(hwnd, _):
        name = C.create_unicode_buffer(128)
        u.GetClassNameW(hwnd, name, len(name))
        if name.value == 'AgentVoiceNativeWindow': previous.append(hwnd)
        return True
    u.EnumWindows(visit, 0)
    snapshots = []
    for hwnd in previous:
        pid = W.DWORD()
        u.GetWindowThreadProcessId(hwnd, C.byref(pid))
        controls = []
        @CALLBACK
        def child(item, _):
            name = C.create_unicode_buffer(128)
            u.GetClassNameW(item, name, len(name))
            controls.append(dict(id=u.GetDlgCtrlID(item), kind=name.value, text=text(item)))
            return True
        u.EnumChildWindows(hwnd, child, 0)
        path = Path.home() / '.agent-voice' / f'window-draft-before-{VERSION}-{pid.value}-{datetime.now():%Y%m%d-%H%M%S}.json'
        path.parent.mkdir(exist_ok=True)
        path.write_text(json.dumps(dict(title=text(hwnd), pid=pid.value, controls=controls,
            saved_config_fingerprints=original, note='Visible controls only; the old process remains alive to preserve other unsaved pages.'),ensure_ascii=False,indent=2),encoding='utf-8')
        snapshots.append(str(path))
        u.ShowWindow(hwnd, 6)
    app = subprocess.Popen([str(ROOT / 'AgentVoice.exe')], stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    hwnd = wait_for(lambda: window_for(app.pid), timeout=15)
    wait_for(lambda: u.GetDlgItem(hwnd, 11))
    u.SendMessageW(hwnd, 0x111, 11, u.GetDlgItem(hwnd, 11))
    wait_for(lambda: u.GetDlgItem(hwnd, 502))
    u.ShowWindow(hwnd, 9)
    u.SetForegroundWindow(hwnd)
    assert fingerprints() == original, 'Opening and draft conversion must not save user settings'
    print(json.dumps(dict(pid=app.pid,title=text(hwnd),old_windows_preserved=len(previous),snapshots=snapshots,
        real_settings_unchanged=True),ensure_ascii=False),flush=True)


if __name__ == '__main__': main()
