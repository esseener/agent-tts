"""Exercise the compiled Windows app under an isolated home; Python is QA only."""
import argparse
import ctypes as C
from ctypes import wintypes as W
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import tomllib
import wave
import zipfile

ROOT = Path(__file__).resolve().parent.parent
VERSION = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding='utf-8'))['package']['version']
u = C.WinDLL('user32', use_last_error=True)
u.FindWindowW.argtypes = [W.LPCWSTR, W.LPCWSTR]
u.FindWindowW.restype = W.HWND
u.GetDlgItem.argtypes = [W.HWND, C.c_int]
u.GetDlgItem.restype = W.HWND
u.SendMessageW.argtypes = [W.HWND, W.UINT, W.WPARAM, W.LPARAM]
u.SendMessageW.restype = W.LPARAM
u.PostMessageW.argtypes = [W.HWND, W.UINT, W.WPARAM, W.LPARAM]
u.SetWindowTextW.argtypes = [W.HWND, W.LPCWSTR]
u.GetWindowTextW.argtypes = [W.HWND, W.LPWSTR, C.c_int]
u.GetClassNameW.argtypes = [W.HWND, W.LPWSTR, C.c_int]
u.GetWindowRect.argtypes = [W.HWND, C.POINTER(W.RECT)]
u.MoveWindow.argtypes = [W.HWND, C.c_int, C.c_int, C.c_int, C.c_int, W.BOOL]
u.SetForegroundWindow.argtypes = [W.HWND]
u.GetWindowThreadProcessId.argtypes = [W.HWND, C.POINTER(W.DWORD)]
CALLBACK = C.WINFUNCTYPE(W.BOOL, W.HWND, W.LPARAM)
u.EnumWindows.argtypes = [CALLBACK, W.LPARAM]
checks = []
limitations = []


def check(condition, name):
    if not condition:
        raise AssertionError(name)
    checks.append(name)


def wait_for(fn, timeout=8):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        value = fn()
        if value:
            return value
        time.sleep(.03)
    raise AssertionError('Timed out: ' + getattr(fn, '__name__', 'condition'))


def window_for(pid, cls='AgentVoiceNativeWindow'):
    found = []
    @CALLBACK
    def visit(hwnd, _):
        process = W.DWORD()
        u.GetWindowThreadProcessId(hwnd, C.byref(process))
        name = C.create_unicode_buffer(128)
        u.GetClassNameW(hwnd, name, 128)
        if process.value == pid and name.value == cls:
            found.append(hwnd)
        return True
    u.EnumWindows(visit, 0)
    return found[0] if found else None


