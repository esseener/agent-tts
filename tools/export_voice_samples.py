"""Render reviewable short phrases using the shipped app, plus the old engine.

This creates listening material, not an automated naturalness score.
"""
import copy
import base64
import argparse
import html
import json
from pathlib import Path
import subprocess
import wave

ROOT=Path(__file__).resolve().parent.parent
OUTPUT=ROOT/'docs/voice-samples-0.4.0'
TEXT='好啦，任务完成了，来看看结果吧。'
ASK='需要你确认一下，我们再继续。'


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--page-only',action='store_true',help='Reuse existing WAVs when updating the listening page')
    args=parser.parse_args()
    if args.page_only:
        write_page(json.loads((OUTPUT/'samples.json').read_text(encoding='utf-8')))
        return
    OUTPUT.mkdir(parents=True,exist_ok=True)
    exe=ROOT/'AgentVoice.exe'
    cli=lambda *a:json.loads(subprocess.run([str(exe),*a],capture_output=True,encoding='utf-8',check=True).stdout)
    recipes=cli('--voice-selections')
    config=cli('--export-config')
    batch=[]
    records=[]
    for i,recipe in enumerate(recipes):
        p=copy.deepcopy(config['profiles'][0]);p.update(voice=recipe['voice'],rate=recipe['rate'],pitch=recipe['pitch'],mixer=recipe['mixer'],volume=100)
        file=OUTPUT/f'{i+1:02}-{recipe["voice"].rsplit(":",1)[1]}.wav'
        batch.append((p,TEXT,str(file)))
        records.append(dict(model='Kokoro v1.1-zh INT8',voice=p['voice'],title=recipe['title'],text=TEXT,file=file.name))
        if i<2:
            file=OUTPUT/f'{i+1:02}-ask.wav'
            batch.append((p,ASK,str(file)))
            records.append(dict(model='Kokoro v1.1-zh INT8',voice=p['voice'],title=recipe['title']+' · 等待确认',text=ASK,file=file.name))
    result=subprocess.run([str(exe),'--render-batch-json'],input=json.dumps(batch,ensure_ascii=False),capture_output=True,encoding='utf-8',timeout=600)
    assert result.returncode==0,result.stderr
    previous=ROOT/'dist/AgentVoice-0.3.0/AgentVoice.exe'
    for sid,title in [(33,'旧版 · 听晓'),(10,'旧版 · 景川')]:
        p=copy.deepcopy(config['profiles'][0]);p.update(voice=f'local:aishell3:{sid}',rate=0,pitch=0,volume=100)
        p['mixer'].update(presence_db=2,high_db=1,compression=45,variation=50,rhythm=55,pause=100)
        file=OUTPUT/f'old-{sid}.wav'
        r=subprocess.run([str(previous),'--render-json',str(file)],input=json.dumps([p,TEXT],ensure_ascii=False),capture_output=True,encoding='utf-8',timeout=90)
        assert r.returncode==0,r.stderr
        records.append(dict(model='AISHELL3 VITS',voice=p['voice'],title=title,text=TEXT,file=file.name))
    for row in records:
        with wave.open(str(OUTPUT/row['file']),'rb') as w:
            row.update(sample_rate=w.getframerate(),seconds=round(w.getnframes()/w.getframerate(),3))
        assert row['sample_rate']==(24000 if row['model'].startswith('Kokoro') else 8000)
    (OUTPUT/'samples.json').write_text(json.dumps(records,ensure_ascii=False,indent=2),encoding='utf-8')
    write_page(records)


def write_page(records):
    cards=''.join('<article><h2>'+html.escape(r['title'])+'</h2><p>'+html.escape(r['text'])+'</p><audio controls preload="metadata" src="data:audio/wav;base64,'+base64.b64encode((OUTPUT/r['file']).read_bytes()).decode()+'"></audio><p class="meta">'+html.escape(r['model'])+' · '+str(r['sample_rate']//1000)+' kHz</p></article>' for r in records)
    page='''<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Agent Voice · 新旧声音试听</title>
<style>body{background:#050505;color:#f5f5f5;font:16px/1.7 "Microsoft YaHei UI",sans-serif;max-width:1050px;margin:auto;padding:36px 24px}h1{font-size:28px}header p{color:#aaa;max-width:850px}.grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:20px}article{border:1px solid #292929;background:#0c0c0c;padding:20px}h2{font-size:19px;margin:0 0 12px}audio{width:100%}.meta{color:#aaa;font-size:13px;margin-bottom:0}@media(max-width:640px){.grid{grid-template-columns:1fr}}</style>
<header><h1>新旧声音，直接听</h1><p>前八条使用新接入的 Kokoro，最后两条使用旧版 AISHELL3。完成通知采用同一句话，各声音使用原速原调、原始停顿与相同最终音量。点击播放比较音色、语气与机械感；这些样例没有经过真人自然度评分。</p></header><main class="grid">'''+cards+'''</main><script>document.addEventListener('play',event=>{for(const audio of document.querySelectorAll('audio'))if(audio!==event.target)audio.pause()},true)</script></html>'''
    (OUTPUT/'试听对比.html').write_text(page,encoding='utf-8')
    print(json.dumps(dict(samples=len(records),output=str(OUTPUT),sample_rates=sorted({r['sample_rate'] for r in records})),ensure_ascii=False),flush=True)


if __name__=='__main__':main()
