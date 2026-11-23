use crate::model::{Config, Playback, Profile, Result};
use std::os::windows::process::CommandExt;
use std::sync::mpsc::{self, Receiver, Sender};
use std::{
    fs,
    hash::{Hash, Hasher},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use windows::{
    core::PCWSTR,
    Win32::{Media::Speech::*, System::Com::*},
};

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
pub fn voices() -> Vec<(String, String)> {
    let mut choices = crate::local_tts::voices();
    choices.push(("sapi:default".into(), "Windows 默认声音 · 离线".into()));
    unsafe {
        if CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() {
            if let Ok(category) = CoCreateInstance::<_, ISpObjectTokenCategory>(
                &SpObjectTokenCategory,
                None,
                CLSCTX_ALL,
            ) {
                if category.SetId(SPCAT_VOICES, false).is_ok() {
                    if let Ok(tokens) = category.EnumTokens(PCWSTR::null(), PCWSTR::null()) {
                        let mut count = 0;
                        let _ = tokens.GetCount(&mut count);
                        for n in 0..count {
                            if let Ok(token) = tokens.Item(n) {
                                if let (Ok(id), Ok(name)) =
                                    (token.GetId(), token.GetStringValue(PCWSTR::null()))
                                {
                                    choices.push((
                                        format!("sapi:{}", id.to_string().unwrap_or_default()),
                                        format!("{} · 离线", name.to_string().unwrap_or_default()),
                                    ));
                                    CoTaskMemFree(Some(id.0.cast()));
                                    CoTaskMemFree(Some(name.0.cast()));
                                }
                            }
                        }
                    }
                }
            }
            CoUninitialize();
        }
    }
    for (id, name) in [
        ("zh-CN-XiaoxiaoNeural", "晓晓 · 温柔女声"),
        ("zh-CN-XiaoyiNeural", "晓伊 · 活泼女声"),
        ("zh-CN-YunxiNeural", "云希 · 清亮男声"),
        ("zh-CN-YunjianNeural", "云健 · 沉稳男声"),
        ("zh-CN-YunyangNeural", "云扬 · 播报男声"),
        ("zh-CN-YunxiaNeural", "云夏 · 少年音"),
        ("zh-CN-liaoning-XiaobeiNeural", "晓北 · 东北女声"),
        ("zh-CN-shaanxi-XiaoniNeural", "晓妮 · 陕西女声"),
        ("zh-HK-HiuMaanNeural", "晓曼 · 粤语女声"),
        ("zh-TW-HsiaoChenNeural", "晓臻 · 台湾女声"),
        ("en-US-AriaNeural", "Aria · 英文女声"),
        ("en-US-GuyNeural", "Guy · 英文男声"),
    ] {
        choices.push((format!("edge:{id}"), format!("{name} · 在线")));
    }
    choices
}
pub fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn render(profile: &Profile, text: &str, out: &Path) -> Result<()> {
    if text.trim().is_empty() {
        return Err("请输入播报内容".into());
    }
    if profile.voice.starts_with("local:") {
        return crate::local_tts::render(profile, text, out);
    }
    if let Some(voice) = profile.voice.strip_prefix("edge:") {
        if voice.is_empty()
            || voice.len() > 128
            || !voice
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("在线音色标识无效，请在声音库重新选择".into());
        }
        let config = msedge_tts::tts::SpeechConfig {
            voice_name: voice.into(),
            audio_format: "audio-24khz-48kbitrate-mono-mp3".into(),
            pitch: profile.pitch,
            rate: profile.rate,
            volume: profile.volume - 100,
        };
        let mut client = msedge_tts::tts::client::connect()
            .map_err(|e| format!("在线语音连接失败：{e}；也可选择 Windows 离线声音"))?;
        let audio = client
            .synthesize(&xml(text), &config)
            .map_err(|e| format!("在线语音合成失败：{e}"))?;
        if audio.audio_bytes.len() < 128 {
            return Err("服务未返回音频".into());
        }
        fs::write(out, audio.audio_bytes).map_err(|e| e.to_string())
    } else if let Some(id) = profile.voice.strip_prefix("sapi:") {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| e.to_string())?;
            let result = (|| -> windows::core::Result<()> {
                let voice: ISpVoice = CoCreateInstance(&SpVoice, None, CLSCTX_ALL)?;
                if id != "default" {
                    let token: ISpObjectToken = CoCreateInstance(&SpObjectToken, None, CLSCTX_ALL)?;
                    token.SetId(PCWSTR::null(), PCWSTR(wide(id).as_ptr()), false)?;
                    voice.SetVoice(&token)?;
                }
                voice.SetRate((profile.rate / 4).clamp(-10, 10))?;
                voice.SetVolume(profile.volume as u16)?;
                let stream: ISpStream = CoCreateInstance(&SpStream, None, CLSCTX_ALL)?;
                use windows::Win32::Media::Audio::WAVEFORMATEX;
                let format = WAVEFORMATEX {
                    wFormatTag: 1,
                    nChannels: 1,
                    nSamplesPerSec: 22050,
                    nAvgBytesPerSec: 44100,
                    nBlockAlign: 2,
                    wBitsPerSample: 16,
                    cbSize: 0,
                };
                stream.BindToFile(
                    PCWSTR(wide(&out.to_string_lossy()).as_ptr()),
                    SPFM_CREATE_ALWAYS,
                    Some(&windows::core::GUID::from_u128(
                        0xc31adbae_527f_4ff5_a230_f62bb61ff70c,
                    )),
                    Some(&format),
                    0,
                )?;
                voice.SetOutput(&stream, true)?;
                let ssml = format!(
                    "<pitch absmiddle=\"{}\">{}</pitch>",
                    (profile.pitch / 4).clamp(-10, 10),
                    xml(text)
                );
                voice.Speak(PCWSTR(wide(&ssml).as_ptr()), SPF_IS_XML.0 as u32, None)?;
                stream.Close()?;
                Ok(())
            })();
            CoUninitialize();
            result.map_err(|e| format!("Windows 语音失败：{e}。请在 Windows 语音设置安装完整的语音包，或改选在线音色。"))
        }
    } else {
        Err("未知音色，请在声音库重新选择".into())
    }
}
// Each MCI operation gets its own alias: read-only cache probes can overlap
// playback without closing or reconfiguring the playing device.
struct MciAudio {
    alias: String,
}

