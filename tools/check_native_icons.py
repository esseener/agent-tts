"""Verify exact embedded/loaded icon sizes and the live window's taskbar icon."""
import argparse
import ctypes as C
from ctypes import wintypes as W
import hashlib
from io import BytesIO
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import zipfile

from PIL import Image, ImageDraw
import pefile
from smoke_native import VERSION, fingerprints, u, wait_for, window_for

ROOT = Path(__file__).resolve().parent.parent
SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
g = C.WinDLL('gdi32', use_last_error=True)
k = C.WinDLL('kernel32', use_last_error=True)
k.LoadLibraryExW.argtypes = [W.LPCWSTR, W.HANDLE, W.DWORD]
k.LoadLibraryExW.restype = W.HMODULE
k.FreeLibrary.argtypes = [W.HMODULE]
u.LoadImageW.argtypes = [W.HINSTANCE, C.c_void_p, W.UINT, C.c_int, C.c_int, W.UINT]
u.LoadImageW.restype = W.HICON
u.DestroyIcon.argtypes = [W.HICON]
u.GetDC.argtypes = [W.HWND]
u.GetDC.restype = W.HDC
u.ReleaseDC.argtypes = [W.HWND, W.HDC]
u.GetDpiForWindow.argtypes = [W.HWND]
u.GetDpiForWindow.restype = W.UINT
u.GetSystemMetricsForDpi.argtypes = [C.c_int, W.UINT]
g.GetObjectW.argtypes = [W.HANDLE, C.c_int, C.c_void_p]
g.GetDIBits.argtypes = [W.HDC, W.HBITMAP, W.UINT, W.UINT, C.c_void_p, C.c_void_p, W.UINT]
g.DeleteObject.argtypes = [W.HANDLE]


class IconInfo(C.Structure):
    _fields_ = [('icon', W.BOOL), ('xhotspot', W.DWORD), ('yhotspot', W.DWORD),
                ('mask', W.HBITMAP), ('color', W.HBITMAP)]


class Bitmap(C.Structure):
    _fields_ = [('type', W.LONG), ('width', W.LONG), ('height', W.LONG),
                ('widthbytes', W.LONG), ('planes', W.WORD), ('bits', W.WORD),
                ('data', C.c_void_p)]


class Header(C.Structure):
    _fields_ = [('size', W.DWORD), ('width', W.LONG), ('height', W.LONG),
                ('planes', W.WORD), ('bitcount', W.WORD), ('compression', W.DWORD),
                ('imagesize', W.DWORD), ('xppm', W.LONG), ('yppm', W.LONG),
                ('used', W.DWORD), ('important', W.DWORD)]


u.GetIconInfo.argtypes = [W.HICON, C.POINTER(IconInfo)]


