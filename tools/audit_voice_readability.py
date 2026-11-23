"""Optional offline QA; Vosk and its model live under .build/qa only."""
import argparse
import audioop
import copy
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import wave

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / '.build/qa/asr-python'))


def distance(a, b):
    row = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        next_row = [i]
        for j, y in enumerate(b, 1):
            next_row.append(min(row[j] + 1, next_row[-1] + 1, row[j-1] + (x != y)))
        row = next_row
    return row[-1]


def main():
    from vosk import Model, KaldiRecognizer, SetLogLevel
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, required=True)
    parser.add_argument('--label', required=True)
    parser.add_argument('--repeat', type=int, default=1)
    parser.add_argument('--selections', action='store_true', help='Audit the exact shipped fixed recipes')
    args = parser.parse_args()
    exe = args.exe.resolve()
    folder = ROOT / '.build/qa/voice-readability' / args.label
    folder.mkdir(parents=True, exist_ok=True)
    profile = json.loads(subprocess.run([str(exe), '--export-config'], capture_output=True,
                         encoding='utf-8', check=True).stdout)['profiles'][0]
    profile.update(rate=0, pitch=0, volume=100)
    profile['mixer'] = dict(normalize=True, gain_db=0, low_db=0, presence_db=2, high_db=1,
                            compression=45, variation=50, rhythm=55, pause=15)
    recipes = json.loads(subprocess.run([str(exe), '--voice-selections'], capture_output=True,
                         encoding='utf-8', check=True).stdout) if args.selections else None
    speakers = [int(r['voice'].rsplit(':',1)[1]) for r in recipes] if recipes else [10,21,33,41,45,66,99,103]
    phrases = [
        ('任务已经完成，请检查结果。', '任务已经完成请检查结果'),
        ('发生错误，请检查日志后重试。', '发生错误请检查日志后重试'),
        ('需要你的确认，请选择下一步。', '需要你的确认请选择下一步'),
        ('子任务已完成，结果已经准备好了。', '子任务已完成结果已经准备好了'),
        ('我是 codex，任务完成了。', '我是扣德克斯任务完成了'),
        ('open code 执行结束，结果已就绪。', '欧盆扣德执行结束结果已就绪'),
    ]
    batch, records = [], []
    for sid in speakers:
        for index, (text, expected) in enumerate(phrases):
            for repeat in range(args.repeat):
                p = copy.deepcopy(profile)
                p['voice'] = f'local:aishell3:{sid}'
                if recipes:
                    recipe = next(r for r in recipes if r['voice'] == p['voice'])
                    p.update(rate=recipe['rate'], pitch=recipe['pitch'], mixer=recipe['mixer'])
                out = folder / f'{sid}-{index}-{repeat}.wav'
                batch.append((p, text, str(out)))
                records.append(dict(sid=sid, text=text, expected=expected, path=str(out)))
    result = subprocess.run([str(exe), '--render-batch-json'], input=json.dumps(batch, ensure_ascii=False),
                            capture_output=True, encoding='utf-8', check=True, timeout=180)
    print(f'{args.label}: generated {len(records)} audio samples', flush=True)
    SetLogLevel(-1)
    model = Model(str(ROOT / '.build/qa/vosk-model-small-cn-0.22'))
    for index, record in enumerate(records):
        with wave.open(record['path'], 'rb') as wav:
            rate = wav.getframerate()
            data = wav.readframes(wav.getnframes())
            record['seconds'] = len(data) / (2 * rate)
        data, _ = audioop.ratecv(data, 2, 1, rate, 16000, None)
        recognizer = KaldiRecognizer(model, 16000)
        parts = []
        for start in range(0, len(data), 8000):
            if recognizer.AcceptWaveform(data[start:start+8000]):
                parts.append(json.loads(recognizer.Result())['text'])
        parts.append(json.loads(recognizer.FinalResult())['text'])
        recognized = re.sub(r'\s+', '', ''.join(parts))
        record['recognized'] = recognized
        record['character_errors'] = distance(record['expected'], recognized)
        record['character_error_rate'] = record['character_errors'] / len(record['expected'])
        if (index + 1) % 12 == 0:
            print(f'{args.label}: recognized {index+1}/{len(records)}', flush=True)
    rankings = []
    for sid in speakers:
        rows = [r for r in records if r['sid'] == sid]
        chinese = [r for r in rows if not re.search('[a-zA-Z]', r['text'])]
        rankings.append(dict(sid=sid,
            chinese_cer=sum(r['character_errors'] for r in chinese) / sum(len(r['expected']) for r in chinese),
            all_cer=sum(r['character_errors'] for r in rows) / sum(len(r['expected']) for r in rows),
            min_seconds=min(r['seconds'] for r in rows), max_seconds=max(r['seconds'] for r in rows)))
    rankings.sort(key=lambda r: r['chinese_cer'])
    report = dict(label=args.label, exe_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),
        method='Vosk small-cn-0.22 unconstrained recognition; objective aid, not a listening quality score.',
        qa_model_included_in_app=False, samples=len(records), rankings=rankings, records=records)
    output = ROOT / 'docs' / f'voice-readability-{args.label}.json'
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({k:v for k,v in report.items() if k != 'records'}, ensure_ascii=False, indent=2), flush=True)


if __name__ == '__main__':
    main()
