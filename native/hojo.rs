//! Experimental Hojo-TTS-Light backend, isolated behind a Python ONNX worker.
//! The model and test runtime are optional and never replace Kokoro defaults.
use crate::{model::{Profile, Result}, voice_catalog::Voice};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
};

pub const VOICE_PREFIX: &str = "local:hojo-40m:";
const MODEL_FILES: [&str; 7] = [
    "Hojo-TTS-Light-40M-llm.onnx",
    "Hojo-TTS-Light-40M-fine_local.onnx",
    "Hojo-TTS-Light-40M-decoder.onnx",
    "Hojo-TTS-Light-40M-voice.npz",
    "config.json",
    "tokenizer.json",
    "tokenizer_config.json",
];

pub fn catalog() -> &'static [Voice] {
    static VOICES: OnceLock<Vec<Voice>> = OnceLock::new();
    VOICES.get_or_init(|| {
        [
            ("hojo_zh_f_01", "Hojo 女声 01", "female"),
            ("hojo_zh_f_02", "Hojo 女声 02", "female"),
            ("hojo_zh_m_01", "Hojo 男声 01", "male"),
            ("hojo_zh_m_02", "Hojo 男声 02", "male"),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (speaker, name, gender))| Voice {
            id: 1000 + index as i32,
            speaker: speaker.into(),
            name: name.into(),
            gender: gender.into(),
            age_group: String::new(),
            accent: String::new(),
            language: "zh".into(),
            featured: false,
        })
        .collect()
    })
}

fn resource_directory() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?;
    let direct = parent.join("resources/hojo");
    if direct.join("onnx_model.py").is_file() {
        return Some(direct);
    }
    for ancestor in parent.ancestors().take(8) {
        if ancestor.join("Cargo.toml").is_file()
            && ancestor.join("resources/hojo/onnx_model.py").is_file()
        {
            return Some(ancestor.join("resources/hojo"));
        }
    }
    None
}

