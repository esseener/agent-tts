"""Compatibility entry point; the application is built entirely with Rust."""
from pathlib import Path
import subprocess

if __name__ == '__main__':
    script = Path(__file__).resolve().with_name('build.ps1')
    subprocess.run(['powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass',
                    '-File', str(script)], check=True)
