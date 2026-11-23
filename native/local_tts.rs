//! A single pinned Kokoro v1.1-zh INT8 model through sherpa-onnx's C API.
//! The engine is serialized, uses two CPU threads and releases after 30s idle.
use crate::model::{Profile, Result};
use std::{
    ffi::{c_char, c_void, CStr, CString},
    fs,
    path::{Path, PathBuf},
    ptr::null,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use windows_sys::Win32::{Foundation::*, System::LibraryLoader::*};

pub const DEFAULT_VOICE: &str = "local:kokoro-v1.1:zf_001";
pub const SPEAKERS: i32 = 103;
pub const MODEL_NAME: &str = "Kokoro v1.1-zh INT8";
pub const IDLE_SECONDS: u64 = 30;

pub fn directory() -> Result<PathBuf> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let parent = exe.parent().ok_or("程序路径无效")?;
    let direct = parent.join("resources/kokoro");
    if direct.join("model.int8.onnx").is_file() {
        return Ok(direct);
    }
    // Cargo output is inside the source tree. A portable EXE only uses its
    // adjacent resources, never an arbitrary working directory or search path.
    for ancestor in parent.ancestors().take(6) {
        if ancestor.join("Cargo.toml").is_file() && ancestor.join("native/local_tts.rs").is_file() {
            let source = ancestor.join("resources/kokoro");
            if source.join("model.int8.onnx").is_file() {
                return Ok(source);
            }
        }
    }
    Err("本地声音资源缺失，请完整解压发布包，保留 EXE 旁的 resources 文件夹".into())
}

pub fn voices() -> Vec<(String, String)> {
    if directory().is_err() {
        return vec![];
    }
    let featured = [3, 59, 9, 75, 21, 11];
    featured
        .into_iter()
        .chain((0..SPEAKERS).filter(|id| !featured.contains(id)))
        .map(|id| {
            (
                format!(
                    "local:kokoro-v1.1:{}",
                    crate::voice_catalog::catalog().voices[id as usize].speaker
                ),
                crate::voice_catalog::catalog().voices[id as usize].label(),
            )
        })
        .chain(crate::hojo::voices())
        .collect()
}

pub fn speaker(voice: &str) -> Result<i32> {
    crate::voice_catalog::get(voice)
        .map(|v| v.id)
        .ok_or_else(|| "本地音色无效，请选择 Kokoro 声音".into())
}

// ABI copied from the pinned sherpa-onnx v1.12.20 c-api.h. Runtime version is
// checked before passing this struct; changing DLL versions needs an ABI review.
#[repr(C)]
struct Vits {
    model: *const c_char,
    lexicon: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    noise_scale: f32,
    noise_scale_w: f32,
    length_scale: f32,
    dict_dir: *const c_char,
}
#[repr(C)]
struct Matcha {
    acoustic_model: *const c_char,
    vocoder: *const c_char,
    lexicon: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    noise_scale: f32,
    length_scale: f32,
    dict_dir: *const c_char,
}
#[repr(C)]
struct Kokoro {
    model: *const c_char,
    voices: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    length_scale: f32,
    dict_dir: *const c_char,
    lexicon: *const c_char,
    lang: *const c_char,
}
#[repr(C)]
struct Kitten {
    model: *const c_char,
    voices: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    length_scale: f32,
}
#[repr(C)]
struct Zipvoice {
    tokens: *const c_char,
    encoder: *const c_char,
    decoder: *const c_char,
    vocoder: *const c_char,
    data_dir: *const c_char,
    lexicon: *const c_char,
    feat_scale: f32,
    t_shift: f32,
    target_rms: f32,
    guidance_scale: f32,
}
#[repr(C)]
struct Model {
    vits: Vits,
    num_threads: i32,
    debug: i32,
    provider: *const c_char,
    matcha: Matcha,
    kokoro: Kokoro,
    kitten: Kitten,
    zipvoice: Zipvoice,
}
#[repr(C)]
struct Config {
    model: Model,
    rule_fsts: *const c_char,
    max_num_sentences: i32,
    rule_fars: *const c_char,
    silence_scale: f32,
}
#[repr(C)]
struct Audio {
    samples: *const f32,
    n: i32,
    sample_rate: i32,
}
type Create = unsafe extern "C" fn(*const Config) -> *const c_void;
type Destroy = unsafe extern "C" fn(*const c_void);
type Generate = unsafe extern "C" fn(*const c_void, *const c_char, i32, f32) -> *const Audio;
type FreeAudio = unsafe extern "C" fn(*const Audio);

