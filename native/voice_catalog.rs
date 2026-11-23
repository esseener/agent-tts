use crate::{mixer::Mixer, model::Profile};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub struct Selection {
    pub sid: i32,
    pub title: &'static str,
    pub description: &'static str,
}
impl Selection {
    pub fn voice(&self) -> String {
        format!(
            "local:kokoro-v1.1:{}",
            catalog().voices[self.sid as usize].speaker
        )
    }
    pub fn apply(&self, profile: &mut Profile) {
        profile.voice = self.voice();
        stable_tuning(profile);
    }
    pub fn matches(&self, profile: &Profile) -> bool {
        let mut candidate = profile.clone();
        self.apply(&mut candidate);
        candidate.voice == profile.voice
            && candidate.rate == profile.rate
            && candidate.pitch == profile.pitch
            && candidate.mixer == profile.mixer
    }
}
pub fn stable_tuning(profile: &mut Profile) {
    profile.rate = 0;
    profile.pitch = 0;
    profile.mixer = Mixer::default();
}
pub fn selections() -> &'static [Selection; 6] {
    &[
        Selection {
            sid: 3,
            title: "晓禾 · 女声",
            description: "Kokoro · zf_001 · 原速原调",
        },
        Selection {
            sid: 59,
            title: "云舟 · 男声",
            description: "Kokoro · zm_010 · 原速原调",
        },
        Selection {
            sid: 9,
            title: "晴音 · 女声",
            description: "Kokoro · zf_007 · 原速原调",
        },
        Selection {
            sid: 75,
            title: "远川 · 男声",
            description: "Kokoro · zm_041 · 原速原调",
        },
        Selection {
            sid: 21,
            title: "知夏 · 女声",
            description: "Kokoro · zf_032 · 原速原调",
        },
        Selection {
            sid: 11,
            title: "若晴 · 女声",
            description: "Kokoro · zf_017 · 原速原调",
        },
    ]
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Voice {
    pub id: i32,
    pub speaker: String,
    pub name: String,
    pub gender: String,
    #[serde(default)]
    pub age_group: String,
    #[serde(default)]
    pub accent: String,
    #[serde(default)]
    pub language: String,
    pub featured: bool,
}
#[derive(Deserialize, Serialize)]
pub struct Catalog {
    pub source: String,
    pub metadata_sha256: String,
    pub name_note: String,
    pub voices: Vec<Voice>,
}
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/kokoro-voice-catalog.json"))
            .expect("embedded voice catalog")
    })
}
pub fn get(voice: &str) -> Option<&'static Voice> {
    if let Some(name) = voice.strip_prefix("local:kokoro-v1.1:") {
        return catalog().voices.iter().find(|v| v.speaker == name);
    }
    let speaker = voice.strip_prefix(crate::hojo::VOICE_PREFIX)?;
    crate::hojo::catalog()
        .iter()
        .find(|v| v.speaker == speaker)
}
/// Upgrade only the retired local model. Leave files untouched until the user
/// saves, and preserve profile identity, names, phrases, volume and host routes.
pub fn upgrade_profile(profile: &mut Profile) -> bool {
    let Some(selection) = legacy_selection(&profile.voice) else {
        return false;
    };
    selection.apply(profile);
    true
}

pub fn legacy_selection(voice: &str) -> Option<&'static Selection> {
    let id = voice
        .strip_prefix("local:aishell3:")
        .and_then(|s| s.parse::<i32>().ok())?;
    static LEGACY: OnceLock<Catalog> = OnceLock::new();
    let legacy = LEGACY.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/voice-catalog.json")).expect("legacy catalog")
    });
    let voice = legacy.voices.iter().find(|v| v.id == id)?;
    let choices: &[usize] = if voice.gender == "male" {
        &[1, 3]
    } else {
        &[0, 2, 4, 5]
    };
    Some(&selections()[choices[id as usize % choices.len()]])
}

