//! Small native voice channel: EQ, active-speech leveling, compressor and limiter.
use crate::model::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(default)]
pub struct Mixer {
    pub normalize: bool,
    pub gain_db: i32,
    pub low_db: i32,
    pub presence_db: i32,
    pub high_db: i32,
    pub compression: i32,
    pub variation: i32,
    pub rhythm: i32,
    pub pause: i32,
}
impl Default for Mixer {
    fn default() -> Self {
        Self {
            normalize: true,
            gain_db: 0,
            low_db: 0,
            presence_db: 0,
            high_db: 0,
            compression: 15,
            variation: 67,
            rhythm: 80,
            pause: 100,
        }
    }
}
impl Mixer {
    pub fn validate(&self) -> Result<()> {
        if !(-12..=18).contains(&self.gain_db)
            || [self.low_db, self.presence_db, self.high_db]
                .iter()
                .any(|n| !(-12..=12).contains(n))
            || [self.compression, self.variation, self.rhythm]
                .iter()
                .any(|n| !(0..=100).contains(n))
            || !(0..=200).contains(&self.pause)
        {
            return Err(
                "调音参数超出范围：增益 -12～18 dB，均衡 -12～12 dB，压缩/变化 0～100，停顿 0～200"
                    .into(),
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Preset {
    pub name: String,
    pub description: String,
    pub source: String,
    pub rate: i32,
    pub pitch: i32,
    pub mixer: Mixer,
}
impl Preset {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty()
            || self.name.chars().count() > 60
            || self.description.len() > 2048
            || self.source.len() > 1024
            || !(-40..=40).contains(&self.rate)
            || !(-40..=40).contains(&self.pitch)
        {
            return Err("调音预设名称或参数无效".into());
        }
        self.mixer.validate()
    }
}
pub fn presets() -> Vec<Preset> {
    let official = "Kokoro v1.1-zh 原速原调；后期处理关闭";
    let app = "Agent Voice 后期调音；基于同一个 Kokoro 模型，可继续修改";
    vec![
        Preset {
            name: "自然原调（推荐）".into(),
            description: "原速原调、原始停顿，轻度响度补偿；保留声音自身语调。".into(),
            source: app.into(),
            rate: 0,
            pitch: 0,
            mixer: Mixer::default(),
        },
        Preset {
            name: "温和陪伴".into(),
            description: "稍慢、柔和高频，保留自然起伏；适合长时间使用。".into(),
            source: app.into(),
            rate: -8,
            pitch: 0,
            mixer: Mixer {
                low_db: 2,
                high_db: -2,
                compression: 25,
                variation: 60,
                rhythm: 65,
                pause: 100,
                ..Mixer::default()
            },
        },
        Preset {
            name: "明亮轻快".into(),
            description: "稍快、轻微升调和明亮均衡；不是模型情绪标签。".into(),
            source: app.into(),
            rate: 10,
            pitch: 6,
            mixer: Mixer {
                presence_db: 2,
                high_db: 3,
                compression: 40,
                variation: 72,
                rhythm: 70,
                ..Mixer::default()
            },
        },
        Preset {
            name: "低沉稳重".into(),
            description: "轻微降调、低频加厚，保持人声清晰。".into(),
            source: app.into(),
            rate: -5,
            pitch: -10,
            mixer: Mixer {
                low_db: 3,
                presence_db: 1,
                high_db: -1,
                compression: 40,
                variation: 45,
                rhythm: 50,
                pause: 100,
                ..Mixer::default()
            },
        },
        Preset {
            name: "强力提醒".into(),
            description: "更紧实的动态和 +4 dB 增益；限幅始终保护峰值。".into(),
            source: app.into(),
            rate: 5,
            pitch: 0,
            mixer: Mixer {
                gain_db: 4,
                presence_db: 3,
                compression: 75,
                variation: 40,
                rhythm: 45,
                ..Mixer::default()
            },
        },
        Preset {
            name: "俏皮助手".into(),
            description: "升调、加快语速；属于后期效果，先试听再使用。".into(),
            source: app.into(),
            rate: 14,
            pitch: 12,
            mixer: Mixer {
                low_db: -2,
                high_db: 2,
                compression: 40,
                variation: 85,
                rhythm: 90,
                pause: 100,
                ..Mixer::default()
            },
        },
        Preset {
            name: "原声对照（官方参数）".into(),
            description: "原始音量、无均衡、无压缩；可能比处理后的声音轻。".into(),
            source: official.into(),
            rate: 0,
            pitch: 0,
            mixer: Mixer {
                normalize: false,
                compression: 0,
                pause: 100,
                ..Mixer::default()
            },
        },
    ]
}
pub fn read_library(data: &[u8]) -> Result<Vec<Preset>> {
    if data.len() > 262144 {
        return Err("预设文件不能超过 256 KB".into());
    }
    let document: serde_json::Value =
        serde_json::from_slice(data).map_err(|e| format!("调音文件无效：{e}"))?;
    if document["format"] != "agent-voice-tunings-v1" {
        return Err("请选择 Agent Voice 调音预设 JSON 文件".into());
    }
    let presets: Vec<Preset> = serde_json::from_value(document["presets"].clone())
        .map_err(|e| format!("调音预设无效：{e}"))?;
    if presets.is_empty() || presets.len() > 128 {
        return Err("预设文件应含 1～128 个调音预设".into());
    }
    for preset in &presets {
        preset.validate()?;
    }
    Ok(presets)
}

pub fn active_rms(samples: &[f32]) -> f32 {
    let peak = samples.iter().fold(0.0f32, |p, x| p.max(x.abs()));
    if peak < 1e-7 {
        return 0.0;
    }
    let threshold = (peak * 0.035).max(0.00003);
    let (sum, count) = samples
        .iter()
        .filter(|x| x.abs() > threshold)
        .fold((0.0f64, 0usize), |(sum, n), &x| {
            (sum + x as f64 * x as f64, n + 1)
        });
    if count == 0 {
        0.0
    } else {
        (sum / count as f64).sqrt() as f32
    }
}

/// Scale only sustained gaps far below this utterance's speech level.
/// Keep 80 ms of original audio on each side; never cut low-level speech
/// merely because a speaker is quiet. Expanding a gap inserts zero samples,
/// rather than reading past the interval as the pinned upstream code does.
pub fn scale_pauses(samples: &mut Vec<f32>, rate: i32, percent: i32) {
    if percent == 100 || samples.is_empty() {
        return;
    }
    let peak = samples.iter().fold(0.0f32, |p, x| p.max(x.abs()));
    if peak < 1e-7 {
        return;
    }
    let threshold = (peak * 0.002).min(active_rms(samples) * 0.02).max(1e-8);
    let frame = (rate as usize / 100).max(1);
    let min_gap = rate as usize * 3 / 10;
    let guard = rate as usize * 8 / 100;
    let mut spans = Vec::new();
    let mut start = None;
    for (index, chunk) in samples.chunks(frame).enumerate() {
        if chunk.iter().all(|s| s.abs() <= threshold) {
            start.get_or_insert(index * frame);
        } else if let Some(begin) = start.take() {
            if index * frame - begin >= min_gap {
                spans.push((begin, index * frame));
            }
        }
    }
    if let Some(begin) = start {
        if samples.len() - begin >= min_gap {
            spans.push((begin, samples.len()));
        }
    }
    if spans.is_empty() {
        return;
    }
    let mut adjusted = Vec::with_capacity(samples.len());
    let mut cursor = 0;
    for (begin, end) in spans {
        let gap = end - begin;
        let target = ((gap * percent.clamp(0, 200) as usize) / 100).max(guard * 2);
        adjusted.extend_from_slice(&samples[cursor..begin + guard]);
        adjusted.resize(adjusted.len() + target - guard * 2, 0.0);
        adjusted.extend_from_slice(&samples[end - guard..end]);
        cursor = end;
    }
    adjusted.extend_from_slice(&samples[cursor..]);
    *samples = adjusted;
}
fn db_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}
impl Biquad {
    fn new(kind: u8, frequency: f32, gain: i32, rate: i32) -> Self {
        let a = 10f32.powf(gain as f32 / 40.0);
        let w = 2.0 * std::f32::consts::PI * frequency.min(rate as f32 * 0.42) / rate as f32;
        let (s, c) = w.sin_cos();
        let alpha = s / 2.0 * 2f32.sqrt();
        let beta = 2.0 * a.sqrt() * alpha;
        let (b, a0, a1, a2) = match kind {
            0 => (
                [
                    a * ((a + 1.0) - (a - 1.0) * c + beta),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                    a * ((a + 1.0) - (a - 1.0) * c - beta),
                ],
                (a + 1.0) + (a - 1.0) * c + beta,
                -2.0 * ((a - 1.0) + (a + 1.0) * c),
                (a + 1.0) + (a - 1.0) * c - beta,
            ),
            1 => {
                let alpha = s / 2.0;
                (
                    [1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a],
                    1.0 + alpha / a,
                    -2.0 * c,
                    1.0 - alpha / a,
                )
            }
            _ => (
                [
                    a * ((a + 1.0) + (a - 1.0) * c + beta),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                    a * ((a + 1.0) + (a - 1.0) * c - beta),
                ],
                (a + 1.0) - (a - 1.0) * c + beta,
                2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - beta,
            ),
        };
        Self {
            b: b.map(|v| v / a0),
            a: [a1 / a0, a2 / a0],
            z: [0.0; 2],
        }
    }
    fn sample(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

pub fn process(samples: &mut [f32], rate: i32, mixer: &Mixer, volume: i32) {
    if samples.is_empty() {
        return;
    }
    // Remove invalid values and DC before EQ; never turn digital silence into noise.
    let mean = samples
        .iter()
        .filter(|x| x.is_finite())
        .map(|x| *x as f64)
        .sum::<f64>()
        / samples.len() as f64;
    for x in samples.iter_mut() {
        *x = if x.is_finite() { *x - mean as f32 } else { 0.0 };
    }
    for (kind, hz, db) in [
        (0, 180.0, mixer.low_db),
        (1, 1500.0, mixer.presence_db),
        (2, 2700.0, mixer.high_db),
    ] {
        if db != 0 {
            let mut eq = Biquad::new(kind, hz, db, rate);
            for x in samples.iter_mut() {
                *x = eq.sample(*x);
            }
        }
    }
    let rms = active_rms(samples);
    let leveling = if mixer.normalize && rms > 1e-7 {
        (db_gain(-18.0) / rms).clamp(0.1, 32.0)
    } else {
        1.0
    };
    for x in samples.iter_mut() {
        *x *= leveling;
    }
    if mixer.compression > 0 {
        let attack = (-1.0 / (rate as f32 * 0.004)).exp();
        let release = (-1.0 / (rate as f32 * 0.090)).exp();
        let ratio = 1.0 + mixer.compression as f32 * 0.05;
        let mut envelope = 0.0f32;
        for x in samples.iter_mut() {
            let a = if x.abs() > envelope { attack } else { release };
            envelope = a * envelope + (1.0 - a) * x.abs();
            let db = 20.0 * envelope.max(1e-9).log10();
            let reduction = (db + 16.0).max(0.0) * (1.0 - 1.0 / ratio);
            *x *= db_gain(-reduction);
        }
        // Match the voice level after dynamics processing, preserving its envelope.
        if mixer.normalize {
            let now = active_rms(samples);
            if now > 1e-7 {
                let g = (db_gain(-18.0) / now).min(4.0);
                for x in samples.iter_mut() {
                    *x *= g;
                }
            }
        }
    }
    let gain = db_gain(mixer.gain_db as f32);
    // 5 ms look-ahead peak limiter, 80 ms release, -1 dBFS sample ceiling.
    let ceiling = db_gain(-1.0);
    let release = (-1.0 / (rate as f32 * 0.080)).exp();
    let ahead = (rate / 200).max(1) as usize;
    let mut peaks = std::collections::VecDeque::<usize>::new();
    let mut limiter = 1.0f32;
    let mut next = 0;
    let output_volume = volume.clamp(0, 100) as f32 / 100.0;
    // Use an immutable peak copy: samples are overwritten only after their future peak is known.
    let amplitude: Vec<f32> = samples.iter().map(|x| x.abs() * gain).collect();
    for (i, x) in samples.iter_mut().enumerate() {
        while next < amplitude.len() && next <= i + ahead {
            while peaks
                .back()
                .is_some_and(|&j| amplitude[j] <= amplitude[next])
            {
                peaks.pop_back();
            }
            peaks.push_back(next);
            next += 1;
        }
        while peaks.front().is_some_and(|&j| j < i) {
            peaks.pop_front();
        }
        let max = peaks.front().map(|&j| amplitude[j]).unwrap_or(0.0);
        let desired = if max > ceiling { ceiling / max } else { 1.0 };
        limiter = if desired < limiter {
            desired
        } else {
            release * limiter + (1.0 - release) * desired
        };
        *x = (*x * gain * limiter).clamp(-ceiling, ceiling) * output_volume;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn library_roundtrip_and_reject_invalid_parameters() {
        let presets = presets();
        let data = serde_json::to_vec(
            &serde_json::json!({"format":"agent-voice-tunings-v1","presets":presets}),
        )
        .unwrap();
        assert_eq!(read_library(&data).unwrap().len(), 7);
        let mut invalid = serde_json::from_slice::<serde_json::Value>(&data).unwrap();
        invalid["presets"][0]["mixer"]["gain_db"] = 99.into();
        assert!(read_library(&serde_json::to_vec(&invalid).unwrap()).is_err());
        assert!(read_library(br#"{"presets":[]}"#).is_err());
    }
    fn signal(scale: f32) -> Vec<f32> {
        (0..16000)
            .map(|i| scale * (i as f32 * 0.26).sin() * (0.6 + 0.4 * (i as f32 * 0.001).cos()))
            .collect()
    }
    #[test]
    fn quiet_speakers_match_loud_speakers_and_mute_is_exact() {
        let mut quiet = signal(0.01);
        let mut loud = signal(0.4);
        process(&mut quiet, 8000, &Mixer::default(), 100);
        process(&mut loud, 8000, &Mixer::default(), 100);
        assert!((20.0 * (active_rms(&quiet) / active_rms(&loud)).log10()).abs() < 0.3);
        assert!(active_rms(&quiet) > 0.10);
        process(&mut loud, 8000, &Mixer::default(), 0);
        assert!(loud.iter().all(|x| *x == 0.0));
    }
    #[test]
    fn boosted_eq_has_headroom_and_silence_stays_silent() {
        let mut audio = signal(0.8);
        audio[100] = 5.0;
        process(
            &mut audio,
            8000,
            &Mixer {
                gain_db: 18,
                low_db: 12,
                presence_db: 12,
                high_db: 12,
                compression: 100,
                ..Mixer::default()
            },
            100,
        );
        assert!(audio.iter().all(|x| x.is_finite() && x.abs() <= 0.892));
        let mut silence = vec![0.0; 8000];
        process(&mut silence, 8000, &Mixer::default(), 100);
        assert!(silence.iter().all(|x| *x == 0.0));
    }
    #[test]
    fn pause_adjustment_preserves_quiet_syllables_and_bounds_expansion() {
        let quiet: Vec<f32> = (0..8000).map(|i| (i as f32 * 0.13).sin() * 0.003).collect();
        let mut same = quiet.clone();
        scale_pauses(&mut same, 8000, 15);
        assert_eq!(quiet, same); // Every sample is below upstream's 0.01.
        let mut separated = quiet.clone();
        separated.extend(vec![0.0; 4000]);
        separated.extend_from_slice(&quiet);
        let original = separated.clone();
        scale_pauses(&mut separated, 8000, 15);
        assert_eq!(&separated[..8000], quiet.as_slice());
        assert_eq!(&separated[separated.len() - 8000..], quiet.as_slice());
        assert_eq!(separated.len(), 16000 + 1280);
        let mut longer = original.clone();
        scale_pauses(&mut longer, 8000, 200);
        assert_eq!(longer.len(), original.len() + 4000);
        assert_eq!(&longer[..8000], quiet.as_slice());
        assert_eq!(&longer[longer.len() - 8000..], quiet.as_slice());
        let mut baseline = original.clone();
        scale_pauses(&mut baseline, 8000, 100);
        assert_eq!(baseline, original);
    }
}