struct Engine {
    module: HMODULE,
    tts: *const c_void,
    destroy: Destroy,
    generate: Generate,
    free_audio: FreeAudio,
    last_used: Instant,
}
// All access, including destruction and idle cleanup, holds ENGINE's mutex.
unsafe impl Send for Engine {}
impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)(self.tts);
            FreeLibrary(self.module);
        }
    }
}
static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

unsafe fn symbol<T: Copy>(module: HMODULE, name: &[u8]) -> Result<T> {
    let proc = GetProcAddress(module, name.as_ptr()).ok_or("本地声音运行库版本不兼容")?;
    Ok(std::mem::transmute_copy(&proc))
}

impl Engine {
    unsafe fn load() -> Result<Self> {
        let dir = directory()?;
        for name in [
            "model.int8.onnx",
            "voices.bin",
            "lexicon-zh.txt",
            "lexicon-us-en.txt",
            "tokens.txt",
            "number-zh.fst",
            "date-zh.fst",
            "phone-zh.fst",
            "espeak-ng-data/phondata",
            "dict/jieba.dict.utf8",
            "sherpa-onnx-c-api.dll",
            "onnxruntime.dll",
        ] {
            if !dir.join(name).is_file() {
                return Err(format!("本地声音缺少 {name}，请完整解压发布包"));
            }
        }
        let module = LoadLibraryExW(
            crate::speech::wide(&dir.join("sherpa-onnx-c-api.dll").to_string_lossy()).as_ptr(),
            null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        );
        if module.is_null() {
            return Err(format!(
                "本地声音运行库加载失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        let result = (|| {
            let version: unsafe extern "C" fn() -> *const c_char =
                symbol(module, b"SherpaOnnxGetVersionStr\0")?;
            let version = CStr::from_ptr(version()).to_string_lossy();
            if version.trim_start_matches('v') != "1.12.20" {
                return Err(format!("本地运行库版本不匹配：{version}"));
            }
            let create: Create = symbol(module, b"SherpaOnnxCreateOfflineTts\0")?;
            let destroy = symbol(module, b"SherpaOnnxDestroyOfflineTts\0")?;
            let generate = symbol(module, b"SherpaOnnxOfflineTtsGenerate\0")?;
            let free_audio = symbol(module, b"SherpaOnnxDestroyOfflineTtsGeneratedAudio\0")?;
            let path = |name: &str| {
                CString::new(dir.join(name).to_string_lossy().as_bytes()).map_err(|e| e.to_string())
            };
            let model = path("model.int8.onnx")?;
            let voices = path("voices.bin")?;
            let data_dir = path("espeak-ng-data")?;
            let dict_dir = path("dict")?;
            let lexicon = CString::new(
                ["lexicon-us-en.txt", "lexicon-zh.txt"]
                    .map(|n| dir.join(n).to_string_lossy().into_owned())
                    .join(","),
            )
            .map_err(|e| e.to_string())?;
            let tokens = path("tokens.txt")?;
            let rules = CString::new(
                ["phone-zh.fst", "date-zh.fst", "number-zh.fst"]
                    .map(|n| dir.join(n).to_string_lossy().into_owned())
                    .join(","),
            )
            .map_err(|e| e.to_string())?;
            let mut config: Config = std::mem::zeroed();
            config.model.kokoro = Kokoro {
                model: model.as_ptr(),
                voices: voices.as_ptr(),
                lexicon: lexicon.as_ptr(),
                tokens: tokens.as_ptr(),
                data_dir: data_dir.as_ptr(),
                length_scale: 1.0,
                dict_dir: dict_dir.as_ptr(),
                lang: null(),
            };
            config.model.num_threads = 2;
            config.model.provider = c"cpu".as_ptr();
            config.rule_fsts = rules.as_ptr();
            config.max_num_sentences = 1;
            // 1.12.20 ScaleSilence treats every sample below 0.01 as silence.
            // Entire quiet syllables can satisfy that absolute threshold.
            // Preserve model output and adjust real gaps ourselves, relative
            // to this speaker's level, before EQ and final volume.
            config.silence_scale = 1.0;
            let tts = create(&config);
            if tts.is_null() {
                return Err("本地声音模型加载失败，请检查发布包完整性".into());
            }
            Ok(Self {
                module,
                tts,
                destroy,
                generate,
                free_audio,
                last_used: Instant::now(),
            })
        })();
        if result.is_err() {
            FreeLibrary(module);
        }
        result
    }
}
use std::ptr::null_mut;

pub fn render(profile: &Profile, text: &str, out: &Path) -> Result<()> {
    if profile.voice.starts_with(crate::hojo::VOICE_PREFIX) {
        return crate::hojo::render(profile, text, out);
    }
    // Also support callers sending a legacy profile directly to --render-json,
    // which bypasses Store::load. Never load the obsolete model as a fallback.
    let mut compatible = profile.clone();
    crate::voice_catalog::upgrade_profile(&mut compatible);
    let profile = &compatible;
    let id = speaker(&profile.voice)?;
    profile.mixer.validate()?;
    // Kokoro has a bilingual front-end: preserve English words and punctuation.
    let text = CString::new(text).map_err(|_| "播报内容包含无效字符")?;
    static CLEANUP: OnceLock<()> = OnceLock::new();
    CLEANUP.get_or_init(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(5));
            if let Ok(mut engine) = ENGINE.try_lock() {
                if engine
                    .as_ref()
                    .is_some_and(|e| e.last_used.elapsed() >= Duration::from_secs(IDLE_SECONDS))
                {
                    *engine = None;
                }
            }
        });
    });
    let mut holder = ENGINE.lock().map_err(|_| "本地声音工作线程不可用")?;
    if holder.is_none() {
        *holder = Some(unsafe { Engine::load()? });
    }
    let engine = holder.as_mut().unwrap();
    let pitch = 2f32.powf(profile.pitch as f32 / 80.0);
    let speed = (1.0 + profile.rate as f32 / 100.0) / pitch;
    let audio = unsafe { (engine.generate)(engine.tts, text.as_ptr(), id, speed) };
    if audio.is_null() {
        return Err("本地模型未生成音频，请检查播报内容".into());
    }
    let result = (|| {
        let audio = unsafe { &*audio };
        if audio.n <= 0 || audio.samples.is_null() || !(8000..=96000).contains(&audio.sample_rate) {
            return Err("本地模型返回了无效音频".into());
        }
        let samples = unsafe { std::slice::from_raw_parts(audio.samples, audio.n as usize) };
        let count = (samples.len() as f32 / pitch) as usize;
        let mut processed = Vec::with_capacity(count);
        for i in 0..count {
            let pos = i as f32 * pitch;
            let left = pos as usize;
            let fraction = pos - left as f32;
            let value = samples[left.min(samples.len() - 1)] * (1.0 - fraction)
                + samples[(left + 1).min(samples.len() - 1)] * fraction;
            processed.push(value);
        }
        crate::mixer::scale_pauses(&mut processed, audio.sample_rate, profile.mixer.pause);
        crate::mixer::process(
            &mut processed,
            audio.sample_rate,
            &profile.mixer,
            profile.volume,
        );
        let pcm: Vec<u8> = processed
            .iter()
            .flat_map(|x| ((*x * 32767.0).round() as i16).to_le_bytes())
            .collect();
        let mut wav = Vec::with_capacity(44 + pcm.len());
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(pcm.len() as u32 + 36).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&(audio.sample_rate as u32).to_le_bytes());
        wav.extend_from_slice(&(audio.sample_rate as u32 * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        wav.extend_from_slice(&pcm);
        fs::write(out, wav).map_err(|e| e.to_string())
    })();
    unsafe {
        (engine.free_audio)(audio);
    }
    engine.last_used = Instant::now();
    result
}

pub fn release() {
    if let Ok(mut holder) = ENGINE.try_lock() {
        *holder = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_speaker_boundaries_and_abi() {
        assert_eq!(speaker(DEFAULT_VOICE), Ok(3));
        assert_eq!(speaker("local:kokoro-v1.1:zm_100"), Ok(102));
        assert!(speaker("local:kokoro-v1.1:zf_999").is_err());
        assert!(speaker("local:aishell3:10").is_err());
        assert_eq!(std::mem::size_of::<Vits>(), 56);
        assert_eq!(std::mem::size_of::<Config>(), 328);
    }
}