fn python_executable(resources: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AGENT_VOICE_HOJO_PYTHON").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }
    let root = resources.parent()?.parent()?;
    [
        resources.join("runtime/python.exe"),
        root.join(".build/hojo-venv/Scripts/python.exe"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

pub fn available() -> bool {
    let Some(resources) = resource_directory() else {
        return false;
    };
    python_executable(&resources).is_some()
        && resources.join("hojo_worker.py").is_file()
        && MODEL_FILES.iter().all(|name| resources.join(name).is_file())
}

pub fn voices() -> Vec<(String, String)> {
    if !available() {
        return Vec::new();
    }
    catalog()
        .iter()
        .map(|voice| {
            (
                format!("{VOICE_PREFIX}{}", voice.speaker),
                voice.label(),
            )
        })
        .collect()
}

fn pcm16_wav(data: &[u8]) -> Result<(u32, Vec<f32>)> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("Hojo 返回的音频不是有效 WAV".into());
    }
    let mut cursor = 12usize;
    let mut format = None;
    let mut samples = None;
    while cursor.checked_add(8).is_some_and(|end| end <= data.len()) {
        let size = u32::from_le_bytes(data[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let start = cursor + 8;
        let end = start.checked_add(size).ok_or("Hojo WAV 数据长度无效")?;
        if end > data.len() {
            return Err("Hojo WAV 数据不完整".into());
        }
        match &data[cursor..cursor + 4] {
            b"fmt " if size >= 16 => {
                format = Some((
                    u16::from_le_bytes(data[start..start + 2].try_into().unwrap()),
                    u16::from_le_bytes(data[start + 2..start + 4].try_into().unwrap()),
                    u32::from_le_bytes(data[start + 4..start + 8].try_into().unwrap()),
                    u16::from_le_bytes(data[start + 14..start + 16].try_into().unwrap()),
                ));
            }
            b"data" => samples = Some(&data[start..end]),
            _ => {}
        }
        cursor = end.checked_add(size & 1).ok_or("Hojo WAV 数据长度无效")?;
    }
    let (encoding, channels, rate, bits) = format.ok_or("Hojo WAV 缺少格式信息")?;
    if encoding != 1 || channels != 1 || bits != 16 || !(8000..=96000).contains(&rate) {
        return Err("Hojo WAV 格式不受支持（需要单声道 PCM16）".into());
    }
    let bytes = samples.ok_or("Hojo WAV 缺少音频数据")?;
    if bytes.len() < 2 || bytes.len() % 2 != 0 {
        return Err("Hojo WAV 音频长度无效".into());
    }
    let samples = bytes
        .chunks_exact(2)
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32768.0)
        .collect();
    Ok((rate, samples))
}

fn write_pcm16_wav(rate: u32, samples: &[f32]) -> Result<Vec<u8>> {
    let pcm_len = samples.len().checked_mul(2).ok_or("合成音频过长")?;
    let riff_len = u32::try_from(pcm_len.checked_add(36).ok_or("合成音频过长")?)
        .map_err(|_| "合成音频过长")?;
    let data_len = u32::try_from(pcm_len).map_err(|_| "合成音频过长")?;
    let mut wav = Vec::with_capacity(pcm_len + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_len.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        wav.extend_from_slice(&value.to_le_bytes());
    }
    Ok(wav)
}

pub fn render(profile: &Profile, text: &str, out: &Path) -> Result<()> {
    if text.trim().is_empty() {
        return Err("请输入播报内容".into());
    }
    profile.mixer.validate()?;
    let speaker = profile
        .voice
        .strip_prefix(VOICE_PREFIX)
        .filter(|id| catalog().iter().any(|voice| voice.speaker == *id))
        .ok_or("Hojo 试用音色标识无效")?;
    let resources = resource_directory().ok_or("找不到 Hojo 试用模型，请保留 EXE 旁的 resources 文件夹")?;
    let python = python_executable(&resources)
        .ok_or("Hojo 试用运行环境缺失：请在项目 .build/hojo-venv 安装本地 ONNX 依赖")?;
    let worker = resources.join("hojo_worker.py");
    let raw_path = out.with_file_name(format!("hojo-{}.wav", crate::model::token()));
    let request = serde_json::json!({
        "text": text,
        "voice": speaker,
        "output": raw_path,
    });
    let result = (|| {
        let mut child = Command::new(python)
            .arg("-X")
            .arg("utf8")
            .arg(worker)
            .arg("--models-dir")
            .arg(&resources)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| format!("启动 Hojo 本地推理失败：{e}"))?;
        child
            .stdin
            .take()
            .ok_or("Hojo 推理输入不可用")?
            .write_all(request.to_string().as_bytes())
            .map_err(|e| format!("发送 Hojo 合成请求失败：{e}"))?;
        let output = child
            .wait_with_output()
            .map_err(|e| format!("等待 Hojo 合成失败：{e}"))?;
        if !output.status.success() {
            let details = String::from_utf8_lossy(&output.stderr);
            let details = details.chars().rev().take(3000).collect::<String>().chars().rev().collect::<String>();
            return Err(if details.trim().is_empty() {
                format!("Hojo 本地合成失败（退出码 {}）", output.status)
            } else {
                format!("Hojo 本地合成失败：{}", details.trim())
            });
        }
        let raw = fs::read(&raw_path).map_err(|e| format!("读取 Hojo 合成结果失败：{e}"))?;
        let (rate, mut samples) = pcm16_wav(&raw)?;
        crate::mixer::scale_pauses(&mut samples, rate as i32, profile.mixer.pause);
        crate::mixer::process(&mut samples, rate as i32, &profile.mixer, profile.volume);
        let wav = write_pcm16_wav(rate, &samples)?;
        fs::write(out, wav).map_err(|e| format!("保存 Hojo 合成结果失败：{e}"))
    })();
    let _ = fs::remove_file(raw_path);
    result
}

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hojo_voices_are_four_chinese_presets_and_not_defaults() {
        assert_eq!(catalog().len(), 4);
        assert_eq!(catalog().iter().filter(|v| v.gender == "female").count(), 2);
        assert_eq!(catalog().iter().filter(|v| v.gender == "male").count(), 2);
        assert!(crate::model::Profile::default().voice.starts_with("local:kokoro"));
    }

    #[test]
    fn parses_pcm16_wav_and_rejects_other_formats() {
        let source = write_pcm16_wav(24000, &[0.5, -0.5]).unwrap();
        let (rate, samples) = pcm16_wav(&source).unwrap();
        assert_eq!(rate, 24000);
        assert_eq!(samples.len(), 2);
        assert!(samples[0] > 0.49 && samples[0] < 0.51);
        assert!(samples[1] < -0.49 && samples[1] > -0.51);
        assert!(pcm16_wav(b"not a wav").is_err());
    }
}