fn mci_command(command: &str, output: &mut [u16]) -> Result<()> {
    use windows_sys::Win32::Media::Multimedia::{mciGetErrorStringW, mciSendStringW};
    let code = unsafe {
        mciSendStringW(
            wide(command).as_ptr(),
            if output.is_empty() { std::ptr::null_mut() } else { output.as_mut_ptr() },
            output.len() as u32,
            std::ptr::null_mut(),
        )
    };
    if code == 0 {
        return Ok(());
    }
    let mut error = [0u16; 256];
    unsafe { mciGetErrorStringW(code, error.as_mut_ptr(), error.len() as u32); }
    Err(String::from_utf16_lossy(
        &error[..error.iter().position(|n| *n == 0).unwrap_or(error.len())],
    ))
}

impl MciAudio {
    fn open(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Err(format!("音频文件不存在：{}", path.display()));
        }
        let path = path.to_string_lossy();
        if path.contains(['"', '\n', '\r']) {
            return Err("音频路径无效".into());
        }
        // Legacy MCI drivers can reject non-BMP characters even through W APIs.
        let mut short = vec![0u16; 32768];
        let count = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetShortPathNameW(
                wide(&path).as_ptr(), short.as_mut_ptr(), short.len() as u32,
            )
        };
        let driver_path = if count > 0 && (count as usize) < short.len() {
            String::from_utf16_lossy(&short[..count as usize])
        } else {
            path.to_string()
        };
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ALIAS: AtomicU64 = AtomicU64::new(0);
        let audio = Self {
            alias: format!("agentvoiceaudio{}", NEXT_ALIAS.fetch_add(1, Ordering::Relaxed)),
        };
        mci_command(&format!("open \"{driver_path}\" alias {}", audio.alias), &mut [])?;
        Ok(audio)
    }
}

impl Drop for MciAudio {
    fn drop(&mut self) {
        let _ = mci_command(&format!("close {}", self.alias), &mut []);
    }
}

#[derive(Debug)]
pub(crate) struct WavData {
    pub range: std::ops::Range<usize>,
    pub format: u16,
    pub bits: u16,
}

