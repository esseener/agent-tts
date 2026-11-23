use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub type Result<T> = std::result::Result<T, String>;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_REPEAT_WINDOW: u16 = 8;
pub const MAX_REPEAT_WINDOW: u16 = 30;
pub const EVENTS: [(&str, &str); 4] = [
    ("done", "完成任务"),
    ("subagent", "子代理结束"),
    ("error", "发生错误"),
    ("ask", "等待确认"),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub voice: String,
    pub rate: i32,
    pub pitch: i32,
    pub volume: i32,
    pub mixer: crate::mixer::Mixer,
    pub enabled: bool,
    pub phrases: BTreeMap<String, Vec<String>>,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            id: "default".into(),
            name: "默认声音".into(),
            voice: crate::local_tts::DEFAULT_VOICE.into(),
            rate: 0,
            pitch: 0,
            volume: 100,
            mixer: crate::mixer::Mixer::default(),
            enabled: true,
            phrases: EVENTS
                .iter()
                .map(|(e, l)| (e.to_string(), vec![format!("{l}了")]))
                .collect(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Route {
    pub enabled: bool,
    pub profile: String,
    pub text: String,
    /// True while `text` still holds plugin-seeded lines, so a project rename may
    /// re-seed it; any GUI/user edit clears it and the text is then kept forever.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub seeded: bool,
    pub audio: String,
}
impl Default for Route {
    fn default() -> Self {
        Self {
            enabled: true,
            profile: "default".into(),
            text: String::new(),
            seeded: false,
            audio: String::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub adapter: String,
    pub enabled: bool,
    /// Same contract as Route::seeded, for the display name.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub name_seeded: bool,
    pub routes: BTreeMap<String, Route>,
}
impl Default for Host {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            adapter: "custom".into(),
            enabled: true,
            name_seeded: false,
            routes: EVENTS
                .iter()
                .map(|(e, _)| (e.to_string(), Route::default()))
                .collect(),
        }
    }
}
pub fn builtins() -> Vec<Host> {
    [
        ("chatgpt", "ChatGPT"),
        ("codex", "Codex"),
        ("claude", "Claude Code"),
        ("opencode", "OpenCode"),
        ("workbuddy", "WorkBuddy"),
        ("gemini", "Gemini CLI"),
        ("cursor", "Cursor"),
        ("windsurf", "Windsurf"),
        ("aider", "Aider"),
        ("goose", "Goose"),
    ]
    .iter()
    .map(|(id, name)| Host {
        id: id.to_string(),
        name: name.to_string(),
        adapter: id.to_string(),
        ..Host::default()
    })
    .collect()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: String,
    pub profiles: Vec<Profile>,
    pub hosts: Vec<Host>,
    pub api_enabled: bool,
    pub api_port: u16,
    pub api_token: String,
    pub repeat_window_seconds: u16,
    pub saved_tunings: Vec<crate::mixer::Preset>,
    pub favorite_voices: Vec<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: VERSION.into(),
            profiles: vec![Profile::default()],
            hosts: builtins(),
            api_enabled: false,
            api_port: 17321,
            api_token: token(),
            repeat_window_seconds: DEFAULT_REPEAT_WINDOW,
            saved_tunings: Vec::new(),
            favorite_voices: Vec::new(),
        }
    }
}
pub fn token() -> String {
    let mut bytes = [0u8; 24];
    let status = unsafe {
        windows_sys::Win32::Security::Cryptography::BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            windows_sys::Win32::Security::Cryptography::BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    assert_eq!(status, 0, "Windows random source unavailable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 48
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
pub(crate) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{} 读取失败：{e}", path.display())),
    }
}
pub(crate) fn parse_json(path: &Path, bytes: &[u8]) -> Result<Value> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| format!("{} 解析失败：{e}", path.display()))?;
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("{} 解析失败：{e}", path.display()))?;
    if !value.is_object() {
        return Err(format!("{} 根节点必须为对象", path.display()));
    }
    Ok(value)
}
pub fn read_json(path: &Path) -> Result<Value> {
    match read_optional(path)? {
        Some(bytes) => parse_json(path, &bytes),
        None => Ok(serde_json::json!({})),
    }
}