pub fn toggle_favorite(favorites: &mut Vec<String>, voice: &str) {
    if favorites.iter().any(|v| v == voice) {
        // An explicit removal also retires the old aliases that would add
        // this favorite again on the next configuration load.
        favorites
            .retain(|v| v != voice && !legacy_selection(v).is_some_and(|s| s.voice() == voice));
    } else {
        favorites.push(voice.into());
    }
}
impl Voice {
    pub fn label(&self) -> String {
        let family = if self.speaker.starts_with("hojo_") {
            "Hojo 试用 · 本地 ONNX"
        } else {
            "Kokoro 本地"
        };
        format!(
            "{} · {} · {} · {}",
            self.name,
            if self.gender == "male" {
                "男声"
            } else {
                "女声"
            },
            self.speaker,
            family
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_ids_are_unique_and_attributed() {
        let c = catalog();
        assert_eq!(c.voices.len(), 103);
        let names: std::collections::HashSet<_> = c.voices.iter().map(|v| &v.name).collect();
        assert_eq!(names.len(), 103);
        assert_eq!(
            get(crate::local_tts::DEFAULT_VOICE).unwrap().speaker,
            "zf_001"
        );
        assert_eq!(get("local:kokoro-v1.1:zm_010").unwrap().gender, "male");
        assert_eq!(c.voices.iter().filter(|v| v.language == "zh").count(), 100);
        assert_eq!(get("local:hojo-40m:hojo_zh_f_01").unwrap().gender, "female");
        assert_eq!(get("local:hojo-40m:hojo_zh_m_02").unwrap().gender, "male");
    }
    #[test]
    fn fixed_selections_preserve_user_content_and_are_from_the_current_model() {
        let p = Profile {
            name: "我的名字".into(),
            volume: 72,
            ..Profile::default()
        };
        for selection in selections() {
            assert!(get(&selection.voice()).unwrap().featured);
            let mut changed = p.clone();
            selection.apply(&mut changed);
            assert_eq!(changed.name, p.name);
            assert_eq!(changed.volume, p.volume);
            assert_eq!(changed.phrases, p.phrases);
            assert!(selection.matches(&changed));
        }
    }
    #[test]
    fn removing_an_upgraded_favorite_does_not_restore_it_next_time() {
        let old = "local:aishell3:10";
        let current = legacy_selection(old).unwrap().voice();
        let mut favorites = vec![old.into(), current.clone(), "sapi:default".into()];
        toggle_favorite(&mut favorites, &current);
        assert_eq!(favorites, vec!["sapi:default"]);
        assert!(favorites.iter().all(|v| legacy_selection(v).is_none()));
        toggle_favorite(&mut favorites, &current);
        assert_eq!(favorites, vec!["sapi:default", &current]);
    }
    #[test]
    fn retired_model_upgrade_keeps_content_and_never_changes_other_engines() {
        for id in 0..174 {
            let mut profile = Profile {
                voice: format!("local:aishell3:{id}"),
                name: "自定义".into(),
                rate: 30,
                pitch: 25,
                volume: 63,
                enabled: false,
                ..Profile::default()
            };
            let before = profile.clone();
            assert!(upgrade_profile(&mut profile));
            assert_eq!(profile.name, before.name);
            assert_eq!(profile.id, before.id);
            assert_eq!(profile.enabled, before.enabled);
            assert_eq!(profile.volume, before.volume);
            assert_eq!(profile.phrases, before.phrases);
            assert_eq!(profile.pitch, 0);
            assert_eq!(profile.mixer.pause, 100);
            assert!(get(&profile.voice).is_some());
            assert!(!upgrade_profile(&mut profile));
        }
        for voice in [
            "sapi:default",
            "edge:zh-CN-XiaoxiaoNeural",
            "local:aishell3:999",
        ] {
            let mut profile = Profile {
                voice: voice.into(),
                ..Profile::default()
            };
            assert!(!upgrade_profile(&mut profile));
            assert_eq!(profile.voice, voice);
        }
    }
}