// Cache writers emit PCM WAVs; also accept standard float/extensible PCM WAVs.
// Unknown ancillary chunks are legal, including their required odd-byte padding.
pub(crate) fn wav_data(bytes: &[u8]) -> Result<WavData> {
    let invalid = || "WAV 音频格式或长度无效".to_string();
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(invalid());
    }
    let end = (u32_at(4) as usize).checked_add(8).ok_or_else(invalid)?;
    if end != bytes.len() {
        return Err(invalid());
    }
    let mut cursor = 12usize;
    let mut format = None;
    let mut data = None;
    while cursor < end {
        let start = cursor.checked_add(8).filter(|n| *n <= end).ok_or_else(invalid)?;
        let size = u32_at(cursor + 4) as usize;
        let finish = start.checked_add(size).filter(|n| *n <= end).ok_or_else(invalid)?;
        match &bytes[cursor..cursor + 4] {
            b"fmt " => {
                if format.is_some() || size < 16 || size == 17 {
                    return Err(invalid());
                }
                if size >= 18 && 18 + u16_at(start + 16) as usize != size {
                    return Err(invalid());
                }
                let mut tag = u16_at(start);
                let channels = u16_at(start + 2) as u32;
                let rate = u32_at(start + 4);
                let byte_rate = u32_at(start + 8);
                let align = u16_at(start + 12) as u32;
                let bits = u16_at(start + 14);
                if tag == 0xfffe {
                    if size < 40 || u16_at(start + 16) < 22 {
                        return Err(invalid());
                    }
                    let valid_bits = u16_at(start + 18);
                    if valid_bits == 0 || valid_bits > bits
                        || bytes[start + 26..start + 40] != [0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]
                    {
                        return Err(invalid());
                    }
                    tag = u16_at(start + 24);
                }
                if channels == 0 || rate == 0
                    || !matches!((tag, bits), (1, 8 | 16 | 24 | 32) | (3, 32 | 64))
                    || align != channels * (bits as u32 / 8)
                    || rate.checked_mul(align) != Some(byte_rate)
                {
                    return Err(invalid());
                }
                format = Some((tag, bits, align as usize));
            }
            b"data" => {
                if data.is_some() || size == 0 {
                    return Err(invalid());
                }
                data = Some(start..finish);
            }
            _ => {}
        }
        cursor = finish.checked_add(size & 1).filter(|n| *n <= end).ok_or_else(invalid)?;
    }
    let (format, bits, align) = format.ok_or_else(invalid)?;
    let range = data.ok_or_else(invalid)?;
    if range.len() % align != 0 {
        return Err(invalid());
    }
    Ok(WavData { range, format, bits })
}

pub(crate) fn audio_identity(bytes: &[u8]) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hash);
    hash.finish()
}

// A successful probe never plays audio and never waits for a generation lock.
// MP3 remains the actual Edge output, checked by Windows' installed decoder.
pub(crate) fn validate_audio(path: &Path) -> Result<u64> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("mp3")) {
        if !bytes.starts_with(b"ID3")
            && !(bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0)
        {
            return Err("MP3 音频标识无效".into());
        }
        let audio = MciAudio::open(path)?;
        mci_command(&format!("set {} time format milliseconds", audio.alias), &mut [])?;
        let mut duration = [0u16; 64];
        mci_command(&format!("status {} length", audio.alias), &mut duration)?;
        let end = duration.iter().position(|n| *n == 0).unwrap_or(duration.len());
        if String::from_utf16_lossy(&duration[..end]).trim().parse::<u64>().unwrap_or(0) == 0 {
            return Err("MP3 音频无法解码或为空".into());
        }
    } else {
        wav_data(&bytes)?;
    }
    Ok(audio_identity(&bytes))
}