def icon_pixels(icon):
    info = IconInfo()
    assert icon and u.GetIconInfo(icon, C.byref(info)), 'Icon handle is valid'
    screen = u.GetDC(None)
    try:
        bitmap = Bitmap()
        assert g.GetObjectW(info.color, C.sizeof(bitmap), C.byref(bitmap))
        header = Header(40, bitmap.width, -bitmap.height, 1, 32, 0, 0, 0, 0, 0, 0)
        buffer = C.create_string_buffer(bitmap.width * bitmap.height * 4)
        assert g.GetDIBits(screen, info.color, 0, bitmap.height, buffer, C.byref(header), 0)
        return Image.frombytes('RGBA', (bitmap.width, bitmap.height), buffer.raw, 'raw', 'BGRA')
    finally:
        u.ReleaseDC(None, screen)
        g.DeleteObject(info.color)
        g.DeleteObject(info.mask)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, default=ROOT / 'AgentVoice.exe')
    args = parser.parse_args()
    exe = args.exe.resolve()
    executable_bytes = exe.read_bytes()
    original = fingerprints()
    assets = ROOT / 'assets'
    branding = json.loads((assets / 'branding.json').read_text(encoding='utf-8'))
    assert hashlib.sha256((ROOT / branding['source']).read_bytes()).hexdigest() == branding['source_sha256']
    for name, size in branding['forms'].items():
        image = Image.open(assets / name)
        assert image.mode == 'RGBA' and list(image.size) == size, name
        assert all(channel.getextrema() == (255, 255) for channel in image.split()[:3]), name + ' is pure white'
        assert image.getchannel('A').getextrema() == (0, 255), name + ' has transparency and opaque strokes'
        assert image.getpixel((0, 0))[3] == 0 and image.getpixel((image.width - 1, image.height - 1))[3] == 0
    header = (assets / 'logo-256.bgra').read_bytes()
    assert len(header) == 256 * 256 * 4 and header in executable_bytes, 'Current high-DPI header artwork embedded'
    ico = Image.open(ROOT / 'assets/AgentVoice.ico')
    assert sorted(width for width, height in ico.ico.sizes()) == SIZES
    pe = pefile.PE(str(exe))
    expected = tuple(map(int, VERSION.split('.'))) + (0,)
    fixed = pe.VS_FIXEDFILEINFO[0]
    actual = (fixed.FileVersionMS >> 16, fixed.FileVersionMS & 65535,
              fixed.FileVersionLS >> 16, fixed.FileVersionLS & 65535)
    assert actual == expected, ('Windows file version', actual, expected)
    strings = {key.decode(): value.decode() for block in pe.FileInfo for entry in block
               if getattr(entry, 'Key', b'') == b'StringFileInfo'
               for table in entry.StringTable for key, value in table.entries.items()}
    assert strings['FileVersion'] == strings['ProductVersion'] == VERSION
    resources = {entry.id: entry for entry in pe.DIRECTORY_ENTRY_RESOURCE.entries}
    assembly = resources[24].directory.entries[0].directory.entries[0].data.struct
    assert f'version="{VERSION}.0"'.encode() in pe.get_data(assembly.OffsetToData, assembly.Size)
    embedded = resources[3].directory.entries
    assert len(embedded) == len(SIZES)
    embedded_frames = {}
    for entry in embedded:
        data = entry.directory.entries[0].data.struct
        image = Image.open(BytesIO(pe.get_data(data.OffsetToData, data.Size))).convert('RGBA')
        embedded_frames[image.width] = image
    assert sorted(embedded_frames) == SIZES
    # Load each requested physical size through the Windows resource loader.
    module = k.LoadLibraryExW(str(exe), None, 2)
    assert module
    loaded = {}
    try:
        for size in SIZES:
            icon = u.LoadImageW(module, 1, 1, size, size, 0)
            try:
                image = icon_pixels(icon)
                assert image.size == (size, size)
                expected = ico.ico.getimage((size, size)).getchannel('A').tobytes()
                assert image.getchannel('A').tobytes() == expected, f'{size}px loaded without resampling'
                assert embedded_frames[size].getchannel('A').tobytes() == expected
                loaded[size] = image
            finally:
                u.DestroyIcon(icon)
    finally:
        k.FreeLibrary(module)
    assert all(sum(value >= 220 for value in loaded[size].getchannel('A').tobytes()) >= size
               for size in SIZES[:7]), 'Small icon strokes have opaque cores'
    with tempfile.TemporaryDirectory(prefix='agent-voice-icons-', dir=Path(os.environ['LOCALAPPDATA']) / 'Temp/opencode') as folder:
        standalone = Path(folder) / 'AgentVoice.exe'
        shutil.copy2(exe, standalone)
        shutil.copytree(ROOT / 'resources/kokoro', Path(folder) / 'resources/kokoro')
        app = subprocess.Popen([str(standalone), '--home', str(Path(folder) / 'home')], cwd=folder)
        try:
            hwnd = wait_for(lambda: window_for(app.pid))
            wait_for(lambda: u.GetDlgItem(hwnd, 211))
            dpi = u.GetDpiForWindow(hwnd)
            taskbar = icon_pixels(u.SendMessageW(hwnd, 0x7f, 1, 0))
            titlebar = icon_pixels(u.SendMessageW(hwnd, 0x7f, 0, 0))
            assert taskbar.size == (u.GetSystemMetricsForDpi(11, dpi), u.GetSystemMetricsForDpi(12, dpi))
            assert titlebar.size == (u.GetSystemMetricsForDpi(49, dpi), u.GetSystemMetricsForDpi(50, dpi))
            assert taskbar.getchannel('A').tobytes() == loaded[taskbar.width].getchannel('A').tobytes()
            assert titlebar.getchannel('A').tobytes() == loaded[titlebar.width].getchannel('A').tobytes()
            preview = Image.new('RGB', (440, 170), (15, 15, 15))
            draw = ImageDraw.Draw(preview)
            for x, name, image in [(20, 'Taskbar', taskbar), (240, 'Title bar', titlebar)]:
                preview.paste(image, (x, 20), image)
                enlarged = image.resize((image.width * 2, image.height * 2), Image.Resampling.NEAREST)
                preview.paste(enlarged, (x + 65, 20), enlarged)
                draw.text((x, 126), f'{name}: {image.width}px', fill='white')
            preview.save(ROOT / 'docs/previews/runtime-icons.png')
            u.PostMessageW(hwnd, 0x10, 0, 0)
            app.wait(5)
            assert app.returncode == 0
        finally:
            if app.poll() is None:
                app.terminate()
                app.wait()
    package = ROOT / f'dist/AgentVoice-{VERSION}.zip'
    with zipfile.ZipFile(package) as zipped:
        assert zipped.read(f'AgentVoice-{VERSION}/AgentVoice.exe') == executable_bytes
        assert f'Agent Voice {VERSION}' in zipped.read(f'AgentVoice-{VERSION}/使用说明.md').decode('utf-8')
        for name in [*branding['forms'], 'AgentVoice.ico', 'README.md', 'branding.json']:
            assert zipped.read(f'AgentVoice-{VERSION}/Logo/' + name) == (assets / name).read_bytes(), name
        package_contents = zipped.namelist()
    assert fingerprints() == original
    report = {'sha256': hashlib.sha256(executable_bytes).hexdigest(),
               'embedded_sizes': SIZES, 'loaded_without_resampling': SIZES,
               'dpi': dpi, 'live_taskbar_size': taskbar.size, 'live_titlebar_size': titlebar.size,
               'package_matches': True, 'real_user_settings_unchanged': True,
               'white_transparent_forms': branding['forms'], 'embedded_header_size': 256,
               'source_sha256': branding['source_sha256'], 'portable_start_without_logo_assets': True}
    (ROOT / 'docs/icon-verification.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    version = pe.VS_FIXEDFILEINFO[0]
    package_report = {
        'version': VERSION, 'bytes': len(executable_bytes), 'sha256': report['sha256'],
        'windows_file_version': [version.FileVersionMS >> 16, version.FileVersionMS & 0xffff,
                                 version.FileVersionLS >> 16, version.FileVersionLS & 0xffff],
        'embedded_icon_sizes': len(SIZES), 'embedded_header_size': 256,
        'imports': [entry.dll.decode() for entry in pe.DIRECTORY_ENTRY_IMPORT],
        'zip_contents': package_contents, 'portable_start_without_logo_assets': True,
        'local_voice_resources_required': True,
        'all_logo_forms_match': True, 'real_settings_unchanged': True,
    }
    (ROOT / 'docs/native-package-verification.json').write_text(json.dumps(package_report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(f'All {len(SIZES)} embedded icon sizes load exactly; live taskbar {taskbar.width}px, title bar {titlebar.width}px; portable ZIP matches.')


if __name__ == '__main__':
    main()