def fingerprints():
    base = Path.home()
    names = ['.workbuddy/settings.json', '.claude/settings.json', '.codex/hooks.json',
             '.gemini/settings.json', '.cursor/hooks.json', '.workbuddy/voice/agents.json',
             '.workbuddy/voice/app-bindings.json', '.agent-voice/config.json',
             '.config/opencode/plugins/agent-voice/index.js', '.config/opencode/plugins/agent-voice.js']
    return {n: hashlib.sha256((base / n).read_bytes()).hexdigest()
            if (base / n).is_file() else None for n in names}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, default=ROOT / '.build/rust/x86_64-pc-windows-msvc/release/AgentVoice.exe')
    parser.add_argument('--online', action='store_true', help='Also synthesize a real online MP3 and play it')
    args = parser.parse_args()
    exe = args.exe.resolve()
    original = fingerprints()
    temp = Path(os.environ['LOCALAPPDATA']) / 'Temp/opencode'
    temp.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='agent-voice-native-', dir=temp) as folder:
        home = Path(folder)
        def cli(*arguments, data=None, timeout=70):
            result = subprocess.run([str(exe), '--home', str(home), *arguments], input=data,
                                  capture_output=True, encoding='utf-8', timeout=timeout)
            if result.returncode: raise RuntimeError(f'{arguments}: {result.stderr}')
            return result
        check(VERSION in cli('--version').stdout, 'Rust CLI version')
        config = json.loads(cli('--export-config').stdout)
        check(not (home / '.agent-voice').exists(), 'Read-only discovery creates no settings')
        with socket.socket() as probe:
            probe.bind(('127.0.0.1', 0))
            config['api_port'] = probe.getsockname()[1]
        config['profiles'][0]['voice'] = 'sapi:default'
        second = dict(config['profiles'][0], id='alternate', name='另一种声音', rate=12)
        config['profiles'].append(second)
        audio = home / 'notification.wav'
        with wave.open(str(audio), 'wb') as wav:
            wav.setparams((1, 2, 22050, 0, 'NONE', 'not compressed'))
            wav.writeframes(b'\0\0' * 2205)
        for host in config['hosts']:
            for route in host['routes'].values():
                route['audio'] = str(audio)
        settings = home / '.agent-voice/config.json'
        settings.parent.mkdir(parents=True)
        settings.write_text(json.dumps(config, ensure_ascii=False), encoding='utf-8')
        voices = json.loads(cli('--voices').stdout)
        check(any(key.startswith('sapi:') for key, _ in voices), 'Installed offline voices enumerated')
        check(any(key.startswith('edge:') for key, _ in voices), 'Online neural voices available')
        rendered = home / 'offline.wav'
        offline = subprocess.run([str(exe),'--home',str(home),'--render','default','--text','原生语音测试。','--out',str(rendered)],capture_output=True,encoding='utf-8',timeout=20)
        if offline.returncode == 0:
            with wave.open(str(rendered), 'rb') as wav:
                check(wav.getnframes() > 1000, 'Real offline SAPI synthesis')
        else:
            check('Windows 语音失败' in offline.stderr and any(code in offline.stderr for code in ('0x8004503A','0x80070003')), 'Missing offline voice data reported explicitly')
            limitations.append('This machine has registered SAPI voices with missing voice data; real offline synthesis unavailable: ' + offline.stderr.strip())
        if args.online:
            config['profiles'][0].update(voice='edge:zh-CN-XiaoxiaoNeural',rate=-5,pitch=-4)
            config['hosts'][0]['routes']['done']['audio']=''
            settings.write_text(json.dumps(config,ensure_ascii=False),encoding='utf-8')
            cli('--play-event','chatgpt','done','--text','原生在线语音测试。')
            check(any(p.stat().st_size>1000 for p in (home/'.agent-voice/cache').glob('*.mp3')),'Real online synthesis, bounded native subprocess and MP3 playback')
            cli('--play-event','chatgpt','done','--text','原生在线语音测试。')
            check(True,'Native cached audio replay without ffmpeg')
            config['hosts'][0]['routes']['done']['audio']=str(audio)
            config['profiles'][0].update(voice='sapi:default',rate=0,pitch=0)
            settings.write_text(json.dumps(config,ensure_ascii=False),encoding='utf-8')

        app = subprocess.Popen([str(exe), '--home', str(home)])
        try:
            hwnd = wait_for(lambda: window_for(app.pid))
            wait_for(lambda: u.GetDlgItem(hwnd, 211))
            check(app.poll() is None, 'Native desktop window started')
            def button(identifier):
                control=wait_for(lambda:u.GetDlgItem(hwnd,identifier))
                u.SendMessageW(hwnd, 0x111, identifier, control)
            def edit(identifier, value):
                check(bool(u.GetDlgItem(hwnd, identifier)), f'Control {identifier} exists')
                buffer = C.create_unicode_buffer(value)
                u.SendMessageW(u.GetDlgItem(hwnd, identifier), 0x0c, 0, C.addressof(buffer))
            def choose(identifier, index):
                u.SendMessageW(u.GetDlgItem(hwnd, identifier), 0x14e, index, 0)
                u.SendMessageW(hwnd, 0x111, identifier | (1 << 16), u.GetDlgItem(hwnd, identifier))
            def text_of(identifier):
                control=u.GetDlgItem(hwnd,identifier)
                count=u.SendMessageW(control,0x0e,0,0)
                buffer=C.create_unicode_buffer(count+1)
                u.SendMessageW(control,0x0d,count+1,C.addressof(buffer))
                return buffer.value
            check(text_of(101)=='选择宿主','Default sidebar explicitly selects hosts')
            check(text_of(10)=='宿主声音' and text_of(11)=='声音预设','Host controls distinguished from voice presets')
            check(u.SendMessageW(u.GetDlgItem(hwnd,100),0x18b,0,0)==len(config['hosts']),'Sidebar contains actual hosts rather than voice profiles')
            check(u.SendMessageW(hwnd,0x7f,1,0)!=0 and u.SendMessageW(hwnd,0x7f,0,0)!=0,'Separate taskbar and title-bar icons assigned')
            # Select Codex explicitly; ChatGPT remains visible at the top.
            u.SendMessageW(u.GetDlgItem(hwnd,100),0x186,1,0)
            u.SendMessageW(hwnd,0x111,100|(1<<16),u.GetDlgItem(hwnd,100))
            choose(211, 2)
            choose(221, 1)
            edit(212, '独立完成台词')
            edit(232, '独立错误台词')
            button(2)
            saved = json.loads(settings.read_text(encoding='utf-8'))
            codex = next(h for h in saved['hosts'] if h['id'] == 'codex')
            check(codex['routes']['done']['profile'] == 'alternate', 'Host done voice independently selectable')
            check(codex['routes']['subagent']['profile'] == 'default', 'Host subagent voice independent')
            check(codex['routes']['done']['text'] == '独立完成台词', f'Host-specific phrase saved: {codex["routes"]["done"]["text"]!r}')
            check(codex['routes']['error']['text'] == '独立错误台词', 'Error phrase independent')
            check(len(list(settings.parent.glob('config.json.bak-*'))) > 0, 'Atomic save with backup')
            # Capture actual native UI, including compact window layout.
            try:
                from PIL import Image
                previews = ROOT / 'docs/previews'
                previews.mkdir(parents=True, exist_ok=True)
                def screenshot(name):
                    time.sleep(.1)
                    rect = W.RECT()
                    u.GetWindowRect(hwnd, C.byref(rect))
                    width,height=rect.right-rect.left,rect.bottom-rect.top
                    g=C.WinDLL('gdi32')
                    u.GetWindowDC.argtypes=[W.HWND];u.GetWindowDC.restype=W.HDC
                    u.ReleaseDC.argtypes=[W.HWND,W.HDC]
                    u.PrintWindow.argtypes=[W.HWND,W.HDC,W.UINT]
                    g.CreateCompatibleDC.argtypes=[W.HDC];g.CreateCompatibleDC.restype=W.HDC
                    g.CreateCompatibleBitmap.argtypes=[W.HDC,C.c_int,C.c_int];g.CreateCompatibleBitmap.restype=W.HBITMAP
                    g.SelectObject.argtypes=[W.HDC,W.HANDLE];g.SelectObject.restype=W.HANDLE
                    g.DeleteObject.argtypes=[W.HANDLE];g.DeleteDC.argtypes=[W.HDC]
                    g.GetDIBits.argtypes=[W.HDC,W.HBITMAP,W.UINT,W.UINT,C.c_void_p,C.c_void_p,W.UINT]
                    class Header(C.Structure):
                        _fields_=[('size',W.DWORD),('width',W.LONG),('height',W.LONG),('planes',W.WORD),('bitcount',W.WORD),('compression',W.DWORD),('imagesize',W.DWORD),('xppm',W.LONG),('yppm',W.LONG),('used',W.DWORD),('important',W.DWORD)]
                    header=Header(40,width,-height,1,32,0,0,0,0,0,0)
                    screen=u.GetWindowDC(hwnd);memory=g.CreateCompatibleDC(screen);bitmap=g.CreateCompatibleBitmap(screen,width,height)
                    prior=g.SelectObject(memory,bitmap)
                    try:
                        check(bool(u.PrintWindow(hwnd,memory,0)),'Native window captured without other applications')
                        pixels=C.create_string_buffer(width*height*4)
                        g.GetDIBits(memory,bitmap,0,height,pixels,C.byref(header),0)
                        Image.frombytes('RGB',(width,height),pixels.raw,'raw','BGRX').save(previews/name)
                    finally:
                        g.SelectObject(memory,prior);g.DeleteObject(bitmap);g.DeleteDC(memory);u.ReleaseDC(hwnd,screen)
                screenshot('native-hosts.png')
                u.MoveWindow(hwnd, 60, 40, 960, 680, True)
                screenshot('native-compact.png')
                u.MoveWindow(hwnd, 60, 40, 1156, 829, True)
            except ImportError:
                pass
            button(11)
            check(text_of(101)=='声音预设','Preset page labeled independently')
            button(10)
            check(text_of(200)=='Codex','Returning from presets retains the selected host')
            button(11)
            edit(300, '离线声音测试')
            button(316)
            choose(313, 0)
            choose(301, next(i for i,(key,_) in enumerate(voices) if key=='sapi:default'))
            edit(308, '完成第一句\r\n完成第二句')
            choose(307, 2)
            edit(308, '错误专属台词')
            choose(307, 0)
            text = C.create_unicode_buffer(256)
            u.SendMessageW(u.GetDlgItem(hwnd, 308), 0x0d, 256, C.addressof(text))
            check('完成第二句' in text.value, 'Switching events retains phrase drafts')
            button(2)
            saved = json.loads(settings.read_text(encoding='utf-8'))
            check(saved['profiles'][0]['phrases']['error'] == ['错误专属台词'], 'Voice-library event phrases persist')
            if 'Image' in locals(): screenshot('native-voices.png')
            button(12)
            edit(400, 'my-harness')
            edit(401, '我的 Harness')
            button(402)
            button(2)
            saved = json.loads(settings.read_text(encoding='utf-8'))
            check(any(h['id'] == 'my-harness' for h in saved['hosts']), 'Custom host added and saved')
            button(11)
            button(302)
            edit(300,'可删除的声音')
            button(2)
            u.PostMessageW(hwnd,0x111,303,u.GetDlgItem(hwnd,303))
            dialog=wait_for(lambda:window_for(app.pid,'#32770'))
            u.PostMessageW(dialog,0x111,6,u.GetDlgItem(dialog,6))
            wait_for(lambda:not window_for(app.pid,'#32770'))
            button(2)
            check(len(json.loads(settings.read_text(encoding='utf-8'))['profiles'])==2,'Voice profile created and deleted through native UI')
            button(12)
            button(404)
            saved = json.loads(settings.read_text(encoding='utf-8'))
            check(saved['api_enabled'], 'HTTP interface enabled from native UI')
            if 'Image' in locals(): screenshot('native-api.png')
            def request(body, token=None):
                payload = json.dumps(body, ensure_ascii=False).encode('utf-8')
                with socket.create_connection(('127.0.0.1', saved['api_port']), timeout=3) as client:
                    header = (f'POST /v1/events HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token or saved["api_token"]}\r\nContent-Length: {len(payload)}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n').encode()
                    client.sendall(header + payload)
                    chunks=[]
                    while True:
                        try:
                            chunk=client.recv(8192)
                        except OSError as error:
                            raise AssertionError(f'HTTP transport failed: {error}; received={b"".join(chunks)!r}; app_exit={app.poll()}; status={text_of(90)!r}') from error
                        if not chunk: break
                        chunks.append(chunk)
                        received=b''.join(chunks)
                        if b'\r\n\r\n' in received:
                            response_header,response_body=received.split(b'\r\n\r\n',1)
                            length=next((int(line.split(b':',1)[1]) for line in response_header.split(b'\r\n') if line.lower().startswith(b'content-length:')),None)
                            if length is not None and len(response_body)>=length: break
                    return b''.join(chunks)
            check(b'400' in request({'host':'chatgpt','event':'done'}, 'wrong'), 'HTTP rejects invalid access token')
            check(b'202' in request({'host':'chatgpt','event':'done'}), 'HTTP accepts authenticated ChatGPT event')
            activity=home/'.agent-voice/activity.jsonl'
            wait_for(lambda:activity.is_file())
            check(activity.is_file(),'Event activity log exists')
            def played_entries():
                try:
                    entries=[json.loads(line) for line in activity.read_text(encoding='utf-8').splitlines()]
                    return entries if any(e['host']=='chatgpt' and e['event']=='done' and e['ok'] and e['message']=='音效已播放' for e in entries) else None
                except (OSError,ValueError):
                    return None
            entries=wait_for(played_entries)
            check(any(e['host']=='chatgpt' and e['event']=='done' and e['ok'] and e['message']=='音效已播放' for e in entries),'HTTP event reaches native audio playback')
            button(404)
            check(not json.loads(settings.read_text(encoding='utf-8'))['api_enabled'], 'HTTP interface stops cleanly')
            button(2)
        finally:
            if app.poll() is None:
                u.PostMessageW(window_for(app.pid), 0x10, 0, 0)
                try: app.wait(5)
                except subprocess.TimeoutExpired: app.terminate(); app.wait()
        check(app.returncode == 0, 'Native app closes without crash')
        # All adapters preserve unrelated settings in the injected home.
        targets = {'codex':'.codex/hooks.json','claude':'.claude/settings.json',
                   'workbuddy':'.workbuddy/settings.json','cursor':'.cursor/hooks.json',
                   'gemini':'.gemini/settings.json'}
        for host, relative in targets.items():
            path = home / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            sentinel = {'keep': {'enabled': True}, 'hooks': {'Unrelated': [{'command':'keep-me'}]}}
            path.write_text(json.dumps(sentinel), encoding='utf-8')
            cli('--install-hook', host)
            installed = json.loads(path.read_text(encoding='utf-8'))
            check(installed['keep'] == sentinel['keep'] and installed['hooks']['Unrelated'] == sentinel['hooks']['Unrelated'], host + ' preserves unrelated configuration')
            cli('--install-hook', host)
            check(json.loads(path.read_text(encoding='utf-8')) == installed, host + ' hook install is idempotent')
            event = 'stop' if host == 'cursor' else 'AfterAgent' if host == 'gemini' else 'Stop'
            entry = installed['hooks'][event][0]
            handler = entry if host == 'cursor' else entry['hooks'][0]
            command = handler.get('commandWindows', handler['command'])
            shell = ['powershell', '-NoProfile', '-Command', command] if host in ('cursor','codex') else 'cmd.exe /d /s /c "'+command+'"'
            result = subprocess.run(shell, input=b'{}', capture_output=True, timeout=10)
            check(result.returncode == 0 and result.stdout.strip() == b'{}', host + ' generated Windows command executes')
            cli('--remove-hook', host)
            check(json.loads(path.read_text(encoding='utf-8'))['hooks'] == sentinel['hooks'], host + ' removes only Agent Voice hooks')
        cli('--install-hook', 'opencode')
        plugin = home / '.config/opencode/plugins/agent-voice/index.js'
        check('Plugin.define' in plugin.read_text(encoding='utf-8'), 'OpenCode uses V2 Plugin.define')
        subprocess.run(['node', '--check', str(plugin)], capture_output=True, check=True)
        check(True, 'OpenCode generated plugin syntax valid')
        subprocess.run(['node',str(ROOT/'tools/check_plugin.mjs'),str(plugin)],capture_output=True,check=True)
        check(True,'OpenCode V2 success, failure and permission events dispatch correctly')
        cli('--remove-hook', 'opencode')
        check(not plugin.exists(), 'OpenCode plugin removed with backup')
        # Migrate a real-shaped legacy fixture without mutating its source.
        legacyhome = home / 'legacy'
        legacyfile = legacyhome / '.workbuddy/voice/agents.json'
        legacyfile.parent.mkdir(parents=True)
        fixture = {'defaults':{'voice':'zh-CN-YunxiNeural'}, 'agents':{'codes':{'name':'codes','phrases':['codes 完成了'],'ask_phrases':['等你确认'],'rate':'-10%'}}, 'events':{'error':{'enabled':False}}}
        raw = json.dumps(fixture, ensure_ascii=False).encode()
        legacyfile.write_bytes(raw)
        migrated = subprocess.run([str(exe), '--home',str(legacyhome),'--export-config'], capture_output=True, encoding='utf-8', check=True)
        migrated = json.loads(migrated.stdout)
        check(migrated['profiles'][0]['name'] == 'Codex' and migrated['profiles'][0]['rate'] == -10, 'Legacy profile migration')
        check(all(not h['routes']['error']['enabled'] for h in migrated['hosts']), 'Legacy mute state migration')
        check(legacyfile.read_bytes() == raw and not (legacyhome / '.agent-voice').exists(), 'Legacy migration is read-only until saved')
        time.sleep(.6)
    after=fingerprints()
    changed=[name for name in original if original[name]!=after[name]]
    if changed: print('Files changed concurrently during verification:',', '.join(changed))
    check(not changed, 'Real user settings unchanged byte-for-byte')
    report = {'checks': len(checks), 'passed': checks, 'limitations': limitations, 'executable': str(exe), 'sha256': hashlib.sha256(exe.read_bytes()).hexdigest()}
    (ROOT / 'docs/native-verification.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(f'{len(checks)} native release checks passed; real user settings unchanged.')


if __name__ == '__main__':
    main()