pub fn play_file(path: &Path, volume: i32) -> Result<()> {
    let audio = MciAudio::open(path)?;
    // Waveaudio drivers need not support setaudio; the actual open/play result
    // remains authoritative. Prepared local/preset PCM already contains volume.
    let _ = mci_command(&format!("setaudio {} volume to {}", audio.alias, volume.clamp(0, 100) * 10), &mut []);
    // Optional QA trace measures actual process-to-Windows playback submission.
    // It does not claim to measure the physical device's output latency.
    if let Some(trace) = std::env::var_os("AGENT_VOICE_PLAYBACK_TRACE") {
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(trace) {
            let _ = writeln!(
                file,
                "{}",
                serde_json::json!({"stage":"play_command","path":path,"pid":std::process::id(),
                "process_ms":crate::PROCESS_STARTED.get().map(|s|s.elapsed().as_millis()),
                "unix_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()*1000.0})
            );
        }
    }
    mci_command(&format!("play {} wait", audio.alias), &mut [])
}
pub fn cache_path(profile: &Profile, text: &str, home: &Path) -> PathBuf {
    let cache = home.join(".agent-voice/cache");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // Names, IDs and other event phrases cannot change this audio.
    (
        if profile.voice.starts_with(crate::hojo::VOICE_PREFIX) {
            "audio-hojo-40m-v2"
        } else {
            "audio-v5-kokoro-v1.1-zh-int8-native-bilingual-2026"
        },
        &profile.voice,
        profile.rate,
        profile.pitch,
        profile.volume,
        if profile.voice.starts_with("local:") {
            Some(&profile.mixer)
        } else {
            None
        },
        text,
    )
        .hash(&mut hasher);
    cache.join(format!(
        "{:x}.{}",
        hasher.finish(),
        if profile.voice.starts_with("edge:") {
            "mp3"
        } else {
            "wav"
        }
    ))
}
struct TemporaryAudio(PathBuf);

impl Drop for TemporaryAudio {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn render_bounded(profile: &Profile, text: &str, out: &Path) -> Result<()> {
    if profile.voice.starts_with("local:") {
        return render(profile, text, out);
    }
    // Isolate online/system I/O and bound connect/handshake or driver stalls.
    let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .creation_flags(0x08000000)
        .arg("--render-json")
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let result = (|| {
        child.stdin.take().ok_or("语音子进程输入不可用")?
            .write_all(serde_json::to_string(&(profile, text)).map_err(|e| e.to_string())?.as_bytes())
            .map_err(|e| e.to_string())?;
        let start = Instant::now();
        loop {
            if let Some(code) = child.try_wait().map_err(|e| e.to_string())? {
                return if code.success() { Ok(()) } else { Err("在线/系统声音合成子进程失败".into()) };
            }
            if start.elapsed() > Duration::from_secs(8) {
                return Err("在线/系统声音合成超过 8 秒，请选择本地中文音色以离线播报".into());
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    // Always reap the child before the temporary-file guard removes its output.
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    result.map_err(|error| {
        let detail = String::from_utf8_lossy(&output.stderr);
        if detail.trim().is_empty() { error } else { format!("{error}：{}", detail.trim()) }
    })
}

pub fn prepare(profile: &Profile, text: &str, home: &Path) -> Result<PathBuf> {
    prepare_with(profile, text, home, render_bounded)
}

fn prepare_with(
    profile: &Profile,
    text: &str,
    home: &Path,
    render_item: impl FnOnce(&Profile, &str, &Path) -> Result<()>,
) -> Result<PathBuf> {
    let out = cache_path(profile, text, home);
    fs::create_dir_all(out.parent().ok_or("缓存路径无效")?).map_err(|e| e.to_string())?;
    let _lock = crate::model::lock_file(&out, 120_000)?;
    if validate_audio(&out).is_ok() {
        return Ok(out);
    }
    let temporary = TemporaryAudio(out.with_file_name(format!(
        "{}.{}", crate::model::token(), out.extension().unwrap().to_string_lossy(),
    )));
    render_item(profile, text, &temporary.0)?;
    validate_audio(&temporary.0).map_err(|error| format!("合成音频无效：{error}"))?;
    // Do not replace a valid result published by an older/non-locking process.
    if validate_audio(&out).is_ok() {
        return Ok(out);
    }
    if let Err(error) = fs::rename(&temporary.0, &out) {
        if validate_audio(&out).is_err() {
            return Err(error.to_string());
        }
    }
    Ok(out)
}
pub fn execute(playback: &Playback, home: &Path) -> Result<String> {
    if !playback.audio.trim().is_empty() {
        let _lock = crate::ProcessLock::audio(home)?;
        play_file(&PathBuf::from(&playback.audio), playback.profile.volume)?;
        return Ok("音效已播放".into());
    }
    let out = prepare(&playback.profile, &playback.text, home)?;
    // Model preparation must not hold the cross-process playback lock.
    let _lock = crate::ProcessLock::audio(home)?;
    play_file(&out, 100)?;
    let meter = if playback.profile.voice.starts_with("local:") {
        audio_meter(&out).unwrap_or_default()
    } else {
        String::new()
    };
    Ok(format!("已播放：{}{}", playback.text, meter))
}
fn audio_meter(path: &Path) -> Option<String> {
    let wav = fs::read(path).ok()?;
    let data = wav_data(&wav).ok()?;
    if data.format != 1 || data.bits != 16 {
        return None;
    }
    let samples: Vec<f32> = wav[data.range]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect();
    let rms = crate::mixer::active_rms(&samples);
    let peak = samples.iter().fold(0.0f32, |p, x| p.max(x.abs()));
    if peak < 1e-7 {
        return Some(" · 静音".into());
    }
    Some(format!(
        " · 语音 {:.1} dBFS / 峰值 {:.1} dBFS",
        20.0 * rms.max(1e-9).log10(),
        20.0 * peak.log10()
    ))
}

pub const WARM_REPORT_PATH: &str = ".agent-voice/cache-preparation.json";

#[derive(Debug, Default, serde::Serialize)]
pub struct WarmReport {
    pub prepared: usize,
    pub failures: Vec<WarmFailure>,
}

#[derive(Debug, serde::Serialize)]
pub struct WarmFailure {
    pub profile_id: String,
    pub profile_name: String,
    pub text: String,
    pub error: String,
}

pub fn warm_cache(config: &Config, home: &Path) -> Result<WarmReport> {
    warm_cache_with(config, home, prepare)
}

fn warm_cache_with(
    config: &Config,
    home: &Path,
    mut prepare_item: impl FnMut(&Profile, &str, &Path) -> Result<PathBuf>,
) -> Result<WarmReport> {
    let mut report = WarmReport::default();
    let mut seen = std::collections::HashSet::new();
    for p in config
        .profiles
        .iter()
        .filter(|p| p.enabled && p.voice.starts_with("local:"))
    {
        for text in p.phrases.values().flatten().map(String::as_str).chain(
            config
                .hosts
                .iter()
                .filter(|h| h.enabled)
                .flat_map(|h| h.routes.values())
                .filter(|r| r.enabled && r.audio.is_empty() && r.profile == p.id)
                .flat_map(|r| r.text.lines().map(str::trim).filter(|line| !line.is_empty())),
        ) {
            if text.trim().is_empty() || !seen.insert(cache_path(p, text, home)) {
                continue;
            }
            match prepare_item(p, text, home) {
                Ok(_) => report.prepared += 1,
                Err(error) => report.failures.push(WarmFailure {
                    profile_id: p.id.clone(),
                    profile_name: p.name.clone(),
                    text: text.to_string(),
                    error,
                }),
            }
        }
    }
    let path = home.join(WARM_REPORT_PATH);
    let bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("无法编码台词准备报告（{}）：{error}", path.display()))?;
    crate::model::atomic_write(&path, &bytes)
        .map_err(|error| format!("无法写入台词准备报告（{}）：{error}", path.display()))?;
    Ok(report)
}
pub fn warm_background(config: Config, home: PathBuf) -> Receiver<Result<WarmReport>> {
    use std::sync::{Mutex, OnceLock};
    type Request = (Config, PathBuf, Sender<Result<WarmReport>>);
    type Pending = (bool, Option<Request>);
    static QUEUE: OnceLock<Mutex<Pending>> = OnceLock::new();
    let queue = QUEUE.get_or_init(|| Mutex::new((false, None)));
    let (tx, rx) = mpsc::channel();
    let Ok(mut state) = queue.lock() else {
        let _ = tx.send(Err("台词准备队列不可用".into()));
        return rx;
    };
    // Replacing pending work disconnects its receiver without reporting completion.
    state.1 = Some((config, home, tx));
    if state.0 {
        return rx;
    }
    state.0 = true;
    if let Err(error) = std::thread::Builder::new().spawn(move || loop {
        let (config, home, tx) = {
            let Ok(mut state) = queue.lock() else {
                break;
            };
            match state.1.take() {
                Some(next) => next,
                None => {
                    state.0 = false;
                    break;
                }
            }
        };
        // A panic must not strand the latest pending request behind a dead worker.
        let result = std::panic::catch_unwind(|| warm_cache(&config, &home))
            .unwrap_or_else(|_| Err("台词准备工作线程异常终止".into()));
        let _ = tx.send(result);
    }) {
        state.0 = false;
        if let Some((_, _, tx)) = state.1.take() {
            let _ = tx.send(Err(format!("无法启动台词准备工作线程：{error}")));
        }
    }
    rx
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn wav_fixture(samples: &[i16]) -> Vec<u8> {
        let data_len = (samples.len() * 2) as u32;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&22050u32.to_le_bytes());
        wav.extend_from_slice(&44100u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        wav
    }

    #[test]
    fn wav_validation_accepts_ancillary_chunks_and_locates_pcm() {
        let basic = wav_fixture(&[1000, -1000]);
        let mut wav = basic[..36].to_vec();
        wav.extend_from_slice(b"JUNK\x03\0\0\0abc\0");
        wav.extend_from_slice(&basic[36..]);
        wav.extend_from_slice(b"LIST\x04\0\0\0INFO");
        let size = (wav.len() - 8) as u32;
        wav[4..8].copy_from_slice(&size.to_le_bytes());
        let data = wav_data(&wav).unwrap();
        assert_eq!(data.format, 1);
        assert_eq!(data.bits, 16);
        assert_eq!(&wav[data.range], &basic[44..]);
    }

    #[test]
    fn wav_validation_rejects_truncation_forged_headers_and_bad_format() {
        let wav = wav_fixture(&[1000, -1000]);
        for length in 0..wav.len() {
            assert!(wav_data(&wav[..length]).is_err(), "accepted truncation at {length}");
        }
        let mut fake = vec![0u8; 128];
        fake[..4].copy_from_slice(b"RIFF");
        fake[36..40].copy_from_slice(b"data");
        assert!(wav_data(&fake).is_err());
        for (offset, bytes) in [
            (4, 35u32.to_le_bytes()),
            (16, 1000u32.to_le_bytes()),
            (28, 1u32.to_le_bytes()),
            (40, 1000u32.to_le_bytes()),
            (40, 3u32.to_le_bytes()),
        ] {
            let mut invalid = wav.clone();
            invalid[offset..offset + 4].copy_from_slice(&bytes);
            assert!(wav_data(&invalid).is_err(), "accepted invalid field at {offset}");
        }
        let mut no_format = wav.clone();
        no_format[12..16].copy_from_slice(b"JUNK");
        assert!(wav_data(&no_format).is_err());
        let mut wrong_align = wav;
        wrong_align[32..34].copy_from_slice(&1u16.to_le_bytes());
        assert!(wav_data(&wrong_align).is_err());
    }

    #[test]
    fn prepare_rebuilds_corrupt_cache_and_reuses_verified_output() {
        let home = std::env::temp_dir().join(crate::model::token());
        let profile = Profile::default();
        let out = cache_path(&profile, "repair", &home);
        fs::create_dir_all(out.parent().unwrap()).unwrap();
        fs::write(&out, vec![0u8; 128]).unwrap();
        let expected = wav_fixture(&[1000, -1000]);
        let prepared = prepare_with(&profile, "repair", &home, |_, _, temp| {
            fs::write(temp, &expected).map_err(|e| e.to_string())
        }).unwrap();
        assert_eq!(prepared, out);
        assert_eq!(fs::read(&out).unwrap(), expected);
        prepare_with(&profile, "repair", &home, |_, _, _| panic!("valid cache must not render")).unwrap();
        assert_eq!(fs::read_dir(out.parent().unwrap()).unwrap().count(), 1);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn prepare_rejects_invalid_render_and_cleans_all_temporary_outputs() {
        let home = std::env::temp_dir().join(crate::model::token());
        let profile = Profile::default();
        let out = cache_path(&profile, "repair", &home);
        fs::create_dir_all(out.parent().unwrap()).unwrap();
        let broken = vec![0u8; 128];
        fs::write(&out, &broken).unwrap();
        assert!(prepare_with(&profile, "repair", &home, |_, _, temp| {
            fs::write(temp, &broken).map_err(|e| e.to_string())
        }).unwrap_err().contains("合成音频无效"));
        assert_eq!(fs::read(&out).unwrap(), broken);
        assert_eq!(fs::read_dir(out.parent().unwrap()).unwrap().count(), 1);
        assert!(prepare_with(&profile, "repair", &home, |_, _, temp| {
            fs::write(temp, wav_fixture(&[1, -1])).unwrap();
            Err("injected render error".into())
        }).unwrap_err().contains("injected render error"));
        assert_eq!(fs::read(&out).unwrap(), broken);
        assert_eq!(fs::read_dir(out.parent().unwrap()).unwrap().count(), 1);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn prepare_preserves_valid_cache_published_during_render() {
        let home = std::env::temp_dir().join(crate::model::token());
        let profile = Profile::default();
        let out = cache_path(&profile, "repair", &home);
        let newer = wav_fixture(&[2000, -2000]);
        prepare_with(&profile, "repair", &home, |_, _, temp| {
            fs::write(temp, wav_fixture(&[1000, -1000])).unwrap();
            fs::write(&out, &newer).map_err(|e| e.to_string())
        }).unwrap();
        assert_eq!(fs::read(&out).unwrap(), newer);
        assert_eq!(fs::read_dir(out.parent().unwrap()).unwrap().count(), 1);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn concurrent_prepare_waits_for_verified_publication_without_rerendering() {
        let home = std::env::temp_dir().join(crate::model::token());
        let profile = Profile::default();
        let (rendering_tx, rendering_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first = {
            let home = home.clone();
            let profile = profile.clone();
            std::thread::spawn(move || prepare_with(&profile, "concurrent", &home, |_, _, temp| {
                rendering_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
                fs::write(temp, wav_fixture(&[1000, -1000])).map_err(|e| e.to_string())
            }))
        };
        rendering_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let second = {
            let home = home.clone();
            let profile = profile.clone();
            std::thread::spawn(move || {
                started_tx.send(()).unwrap();
                prepare_with(&profile, "concurrent", &home, |_, _, _| panic!("second render must reuse publication"))
            })
        };
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        release_tx.send(()).unwrap();
        let first_out = first.join().unwrap().unwrap();
        assert_eq!(second.join().unwrap().unwrap(), first_out);
        assert!(validate_audio(&first_out).is_ok());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn edge_cache_keeps_mp3_extension_and_rejects_disguised_wav_output() {
        let home = std::env::temp_dir().join(crate::model::token());
        let profile = Profile { voice: "edge:zh-CN-XiaoxiaoNeural".into(), ..Profile::default() };
        let out = cache_path(&profile, "edge", &home);
        assert_eq!(out.extension().unwrap(), "mp3");
        let error = prepare_with(&profile, "edge", &home, |_, _, temp| {
            assert_eq!(temp.extension().unwrap(), "mp3");
            fs::write(temp, wav_fixture(&[1000, -1000])).map_err(|e| e.to_string())
        }).unwrap_err();
        assert!(error.contains("MP3"));
        assert!(!out.exists());
        assert_eq!(fs::read_dir(out.parent().unwrap()).unwrap().count(), 0);
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn spoken_text_cannot_inject_ssml() {
        assert_eq!(xml("<a>&\"'"), "&lt;a&gt;&amp;&quot;&apos;");
    }
    #[test]
    fn cache_depends_only_on_audio_inputs() {
        let mut p = Profile::default();
        let key = cache_path(&p, "完成了", Path::new("test"));
        p.name = "改名".into();
        p.id = "different".into();
        p.phrases.insert("error".into(), vec!["新错误台词".into()]);
        assert_eq!(key, cache_path(&p, "完成了", Path::new("test")));
        p.rate = 10;
        assert_ne!(key, cache_path(&p, "完成了", Path::new("test")));
        p.rate = 0;
        p.mixer.gain_db = 6;
        assert_ne!(key, cache_path(&p, "完成了", Path::new("test")));
    }

    #[test]
    fn warm_cache_continues_after_first_failure_and_persists_reason() {
        use crate::model::{Host, Route};

        let home = std::env::temp_dir().join(crate::model::token());
        let config = Config {
            profiles: vec![
                Profile {
                    phrases: [
                        ("done".into(), vec!["broken".into(), "broken".into(), "same event".into()]),
                        ("error".into(), vec!["later event".into()]),
                    ]
                    .into(),
                    ..Profile::default()
                },
                Profile {
                    id: "second".into(),
                    phrases: [("ask".into(), vec!["other profile".into()])].into(),
                    ..Profile::default()
                },
            ],
            hosts: vec![Host {
                routes: [
                    ("done".into(), Route { text: "first host text".into(), ..Route::default() }),
                    ("ask".into(), Route {
                        text: "second host text".into(),
                        profile: "second".into(),
                        ..Route::default()
                    }),
                ]
                .into(),
                ..Host::default()
            }],
            ..Config::default()
        };
        let mut calls = Vec::new();
        let report = warm_cache_with(&config, &home, |p, text, root| {
            calls.push(format!("{}:{text}", p.id));
            if text == "broken" {
                Err("injected synthesis error".into())
            } else {
                Ok(cache_path(p, text, root))
            }
        })
        .unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join(WARM_REPORT_PATH)).unwrap()).unwrap();
        fs::remove_dir_all(&home).unwrap();

        assert_eq!(calls, vec![
            "default:broken",
            "default:same event",
            "default:later event",
            "default:first host text",
            "second:other profile",
            "second:second host text",
        ]);
        assert_eq!(report.prepared, 5);
        assert_eq!(report.failures.len(), 1);
        let failure = &report.failures[0];
        assert_eq!(failure.profile_id, "default");
        assert_eq!(failure.profile_name, config.profiles[0].name);
        assert_eq!(failure.text, "broken");
        assert_eq!(failure.error, "injected synthesis error");
        assert_eq!(json, serde_json::json!({
            "prepared": 5,
            "failures": [{
                "profile_id": "default",
                "profile_name": config.profiles[0].name,
                "text": "broken",
                "error": "injected synthesis error",
            }],
        }));
    }

    #[test]
    fn warm_cache_deduplicates_cache_keys_and_filters_ineligible_text() {
        use crate::model::{Host, Route};

        let home = std::env::temp_dir().join(crate::model::token());
        let profile = Profile {
            phrases: [("done".into(), vec!["shared".into(), "shared".into(), " \t".into()])].into(),
            ..Profile::default()
        };
        let config = Config {
            profiles: vec![
                profile.clone(),
                Profile { id: "same-key".into(), ..profile.clone() },
                Profile { id: "different-key".into(), rate: profile.rate + 1, ..profile },
                Profile { enabled: false, id: "disabled".into(), ..Profile::default() },
                Profile { voice: "sapi:default".into(), id: "system".into(), ..Profile::default() },
            ],
            hosts: vec![
                Host {
                    routes: [
                        ("done".into(), Route { text: "shared".into(), ..Route::default() }),
                        ("ask".into(), Route {
                            enabled: false,
                            text: "muted route".into(),
                            ..Route::default()
                        }),
                        ("error".into(), Route {
                            audio: "custom.wav".into(),
                            text: "audio route".into(),
                            ..Route::default()
                        }),
                        ("subagent".into(), Route {
                            profile: "missing".into(),
                            text: "unbound route".into(),
                            ..Route::default()
                        }),
                    ]
                    .into(),
                    ..Host::default()
                },
                Host {
                    enabled: false,
                    routes: [("done".into(), Route {
                        text: "muted host".into(),
                        ..Route::default()
                    })]
                    .into(),
                    ..Host::default()
                },
            ],
            ..Config::default()
        };
        let mut calls = Vec::new();
        let report = warm_cache_with(&config, &home, |p, text, root| {
            calls.push(format!("{}:{text}", p.id));
            Ok(cache_path(p, text, root))
        })
        .unwrap();
        fs::remove_dir_all(&home).unwrap();

        assert_eq!(calls, vec!["default:shared", "different-key:shared"]);
        assert_eq!(report.prepared, 2);
        assert!(report.failures.is_empty());
    }

    #[test]
    fn warm_cache_reports_persistence_failure() {
        let home = std::env::temp_dir().join(crate::model::token());
        fs::write(&home, b"not a directory").unwrap();
        let config = Config { profiles: vec![], hosts: vec![], ..Config::default() };
        let error = warm_cache_with(&config, &home, |_, _, _| unreachable!()).unwrap_err();
        fs::remove_file(&home).unwrap();

        assert!(error.contains("无法写入台词准备报告"));
        assert!(error.contains(&home.join(WARM_REPORT_PATH).display().to_string()));
    }
}
