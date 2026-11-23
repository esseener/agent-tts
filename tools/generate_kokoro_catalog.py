"""Stable voice IDs from sherpa's published Kokoro v1.1-zh speaker mapping."""
import hashlib
import json
from pathlib import Path

ROOT=Path(__file__).resolve().parent.parent
female=[1,2,3,4,5,6,7,8,17,18,19,21,22,23,24,26,27,28,32,36,38,39,40,42,43,44,46,47,48,49,51,59,60,67,70,71,72,73,74,75,76,77,78,79,83,84,85,86,87,88,90,92,93,94,99]
male=[9,10,11,12,13,14,15,16,20,25,29,30,31,33,34,35,37,41,45,50,52,53,54,55,56,57,58,61,62,63,64,65,66,68,69,80,81,82,89,91,95,96,97,98,100]
old=json.loads((ROOT/'assets/voice-catalog.json').read_text(encoding='utf-8'))
aliases={gender:[v['name'] for v in old['voices'] if v['gender']==gender] for gender in ['female','male']}
featured={3:'晓禾',59:'云舟',9:'晴音',75:'远川',21:'知夏',11:'若晴'}
used=set(featured.values())|{'Maple','Sol','Vale'}
voices=[]
for sid,speaker in enumerate(['af_maple','af_sol','bf_vale']+[f'zf_{n:03}' for n in female]+[f'zm_{n:03}' for n in male]):
    gender='male' if speaker.startswith('zm_') else 'female'
    if sid<3:name=['Maple','Sol','Vale'][sid]
    elif sid in featured:name=featured[sid]
    else:
        name=next((n for n in aliases[gender] if n not in used),f'男声{sid:03}')
    used.add(name)
    voices.append(dict(id=sid,speaker=speaker,name=name,gender=gender,language='en' if sid<3 else 'zh',featured=sid in featured))
assert len(voices)==103 and len({v['name'] for v in voices})==103
catalog=dict(source='https://k2-fsa.github.io/sherpa/onnx/tts/all/Chinese-English/kokoro-multi-lang-v1_1.html',
             metadata_sha256=hashlib.sha256('\n'.join(v['speaker'] for v in voices).encode()).hexdigest(),
             model='Kokoro v1.1-zh INT8',sample_rate=24000,
             name_note='中文名为应用昵称，不是真人身份；zf/zm 原始标识和性别来自模型映射。精选为应用入口，不是官方质量排名。',voices=voices)
(ROOT/'assets/kokoro-voice-catalog.json').write_text(json.dumps(catalog,ensure_ascii=False,indent=2),encoding='utf-8')
print('Kokoro catalog:',len(voices),'voices,',sum(v['language']=='zh' for v in voices),'Chinese')