fn file_lock_identity(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => { normalized.pop(); }
            _ => normalized.push(component.as_os_str()),
        }
    }
    let mut ancestor = normalized.as_path();
    let mut missing = Vec::new();
    let mut canonical = loop {
        match fs::canonicalize(ancestor) {
            Ok(path) => break path,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                missing.push(ancestor.file_name().ok_or("文件锁路径无效")?);
                ancestor = ancestor.parent().ok_or("文件锁路径无效")?;
            }
            Err(e) => return Err(format!("{} 锁路径解析失败：{e}", path.display())),
        }
    };
    for name in missing.into_iter().rev() {
        canonical.push(name);
    }
    // Canonicalize the nearest existing ancestor so junctions, symlinks and
    // extended-length paths share an identity even before the target exists.
    // Case folding also covers not-yet-created path components on Windows.
    Ok(PathBuf::from(canonical.as_os_str().to_string_lossy().to_uppercase()))
}

/// Cooperating writers lock the normalized target path, without creating it.
/// Unlocked external replacements still require a best-effort snapshot check.
pub fn lock_file(path: &Path, timeout_ms: u32) -> Result<crate::ProcessLock> {
    crate::ProcessLock::acquire(&file_lock_identity(path)?, "File", timeout_ms)
}

pub(crate) fn ensure_unchanged(path: &Path, original: Option<&[u8]>) -> Result<()> {
    if read_optional(path)?.as_deref() != original {
        return Err(format!("{} 已被其他程序修改，请重新载入后重试", path.display()));
    }
    Ok(())
}
pub fn backup(path: &Path) -> Result<Option<PathBuf>> {
    if !path.is_file() {
        return Ok(None);
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let saved = path.with_file_name(format!(
        "{}.bak-{stamp}",
        path.file_name().unwrap().to_string_lossy()
    ));
    fs::copy(path, &saved).map_err(|e| e.to_string())?;
    Ok(Some(saved))
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().ok_or("路径无效")?).map_err(|e| e.to_string())?;
    let tmp = path.with_extension(format!("{}.tmp", token()));
    let result = (|| {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        f.write_all(bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        use std::os::windows::ffi::OsStrExt;
        let src: Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
        let dst: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            if windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                src.as_ptr(),
                dst.as_ptr(),
                windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                    | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    })();
    if tmp.exists() {
        let _ = fs::remove_file(tmp);
    }
    result
}
pub struct Store {
    pub home: PathBuf,
    pub path: PathBuf,
    original: Option<Vec<u8>>,
    pub migrated: bool,
    pub upgraded_voices: usize,
}
impl Store {
    pub fn load(home: PathBuf) -> Result<(Self, Config)> {
        let path = home.join(".agent-voice/config.json");
        let original = read_optional(&path)?;
        let legacy = if original.is_none() {
            let legacy_path = home.join(".workbuddy/voice/agents.json");
            read_optional(&legacy_path)?.as_deref()
                .map(|bytes| parse_json(&legacy_path, bytes)).transpose()?
        } else {
            None
        };
        let migrated = legacy.is_some();
        let mut config: Config = if let Some(bytes) = original.as_deref() {
            serde_json::from_value(parse_json(&path, bytes)?)
                .map_err(|e| format!("声音配置无效：{e}"))?
        } else if let Some(legacy) = legacy {
            migrate(&home, legacy)?
        } else {
            Config::default()
        };
        if config.api_port == 0 {
            return Err("本地接口端口不能为 0".into());
        }
        config.version = VERSION.into();
        let mut upgraded_voices = 0;
        for p in &mut config.profiles {
            if crate::voice_catalog::upgrade_profile(p) {
                upgraded_voices += 1;
            }
        }
        // Keep retired IDs usable by older releases and add their current
        // equivalents so the Favorites view stays useful after upgrading.
        let favorites = config.favorite_voices.clone();
        for voice in favorites {
            if let Some(selection) = crate::voice_catalog::legacy_selection(&voice) {
                let current = selection.voice();
                if !config.favorite_voices.contains(&current) {
                    config.favorite_voices.push(current);
                    upgraded_voices += 1;
                }
            }
        }
        let default_profile = config
            .profiles
            .first()
            .map(|p| p.id.clone())
            .unwrap_or_default();
        for mut known in builtins() {
            if !config.hosts.iter().any(|h| h.id == known.id) {
                for r in known.routes.values_mut() {
                    r.profile = default_profile.clone();
                    r.enabled = !default_profile.is_empty();
                }
                config.hosts.push(known);
            }
        }
        for host in &mut config.hosts {
            for (e, _) in EVENTS {
                host.routes.entry(e.into()).or_insert_with(|| Route {
                    profile: default_profile.clone(),
                    enabled: !default_profile.is_empty(),
                    ..Route::default()
                });
            }
        }
        validate(&config)?;
        Ok((
            Self {
                home,
                path,
                original,
                migrated,
                upgraded_voices,
            },
            config,
        ))
    }
    pub fn save(&mut self, config: &Config) -> Result<()> {
        validate(config)?;
        let data = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
        let _lock = lock_file(&self.path, 5000)?;
        ensure_unchanged(&self.path, self.original.as_deref())?;
        backup(&self.path)?;
        ensure_unchanged(&self.path, self.original.as_deref())?;
        atomic_write(&self.path, &data)?;
        self.original = Some(data);
        self.migrated = false;
        Ok(())
    }
}
pub fn validate(c: &Config) -> Result<()> {
    let mut ids = std::collections::HashSet::new();
    for p in &c.profiles {
        p.mixer.validate()?;
        if p.name.trim().is_empty() || p.name.chars().count() > 100 {
            return Err("声音名称应为 1～100 个字符".into());
        }
        if !valid_id(&p.id) || !ids.insert(p.id.clone()) {
            return Err("声音标识无效或重复".into());
        }
        if !(-40..=40).contains(&p.rate)
            || !(-40..=40).contains(&p.pitch)
            || !(0..=100).contains(&p.volume)
        {
            return Err("语速和音高范围为 -40～40，音量为 0～100".into());
        }
    }
    if c.saved_tunings.len() > 128 {
        return Err("自定义调音预设最多 128 个".into());
    }
    for preset in &c.saved_tunings {
        preset.validate()?;
    }
    let mut hosts = std::collections::HashSet::new();
    for h in &c.hosts {
        if h.name.trim().is_empty() || h.name.chars().count() > 100 {
            return Err("宿主名称应为 1～100 个字符".into());
        }
        if !valid_id(&h.id) || !hosts.insert(&h.id) {
            return Err("宿主标识无效或重复".into());
        }
        for r in h.routes.values() {
            if !r.profile.is_empty() && !ids.contains(&r.profile) {
                return Err(format!("{} 绑定了不存在的声音", h.name));
            }
            if r.enabled && r.profile.is_empty() {
                return Err(format!("{} 有启用的状态尚未选择声音", h.name));
            }
        }
    }
    if c.api_port == 0 || c.api_token.len() < 24 {
        return Err("本地接口端口或密钥无效".into());
    }
    if c.repeat_window_seconds > MAX_REPEAT_WINDOW {
        return Err(format!("重复播报合并请输入 0～{MAX_REPEAT_WINDOW} 秒"));
    }
    Ok(())
}
fn migrate(home: &Path, legacy: Value) -> Result<Config> {
    if !legacy["agents"].is_object() {
        return Err("旧版 agents 字段无效，未改写配置".into());
    }
    let mut c = Config::default();
    c.profiles.clear();
    if let Some(agents) = legacy["agents"].as_object() {
        for (id, a) in agents {
            if !valid_id(id) {
                continue;
            }
            let num = |k: &str| {
                a.get(k)
                    .or_else(|| legacy["defaults"].get(k))
                    .and_then(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .or_else(|| v.as_i64().map(|n| n.to_string()))
                    })
                    .unwrap_or_else(|| "0".into())
                    .trim_end_matches('%')
                    .trim_end_matches("Hz")
                    .trim_start_matches('+')
                    .parse::<i32>()
                    .unwrap_or(0)
            };
            let name = a["name"].as_str().unwrap_or(id);
            let mut p = Profile {
                id: id.clone(),
                name: if name.eq_ignore_ascii_case("codes") || name.eq_ignore_ascii_case("codex") {
                    "Codex".into()
                } else {
                    name.into()
                },
                voice: format!(
                    "edge:{}",
                    a["voice"]
                        .as_str()
                        .or_else(|| legacy["defaults"]["voice"].as_str())
                        .unwrap_or("zh-CN-XiaoxiaoNeural")
                ),
                rate: num("rate").clamp(-40, 40),
                pitch: num("pitch").clamp(-40, 40),
                volume: (100 + num("volume")).clamp(0, 100),
                enabled: a["enabled"].as_bool().unwrap_or(true),
                ..Profile::default()
            };
            for (e, _) in EVENTS {
                let phrases = if a["phrases"].is_array() {
                    if e == "done" {
                        &a["phrases"]
                    } else if e == "ask" {
                        &a["ask_phrases"]
                    } else {
                        &Value::Null
                    }
                } else if e == "ask" && a["phrases"].is_null() {
                    &a["ask_phrases"]
                } else {
                    &a["phrases"][e]
                };
                let lines = phrases
                    .as_array()
                    .map(|v| {
                        v.iter()
                            .filter_map(|s| {
                                s.as_str().map(|s| {
                                    if id == "codes" {
                                        s.replace("codes", "Codex")
                                    } else {
                                        s.into()
                                    }
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                p.phrases.insert(e.into(), lines);
            }
            c.profiles.push(p);
        }
    }
    let bindings = read_json(&home.join(".workbuddy/voice/app-bindings.json"))?;
    for h in &mut c.hosts {
        if c.profiles.is_empty() {
            for r in h.routes.values_mut() {
                r.profile.clear();
                r.enabled = false;
            }
            continue;
        }
        let prior = bindings[&h.id]
            .as_str()
            .map(str::to_string)
            .or_else(|| legacy_binding(home, &h.adapter));
        let p = prior
            .as_deref()
            .and_then(|id| c.profiles.iter().find(|p| p.id == id))
            .or_else(|| c.profiles.iter().find(|p| p.id == h.id))
            .unwrap_or(&c.profiles[0]);
        for (event, r) in h.routes.iter_mut() {
            r.profile = p.id.clone();
            r.enabled = legacy["events"][event]["enabled"].as_bool().unwrap_or(true);
        }
    }
    Ok(c)
}

fn legacy_binding(home: &Path, adapter: &str) -> Option<String> {
    let mut paths = vec![crate::integrations::target(home, adapter)?];
    if adapter == "opencode" {
        paths.push(home.join(".config/opencode/plugins/agent-voice.js"));
    }
    for path in paths {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for tail in text.split("speak.cmd").skip(1) {
            let tail =
                tail.trim_start_matches(|c: char| c == '"' || c == '\\' || c.is_whitespace());
            let id: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if valid_id(&id) {
                return Some(id);
            }
        }
    }
    None
}

/// Pick a random element from a non-empty slice.
fn pick<'a, T>(items: &'a [T]) -> &'a T {
    // Coarse clock granularity biases nearby calls; a counter + xorshift mixes
    // consecutive picks into different slots.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut x = seed
        ^ COUNTER.fetch_add(0x9E3779B97F4A7C15, std::sync::atomic::Ordering::Relaxed);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    &items[(x as usize) % items.len()]
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Event {
    pub host: String,
    pub event: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub event_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_id: String,
}
impl Event {
    pub fn validate(&self) -> Result<()> {
        if !valid_id(&self.host)
            || !EVENTS.iter().any(|(key, _)| *key == self.event)
            || self.text.len() > 8192
            || self.event_id.len() > 512
            || self.session_id.len() > 512
        {
            return Err("宿主、状态、文本或事件身份无效".into());
        }
        Ok(())
    }

    pub fn apply_hook_input(&mut self, input: &Value) {
        let field = |key: &str| input.get(key).and_then(Value::as_str).filter(|s| !s.is_empty());
        if self.host == "cursor" && self.event == "done" && input["status"] == "error" {
            self.event = "error".into();
        }
        if self.session_id.is_empty() {
            if let Some(id) = field("session_id").or_else(|| {
                (self.host == "cursor").then(|| field("conversation_id")).flatten()
            }) {
                self.session_id = id.into();
            }
        }
        if self.event_id.is_empty() {
            let id = field("event_id").or_else(|| match self.event.as_str() {
                "done" => field("turn_id").or_else(|| field("generation_id")),
                "subagent" => field("agent_id"),
                "ask" | "error" => field("tool_use_id").or_else(|| {
                    (self.host == "cursor").then(|| field("generation_id")).flatten()
                }),
                _ => None,
            });
            if let Some(id) = id {
                self.event_id = id.into();
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct Playback {
    pub profile: Profile,
    pub text: String,
    pub audio: String,
}
pub fn resolve(c: &Config, e: &Event) -> Result<Option<Playback>> {
    e.validate()?;
    let host = c
        .hosts
        .iter()
        .find(|h| h.id == e.host)
        .ok_or("找不到宿主")?;
    if !host.enabled {
        return Ok(None);
    }
    let route = host.routes.get(&e.event).ok_or("未配置该状态")?;
    if !route.enabled {
        return Ok(None);
    }
    if route.profile.is_empty() {
        return Ok(None);
    }
    let profile = c
        .profiles
        .iter()
        .find(|p| p.id == route.profile)
        .ok_or("绑定的声音已不存在")?;
    if !profile.enabled {
        return Ok(None);
    }
    let text = if !e.text.is_empty() {
        e.text.clone()
    } else if !route.text.trim().is_empty() {
        // One announcement per line; a random non-empty line is spoken each time.
        let lines: Vec<&str> = route
            .text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        pick(&lines).to_string()
    } else {
        let lines = profile.phrases.get(&e.event).filter(|v| !v.is_empty());
        lines
            .map(|v| pick(v.as_slice()).clone())
            .unwrap_or_else(|| {
                format!(
                    "{}，{}",
                    host.name,
                    EVENTS.iter().find(|(key, _)| *key == e.event).unwrap().1
                )
            })
    };
    if text.len() > 8192 {
        return Err("播报内容过长".into());
    }
    Ok(Some(Playback {
        profile: profile.clone(),
        text,
        audio: route.audio.clone(),
    }))
}

/// Remove a profile and detach all routes that reference it, including muted ones.
pub fn delete_profile(c: &mut Config, id: &str) -> Result<usize> {
    let index = c
        .profiles
        .iter()
        .position(|p| p.id == id)
        .ok_or("找不到声音")?;
    let mut detached = 0;
    for host in &mut c.hosts {
        for route in host.routes.values_mut() {
            if route.profile == id {
                route.profile.clear();
                route.enabled = false;
                detached += 1;
            }
        }
    }
    c.profiles.remove(index);
    Ok(detached)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simultaneous_saves_from_one_snapshot_have_one_winner() {
        use std::sync::{Arc, Barrier};
        let root = std::env::temp_dir().join(token());
        let (mut seed, config) = Store::load(root.clone()).unwrap();
        seed.save(&config).unwrap();
        let original = fs::read(&seed.path).unwrap();
        let (mut first_store, mut first_config) = Store::load(root.clone()).unwrap();
        let (mut second_store, mut second_config) = Store::load(root.clone()).unwrap();
        first_config.profiles[0].name = "first writer".into();
        second_config.profiles[0].name = "second writer".into();
        let barrier = Arc::new(Barrier::new(3));
        let first_barrier = Arc::clone(&barrier);
        let first = std::thread::spawn(move || {
            first_barrier.wait();
            (first_store.save(&first_config), first_config.profiles[0].name.clone())
        });
        let second_barrier = Arc::clone(&barrier);
        let second = std::thread::spawn(move || {
            second_barrier.wait();
            (second_store.save(&second_config), second_config.profiles[0].name.clone())
        });
        barrier.wait();
        let results = [first.join().unwrap(), second.join().unwrap()];
        assert_eq!(results.iter().filter(|(result, _)| result.is_ok()).count(), 1);
        let loser = results.iter().find(|(result, _)| result.is_err()).unwrap();
        assert!(loser.0.as_ref().unwrap_err().contains("已被其他程序修改"));
        let winner = results.iter().find(|(result, _)| result.is_ok()).unwrap();
        let (_, loaded) = Store::load(root.clone()).unwrap();
        assert_eq!(loaded.profiles[0].name, winner.1);
        let backups: Vec<_> = fs::read_dir(seed.path.parent().unwrap()).unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.file_name().unwrap().to_string_lossy().starts_with("config.json.bak-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0]).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_lock_aliases_share_identity_before_and_after_creation() {
        let root = std::env::temp_dir().join(token());
        let parent = root.join("MixedParent");
        fs::create_dir_all(parent.join("Other")).unwrap();
        let target = parent.join("Missing/deep/Config.json");
        let alias = fs::canonicalize(&parent).unwrap()
            .join("Other/../missing/DEEP/config.JSON");
        let guard = lock_file(&target, 0).unwrap();
        assert!(!target.parent().unwrap().exists());
        let missing_alias = alias.clone();
        assert!(std::thread::spawn(move || lock_file(&missing_alias, 0).is_err()).join().unwrap());
        atomic_write(&target, b"untouched").unwrap();
        let existing_alias = alias.clone();
        assert!(std::thread::spawn(move || lock_file(&existing_alias, 0).is_err()).join().unwrap());
        drop(guard);
        assert!(std::thread::spawn(move || lock_file(&alias, 0).is_ok()).join().unwrap());
        assert_eq!(fs::read(&target).unwrap(), b"untouched");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bom_bytes_are_the_parse_snapshot_and_backup_source() {
        let root = std::env::temp_dir().join(token());
        let path = root.join(".agent-voice/config.json");
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend(serde_json::to_vec(&Config::default()).unwrap());
        atomic_write(&path, &bytes).unwrap();
        let (mut store, mut config) = Store::load(root.clone()).unwrap();
        assert_eq!(store.original.as_deref(), Some(bytes.as_slice()));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        config.profiles[0].name = "updated".into();
        store.save(&config).unwrap();
        let saved = fs::read_dir(path.parent().unwrap()).unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.file_name().unwrap().to_string_lossy().starts_with("config.json.bak-"))
            .unwrap();
        assert_eq!(fs::read(saved).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn read_errors_are_not_missing_configuration() {
        let root = std::env::temp_dir().join(token());
        let (mut store, config) = Store::load(root.clone()).unwrap();
        fs::create_dir_all(&store.path).unwrap();
        fs::write(store.path.join("keep"), b"untouched").unwrap();
        assert!(Store::load(root.clone()).is_err());
        assert!(read_json(&store.path).is_err());
        assert!(store.save(&config).is_err());
        assert_eq!(fs::read(store.path.join("keep")).unwrap(), b"untouched");
        assert_eq!(fs::read_dir(store.path.parent().unwrap()).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_or_unreadable_legacy_sources_do_not_become_defaults() {
        let root = std::env::temp_dir().join(token());
        let legacy = root.join(".workbuddy/voice/agents.json");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("keep"), b"untouched").unwrap();
        assert!(Store::load(root.clone()).is_err());
        assert!(!root.join(".agent-voice").exists());
        assert_eq!(fs::read(legacy.join("keep")).unwrap(), b"untouched");
        fs::remove_dir_all(&legacy).unwrap();
        fs::write(&legacy, b"broken").unwrap();
        assert!(Store::load(root.clone()).is_err());
        assert!(!root.join(".agent-voice").exists());
        assert_eq!(fs::read(legacy).unwrap(), b"broken");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_migration_is_read_only_until_explicit_save() {
        let root = std::env::temp_dir().join(token());
        let legacy = root.join(".workbuddy/voice/agents.json");
        let bytes = br#"{"agents":{"legacy":{"name":"Legacy","voice":"zh-CN-XiaoxiaoNeural","phrases":["legacy done"]}}}"#;
        atomic_write(&legacy, bytes).unwrap();
        let (mut store, config) = Store::load(root.clone()).unwrap();
        assert!(store.migrated);
        assert!(store.original.is_none());
        assert_eq!(config.profiles[0].id, "legacy");
        assert_eq!(config.profiles[0].phrases["done"], vec!["legacy done"]);
        assert!(!root.join(".agent-voice").exists());
        assert_eq!(fs::read(&legacy).unwrap(), bytes);
        store.save(&config).unwrap();
        assert!(!store.migrated);
        assert_eq!(fs::read(legacy).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hook_identity_separates_sessions_turns_and_subagents() {
        let mut e = Event { host: "codex".into(), event: "done".into(), ..Event::default() };
        e.apply_hook_input(&serde_json::json!({"session_id":"session-a","turn_id":"turn-1"}));
        assert_eq!((&*e.session_id, &*e.event_id), ("session-a", "turn-1"));
        let mut next = Event { host: "codex".into(), event: "subagent".into(), ..Event::default() };
        next.apply_hook_input(&serde_json::json!({"session_id":"session-b","agent_id":"agent-2"}));
        assert_eq!((&*next.session_id, &*next.event_id), ("session-b", "agent-2"));
        let mut cursor = Event { host: "cursor".into(), event: "done".into(), ..Event::default() };
        cursor.apply_hook_input(&serde_json::json!({"conversation_id":"conversation","generation_id":"generation"}));
        assert_eq!((&*cursor.session_id, &*cursor.event_id), ("conversation", "generation"));
        cursor.apply_hook_input(&serde_json::json!({"status":"error","session_id":"other","event_id":"other"}));
        assert_eq!(cursor.event, "error");
        assert_eq!((&*cursor.session_id, &*cursor.event_id), ("conversation", "generation"));
        let mut failed = Event { host: "cursor".into(), event: "done".into(), ..Event::default() };
        failed.apply_hook_input(&serde_json::json!({"status":"error","conversation_id":"conversation","generation_id":"failed-generation"}));
        assert_eq!((&*failed.event, &*failed.session_id, &*failed.event_id), ("error", "conversation", "failed-generation"));
    }

    #[test]
    fn legacy_events_load_and_identity_is_bounded() {
        let mut e: Event = serde_json::from_str(r#"{"host":"codex","event":"done"}"#).unwrap();
        assert!(e.event_id.is_empty() && e.session_id.is_empty());
        e.event_id = "e".repeat(512);
        e.session_id = "s".repeat(512);
        assert!(e.validate().is_ok());
        e.session_id.push('s');
        assert!(e.validate().is_err());
        e.session_id.clear();
        e.event_id.push('e');
        assert!(e.validate().is_err());
    }

    #[test]
    fn independent_per_status_routes_and_mute() {
        let mut c = Config::default();
        c.profiles.push(Profile {
            id: "second".into(),
            ..Profile::default()
        });
        c.hosts[0].routes.get_mut("error").unwrap().profile = "second".into();
        let mut e = Event {
            host: "chatgpt".into(),
            event: "error".into(),
            text: String::new(),
            ..Event::default()
        };
        assert_eq!(resolve(&c, &e).unwrap().unwrap().profile.id, "second");
        e.event = "done".into();
        assert_eq!(resolve(&c, &e).unwrap().unwrap().profile.id, "default");
        c.hosts[0].routes.get_mut("done").unwrap().enabled = false;
        assert!(resolve(&c, &e).unwrap().is_none());
    }
    #[test]
    fn reject_invalid_or_duplicate_ids() {
        let mut c = Config::default();
        c.hosts[0].id = "../../bad".into();
        assert!(validate(&c).is_err());
    }
    #[test]
    fn text_override_and_audio_preserved() {
        let mut c = Config::default();
        let r = c.hosts[0].routes.get_mut("done").unwrap();
        r.text = "host text".into();
        r.audio = "clip.wav".into();
        let e = Event {
            host: "chatgpt".into(),
            event: "done".into(),
            text: String::new(),
            ..Event::default()
        };
        let p = resolve(&c, &e).unwrap().unwrap();
        assert_eq!(p.text, "host text");
        assert_eq!(p.audio, "clip.wav");
    }
    #[test]
    fn deleting_all_profiles_mutes_routes_and_reloads() {
        let root = std::env::temp_dir().join(token());
        let (mut store, mut c) = Store::load(root.clone()).unwrap();
        assert_eq!(delete_profile(&mut c, "default").unwrap(), 40);
        assert!(validate(&c).is_ok());
        store.save(&c).unwrap();
        let (_, empty) = Store::load(root.clone()).unwrap();
        assert!(empty.profiles.is_empty());
        assert!(empty.hosts.iter().all(|h| h
            .routes
            .values()
            .all(|r| !r.enabled && r.profile.is_empty())));
        c.profiles.push(Profile {
            id: "new".into(),
            ..Profile::default()
        });
        assert!(validate(&c).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn model_upgrade_is_read_only_until_save_and_keeps_routes() {
        let root = std::env::temp_dir().join(token());
        let (mut store, mut original) = Store::load(root.clone()).unwrap();
        original.profiles[0].voice = "local:aishell3:10".into();
        original.profiles[0].name = "保留自定义声音".into();
        original.profiles[0].volume = 61;
        original.favorite_voices = vec!["local:aishell3:10".into(), "sapi:default".into()];
        original.hosts[0].routes.get_mut("done").unwrap().text = "保留宿主覆盖台词".into();
        original.hosts[0].routes.get_mut("ask").unwrap().enabled = false;
        store.save(&original).unwrap();
        let bytes = fs::read(&store.path).unwrap();
        let (mut loaded, config) = Store::load(root.clone()).unwrap();
        assert_eq!(loaded.upgraded_voices, 2);
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        assert_eq!(config.profiles[0].name, original.profiles[0].name);
        assert_eq!(config.profiles[0].volume, 61);
        assert!(config
            .favorite_voices
            .starts_with(&original.favorite_voices));
        assert!(config.favorite_voices.contains(&config.profiles[0].voice));
        assert_eq!(
            serde_json::to_value(&config.hosts).unwrap(),
            serde_json::to_value(&original.hosts).unwrap()
        );
        assert!(config.profiles[0].voice.starts_with("local:kokoro-v1.1:"));
        loaded.save(&config).unwrap();
        let (reloaded, _) = Store::load(root.clone()).unwrap();
        assert_eq!(reloaded.upgraded_voices, 0);
        assert!(fs::read_dir(root.join(".agent-voice"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("config.json.bak-")));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn storage_conflict_and_corrupt_protection() {
        let root = std::env::temp_dir().join(token());
        let (mut store, c) = Store::load(root.clone()).unwrap();
        assert!(!store.path.exists());
        store.save(&c).unwrap();
        fs::write(&store.path, b"broken").unwrap();
        assert!(store.save(&c).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), b"broken");
        assert!(Store::load(root.clone()).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
