"""Run the pinned native C ABI against Kokoro before changing the app engine."""
import ctypes as C
import argparse
import json
import os
from pathlib import Path
import time
import wave
from array import array

ROOT = Path(__file__).resolve().parent.parent
P = C.c_char_p
I = C.c_int32
F = C.c_float


class Vits(C.Structure):
    _fields_ = [(n, P) for n in ['model','lexicon','tokens','data_dir']] + [(n,F) for n in ['noise_scale','noise_scale_w','length_scale']] + [('dict_dir',P)]
class Matcha(C.Structure):
    _fields_ = [(n,P) for n in ['acoustic_model','vocoder','lexicon','tokens','data_dir']] + [('noise_scale',F),('length_scale',F),('dict_dir',P)]
class Kokoro(C.Structure):
    _fields_ = [(n,P) for n in ['model','voices','tokens','data_dir']] + [('length_scale',F)] + [(n,P) for n in ['dict_dir','lexicon','lang']]
class Kitten(C.Structure):
    _fields_ = [(n,P) for n in ['model','voices','tokens','data_dir']] + [('length_scale',F)]
class Zipvoice(C.Structure):
    _fields_ = [(n,P) for n in ['tokens','encoder','decoder','vocoder','data_dir','lexicon']] + [(n,F) for n in ['feat_scale','t_shift','target_rms','guidance_scale']]
class Model(C.Structure):
    _fields_ = [('vits',Vits),('num_threads',I),('debug',I),('provider',P),('matcha',Matcha),('kokoro',Kokoro),('kitten',Kitten),('zipvoice',Zipvoice)]
class Config(C.Structure):
    _fields_ = [('model',Model),('rule_fsts',P),('max_num_sentences',I),('rule_fars',P),('silence_scale',F)]
class Audio(C.Structure):
    _fields_ = [('samples',C.POINTER(F)),('n',I),('sample_rate',I)]


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--threads',type=int,default=2)
    parser.add_argument('--quick',action='store_true')
    args=parser.parse_args()
    source = ROOT / '.build/kokoro-source/kokoro-int8-multi-lang-v1_1'
    runtime = ROOT / 'resources/local-tts'
    output = ROOT / (f'.build/qa/kokoro-probe-{args.threads}-thread' if args.quick else '.build/qa/kokoro-probe')
    output.mkdir(parents=True,exist_ok=True)
    with os.add_dll_directory(str(runtime)):
        lib = C.CDLL(str(runtime / 'sherpa-onnx-c-api.dll'))
        lib.SherpaOnnxCreateOfflineTts.argtypes = [C.POINTER(Config)]
        lib.SherpaOnnxCreateOfflineTts.restype = C.c_void_p
        lib.SherpaOnnxOfflineTtsGenerate.argtypes = [C.c_void_p,P,I,F]
        lib.SherpaOnnxOfflineTtsGenerate.restype = C.POINTER(Audio)
        lib.SherpaOnnxDestroyOfflineTtsGeneratedAudio.argtypes = [C.POINTER(Audio)]
        lib.SherpaOnnxDestroyOfflineTts.argtypes = [C.c_void_p]
        config = Config()
        assert C.sizeof(config)==328
        path = lambda name:str(source/name).encode('utf-8')
        config.model.kokoro = Kokoro(path('model.int8.onnx'),path('voices.bin'),path('tokens.txt'),path('espeak-ng-data'),1.,None,
                                      b','.join(path(n) for n in ['lexicon-us-en.txt','lexicon-zh.txt']),None)
        config.model.num_threads = args.threads
        config.model.provider = b'cpu'
        config.max_num_sentences = 1
        config.silence_scale = 1.
        config.rule_fsts = b','.join(path(n) for n in ['phone-zh.fst','date-zh.fst','number-zh.fst'])
        started=time.monotonic()
        engine=lib.SherpaOnnxCreateOfflineTts(C.byref(config))
        assert engine, 'Native Kokoro creation failed'
        print('Loaded in',round(time.monotonic()-started,3),'seconds',flush=True)
        records=[]
        try:
            for sid in ([3] if args.quick else [3,59,9,11,21,75]):
                texts=['完成啦。','请确认一下。','任务已经完成，请检查结果。'] if args.quick else ['好啦，任务完成了，来看看结果吧。','需要你确认一下，我们再继续。','我是 Codex，代码检查已经通过了。']
                for idx,text in enumerate(texts):
                    started=time.monotonic()
                    audio=lib.SherpaOnnxOfflineTtsGenerate(engine,text.encode('utf-8'),sid,1.)
                    assert audio and audio.contents.n>1000
                    try:
                        a=audio.contents
                        pcm=array('h',(round(max(-1.,min(1.,v))*32767) for v in a.samples[:a.n]))
                        file=output/f'{sid}-{idx}.wav'
                        with wave.open(str(file),'wb') as w:
                            w.setparams((1,2,a.sample_rate,0,'NONE','not compressed'));w.writeframes(pcm.tobytes())
                        records.append(dict(sid=sid,text=text,sample_rate=a.sample_rate,seconds=a.n/a.sample_rate,ms=round((time.monotonic()-started)*1000),file=str(file)))
                    finally:lib.SherpaOnnxDestroyOfflineTtsGeneratedAudio(audio)
                print(json.dumps(records[-1],ensure_ascii=False),flush=True)
        finally:lib.SherpaOnnxDestroyOfflineTts(engine)
        (output/'report.json').write_text(json.dumps(records,ensure_ascii=False,indent=2),encoding='utf-8')


if __name__=='__main__':main()
