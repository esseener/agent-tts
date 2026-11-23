"""Time hook/event -> Windows playback submission with the model absent.

The trace marks the MCI play command, not sound measured at a speaker/mic.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics
import socket
import subprocess
import tempfile
import time
import wave
import urllib.request

import psutil
from smoke_native import fingerprints, VERSION, u, wait_for, window_for

ROOT = Path(__file__).resolve().parent.parent


def main():
    original = fingerprints()
    manifest = json.loads((ROOT/'resources/notifications/manifest.json').read_text(encoding='utf-8'))
    assert len(manifest['clips']) == 24
    for clip in manifest['clips']:
        path = ROOT/'resources/notifications'/clip['file']
        assert hashlib.sha256(path.read_bytes()).hexdigest() == clip['sha256']
    timings = []
    with tempfile.TemporaryDirectory(prefix='agent-voice-latency-',dir=Path(os.environ['LOCALAPPDATA'])/'Temp/opencode') as folder:
        portable = Path(folder)/'中文 通知速度 🎵'
        portable.mkdir()
        exe = portable/'AgentVoice.exe'
        shutil.copy2(ROOT/'AgentVoice.exe',exe)
        shutil.copytree(ROOT/'resources/notifications',portable/'resources/notifications')
        home = Path(folder)/'home'
        trace = Path(folder)/'playback.jsonl'
        env = dict(os.environ,AGENT_VOICE_PLAYBACK_TRACE=str(trace),PATH=str(Path(os.environ['WINDIR'])/'System32'),
            HTTP_PROXY='http://127.0.0.1:9',HTTPS_PROXY='http://127.0.0.1:9')
        def cli(*args):
            result = subprocess.run([str(exe),'--home',str(home),*args],capture_output=True,encoding='utf-8',env=env,timeout=10)
            assert result.returncode == 0, result.stderr
            return json.loads(result.stdout) if result.stdout.strip() else None
        config = cli('--export-config')
        settings = home/'.agent-voice/config.json'
        settings.parent.mkdir(parents=True)
        def save():settings.write_text(json.dumps(config,ensure_ascii=False),encoding='utf-8')
        save()
        # No model or runtime is shipped in this test directory. A cache miss
        # must still work on the very first hook without a GUI or prewarming.
        assert not (portable/'resources/kokoro').exists()
        for clip in manifest['clips']:
            config['profiles'][0]['voice'] = clip['voice']
            save()
            planned = cli('--notification-plan','codex',clip['event'],'--text','从来没有合成过的临时台词。')
            assert planned['plan']['fallback'] and planned['plan']['text'] == clip['text']
            assert planned['plan']['voice'] == clip['voice']
        config['profiles'][0]['voice'] = 'local:kokoro-v1.1:zf_001'
        save()

        def run_play(command,event,text,label):
            trace.unlink(missing_ok=True)
            started_ms = time.time()*1000
            command_start = time.perf_counter()
            app = subprocess.Popen([str(exe),'--home',str(home),command,'codex',event,'--text',text],
                env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,encoding='utf-8')
            app.stdin.close()
            app.stdin = None
            peak_rss = 0
            loaded_model = False
            while not trace.exists():
                if app.poll() is not None and (command != '--hook' or app.returncode != 0):
                    stdout,stderr=app.communicate()
                    raise AssertionError(f'Player exited before playback: {app.returncode}; {stderr}; {stdout}')
                try:
                    p = psutil.Process(app.pid)
                    peak_rss = max(peak_rss,p.memory_info().rss)
                except psutil.NoSuchProcess:
                    pass
                if time.perf_counter()-command_start > 3:
                    app.kill()
                    raise AssertionError('Event did not submit audio within 3 seconds')
                time.sleep(.005)
            deadline = time.monotonic()+2
            while True:
                try:
                    row=json.loads(trace.read_text(encoding='utf-8').splitlines()[0])
                    break
                except (json.JSONDecodeError,IndexError):
                    assert time.monotonic()<deadline
                    time.sleep(.005)
            try:
                player=psutil.Process(row['pid'])
                peak_rss=max(peak_rss,player.memory_info().rss)
                loaded_model=any('sherpa' in m.path.lower() or 'onnxruntime' in m.path.lower() for m in player.memory_maps())
            except psutil.NoSuchProcess:
                pass
            stdout,stderr=app.communicate(timeout=10)
            assert app.returncode == 0,stderr
            # A hook returns before its child finishes playing; wait for that
            # one child only so later cases measure an uncontended audio device.
            if row['pid'] != app.pid:
                try:psutil.Process(row['pid']).wait(timeout=10)
                except psutil.NoSuchProcess:pass
            lag=row['unix_ms']-started_ms
            assert lag < 1500,(label,lag)
            assert not loaded_model
            timings.append(dict(case=label,launch_to_play_command_ms=round(lag,2),
                player_process_ms=row['process_ms'],sampled_peak_rss_mib=round(peak_rss/2**20,2),model_loaded=False))

        for event in ['done','ask','error','subagent']:
            run_play('--play-event',event,'未缓存的动态文字。','empty-cache-'+event)
        run_play('--hook','done','未缓存的动态文字。','real-hook-spawn')
        # Exact cached user text wins; do not replace it with the canned phrase.
        text='这是用户自定义台词，必须优先使用。'
        planned=cli('--notification-plan','codex','done','--text',text)
        target=Path(planned['requested_audio'])
        target.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(portable/'resources/notifications/zf_001-done.wav',target)
        exact=cli('--notification-plan','codex','done','--text',text)
        assert not exact['plan']['fallback'] and exact['plan']['text']==text
        assert Path(exact['plan']['path'])==target
        run_play('--play-event','done',text,'exact-cached-custom-text')
        for event in ['done','ask','subagent','error']:
            config['profiles'][0]['phrases'][event]=[]
        config['profiles'][0]['phrases']['done']=[f'预生成台词 {n}' for n in range(70)]
        save()
        for text in config['profiles'][0]['phrases']['done']:
            path=Path(cli('--notification-plan','codex','done','--text',text)['requested_audio'])
            shutil.copy2(target,path)
        assert cli('--warm-cache')['prepared']==70, 'Background preparation must not stop after 64 cached phrases'
        config['profiles'][0]['volume']=0
        save()
        muted=cli('--notification-plan','codex','ask','--text','静音测试')
        with wave.open(muted['plan']['path'],'rb') as audio:
            assert set(audio.readframes(audio.getnframes()))=={0}
        config['hosts'][1]['routes']['done']['enabled']=False
        save()
        assert cli('--notification-plan','codex','done')['plan'] is None
        # A real GUI preview starts the CPU model. Its preparation must not
        # hold the playback mutex or block HTTP event dispatch.
        gui_home=Path(folder)/'preview-home'
        gui_config=json.loads(subprocess.run([str(ROOT/'AgentVoice.exe'),'--home',str(gui_home),'--export-config'],
            capture_output=True,encoding='utf-8',check=True).stdout)
        with socket.socket() as probe:
            probe.bind(('127.0.0.1',0))
            gui_config['api_port']=probe.getsockname()[1]
        gui_config['api_enabled']=True
        gui_config['profiles'][0]['phrases']={event:[] for event in ['done','ask','subagent','error']}
        gui_config['profiles'][0]['name']='正在验证后台模型准备不阻塞任务通知'
        gui_settings=gui_home/'.agent-voice/config.json'
        gui_settings.parent.mkdir(parents=True)
        gui_settings.write_text(json.dumps(gui_config,ensure_ascii=False),encoding='utf-8')
        trace.unlink(missing_ok=True)
        gui=subprocess.Popen([str(ROOT/'AgentVoice.exe'),'--home',str(gui_home)],env=env,
            stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        try:
            hwnd=wait_for(lambda:window_for(gui.pid))
            u.SendMessageW(hwnd,0x111,11,wait_for(lambda:u.GetDlgItem(hwnd,11)))
            u.SendMessageW(hwnd,0x111,310,wait_for(lambda:u.GetDlgItem(hwnd,310)))
            started_ms=time.time()*1000
            body=json.dumps({'host':'codex','event':'ask','text':'这句话没有缓存，但通知必须先响。'}).encode()
            request=urllib.request.Request(f'http://127.0.0.1:{gui_config["api_port"]}/v1/events',data=body,
                headers={'Authorization':'Bearer '+gui_config['api_token'],'Content-Type':'application/json'})
            with urllib.request.urlopen(request,timeout=3) as response:assert response.status==202
            def notification_trace():
                if trace.exists():
                    for line in trace.read_text(encoding='utf-8').splitlines():
                        try:row=json.loads(line)
                        except json.JSONDecodeError:continue
                        if 'notifications' in row['path'] and row['pid']==gui.pid:return row
                return None
            row=wait_for(notification_trace,timeout=3)
            lag=row['unix_ms']-started_ms
            assert lag<1500,lag
            timings.append(dict(case='http-during-real-model-preview',launch_to_play_command_ms=round(lag,2),
                player_process_ms=row['process_ms'],model_preview_running=True))
        finally:
            gui.terminate()
            gui.wait(timeout=10)
    assert fingerprints()==original
    report=dict(version=VERSION,model_absent=True,inference_runtime_absent=True,network_blocked=True,portable_unicode_path=True,
        preset_clips=24,checked_voice_state_plans=24,exact_cached_text_prioritized=True,volume_zero_is_silent=True,
        disabled_route_stays_silent=True,real_settings_unchanged=True,timing_boundary='Process launch to MCI play command; physical device output latency excluded.',
        seventy_phrase_preparation_verified=True,
        median_launch_to_play_command_ms=round(statistics.median(t['launch_to_play_command_ms'] for t in timings),2),
        maximum_launch_to_play_command_ms=max(t['launch_to_play_command_ms'] for t in timings),timings=timings)
    (ROOT/f'docs/notification-latency-{VERSION}.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(report,ensure_ascii=False,indent=2),flush=True)


if __name__=='__main__':main()
