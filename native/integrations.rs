use crate::{
    detect,
    model::{
        atomic_write, backup, ensure_unchanged, lock_file, parse_json, read_json, read_optional, Host,
        Result,
    },
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
const MARKER: &str = "agent-voice-native";
pub fn target(home: &Path, adapter: &str) -> Option<PathBuf> {
    Some(home.join(match adapter {
        "codex" => ".codex/hooks.json",
        "claude" => ".claude/settings.json",
        "workbuddy" => ".workbuddy/settings.json",
        "cursor" => ".cursor/hooks.json",
        "gemini" => ".gemini/settings.json",
        "opencode" => ".config/opencode/plugins/agent-voice/index.js",
        _ => return None,
    }))
}
fn ours(h: &Value) -> bool {
    h["command"]
        .as_str()
        .map(|s| {
            s.contains(MARKER)
                || s.to_lowercase()
                    .replace('\\', "/")
                    .contains("/.workbuddy/voice/speak.cmd")
        })
        .unwrap_or(false)
}
pub fn clean(v: &mut Value) -> Result<()> {
    let hooks = v.as_object_mut().ok_or("hooks 必须是对象")?;
    for entries in hooks.values_mut() {
        if let Some(entries) = entries.as_array_mut() {
            entries.retain_mut(|entry| {
                if ours(entry) {
                    return false;
                }
                if let Some(inner) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
                    let before = inner.len();
                    inner.retain(|h| !ours(h));
                    if before > 0 && inner.is_empty() {
                        return false;
                    }
                }
                true
            });
        }
    }
    hooks.retain(|_, v| !v.as_array().map(|a| a.is_empty()).unwrap_or(false));
    Ok(())
}
pub fn installed(home: &Path, host: &Host) -> bool {
    let Some(path) = target(home, &host.adapter) else {
        return false;
    };
    if host.adapter == "opencode" {
        return fs::read_to_string(path)
            .map(|s| s.contains(MARKER))
            .unwrap_or(false);
    }
    let Ok(v) = read_json(&path) else {
        return false;
    };
    v["hooks"]
        .as_object()
        .map(|m| {
            m.values().any(|entries| {
                entries
                    .as_array()
                    .map(|a| {
                        a.iter().any(|e| {
                            e["command"]
                                .as_str()
                                .map(|s| s.contains(MARKER))
                                .unwrap_or(false)
                                || e["hooks"]
                                    .as_array()
                                    .map(|a| {
                                        a.iter().any(|e| {
                                            e["command"]
                                                .as_str()
                                                .map(|s| s.contains(MARKER))
                                                .unwrap_or(false)
                                        })
                                    })
                                    .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}
fn own_plugin(text: &str) -> bool {
    text.contains(MARKER)
        || (text.to_lowercase().contains("speak.cmd")
            && (text.contains("AgentVoice") || text.contains("Agent Voice")))
}
fn command(exe: &Path, home: &Path, host: &str, event: &str, powershell: bool) -> Result<String> {
    let executable = exe.to_string_lossy().replace('\\', "/");
    let root = home.to_string_lossy().replace('\\', "/");
    if [executable.as_str(), root.as_str()]
        .iter()
        .any(|s| s.contains(['\n', '\r', '"', '`', '$']))
    {
        return Err("接入路径不能包含引号、换行、$ 或反引号".into());
    }
    Ok(format!(
        "{}\"{executable}\" --home \"{root}\" --hook {host} {event} --{MARKER}",
        if powershell { "& " } else { "" }
    ))
}
fn mapping(adapter: &str) -> Vec<(&'static str, &'static str)> {
    match adapter {
        "codex" => vec![
            ("done", "Stop"),
            ("subagent", "SubagentStop"),
            ("ask", "PermissionRequest"),
        ],
        "claude" => vec![
            ("done", "Stop"),
            ("subagent", "SubagentStop"),
            ("error", "StopFailure"),
            ("ask", "Notification"),
        ],
        "workbuddy" => vec![
            ("done", "Stop"),
            ("subagent", "SubagentStart"),
            ("error", "StopFailure"),
            ("ask", "Notification"),
        ],
        "cursor" => vec![
            ("done", "stop"),
            ("subagent", "subagentStop"),
            ("error", "postToolUseFailure"),
        ],
        "gemini" => vec![("done", "AfterAgent"), ("ask", "Notification")],
        _ => vec![],
    }
}
fn replace_snapshot(path: &Path, original: Option<&[u8]>, bytes: Option<&[u8]>) -> Result<()> {
    ensure_unchanged(path, original)?;
    backup(path)?;
    // A file mutex coordinates this application, not arbitrary external tools.
    // Recheck after backup and keep the atomic replacement; this is not a CAS.
    ensure_unchanged(path, original)?;
    if let Some(bytes) = bytes {
        atomic_write(path, bytes)
    } else if original.is_some() {
        fs::remove_file(path).map_err(|e| format!("{} 删除失败：{e}", path.display()))
    } else {
        Ok(())
    }
}

fn update_settings(path: &Path, edit: impl FnOnce(&mut Value) -> Result<()>) -> Result<()> {
    let _lock = lock_file(path, 5000)?;
    let original = read_optional(path)?;
    let mut settings = match original.as_deref() {
        Some(bytes) => parse_json(path, bytes)?,
        None => json!({}),
    };
    edit(&mut settings)?;
    let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    replace_snapshot(path, original.as_deref(), Some(&bytes))
}

pub fn update(home: &Path, exe: &Path, host: &Host, on: bool) -> Result<String> {
    if !detect::native(&host.adapter) {
        return Err("该宿主使用通用命令或 HTTP 接口，不改写其配置".into());
    }
    let path = target(home, &host.adapter).ok_or("没有原生适配器")?;
    if host.adapter == "opencode" {
        let old = home.join(".config/opencode/plugins/agent-voice.js");
        // Always acquire both plugin paths in legacy-then-current order.
        let _old_lock = lock_file(&old, 5000)?;
        let _lock = lock_file(&path, 5000)?;
        let original = read_optional(&path)?;
        if let Some(bytes) = original.as_deref() {
            let text = std::str::from_utf8(bytes)
                .map_err(|e| format!("{} 读取失败：{e}", path.display()))?;
            if !own_plugin(text) {
                return Err("同名插件属于其他工具，未覆盖".into());
            }
        }
        let legacy = read_optional(&old)?;
        let remove_legacy = legacy.as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map(own_plugin)
            .unwrap_or(false);
        let generated = if on { Some(plugin(home, exe, &host.id)?) } else { None };
        ensure_unchanged(&old, legacy.as_deref())?;
        replace_snapshot(&path, original.as_deref(), generated.as_deref().map(str::as_bytes))?;
        if remove_legacy {
            replace_snapshot(&old, legacy.as_deref(), None)?;
        }
    } else {
        update_settings(&path, |settings| {
            if host.adapter == "cursor" && settings.get("version").map(|v| v != 1).unwrap_or(false) {
                return Err("Cursor hooks 版本不是 1，未改写该文件".into());
            }
            if settings.get("hooks").is_none() {
                settings["hooks"] = json!({});
            }
            clean(&mut settings["hooks"])?;
            if on {
                for (event, name) in mapping(&host.adapter) {
                    let powershell = host.adapter == "cursor" || host.adapter == "codex";
                    let cmd = command(exe, home, &host.id, event, powershell)?;
                    let mut entry = if host.adapter == "cursor" {
                        json!({"command":cmd,"timeout":5})
                    } else {
                        json!({"hooks":[{"type":"command","command":cmd,"timeout":5}]})
                    };
                    if host.adapter == "codex" {
                        entry["hooks"][0]["commandWindows"] =
                            json!(command(exe, home, &host.id, event, true)?);
                    }
                    if event == "ask" && matches!(host.adapter.as_str(), "claude" | "workbuddy") {
                        entry["matcher"] = json!("permission_prompt");
                    }
                    let object = settings["hooks"]
                        .as_object_mut()
                        .ok_or("hooks 必须为对象")?;
                    let entries = object
                        .entry(name)
                        .or_insert(json!([]))
                        .as_array_mut()
                        .ok_or("hook 列表格式无效")?;
                    entries.push(entry);
                }
            }
            if host.adapter == "cursor" {
                settings["version"] = json!(1);
            }
            Ok(())
        })?;
    }
    Ok(format!(
        "{}：{}。{}",
        host.name,
        if on {
            "已写入接入配置"
        } else {
            "已移除接入"
        },
        if host.adapter == "codex" && on {
            "请在 Codex /hooks 审阅信任，并确认 hooks 未被禁用。"
        } else {
            "原文件已备份。"
        }
    ))
}
pub fn plugin(home: &Path, exe: &Path, host: &str) -> Result<String> {
    let executable = serde_json::to_string(&exe.to_string_lossy()).unwrap();
    let args = serde_json::to_string(&["--home", &home.to_string_lossy(), "--play-event", host]).unwrap();
    Ok(format!(
        r#"// agent-voice-native — generated by Agent Voice
import {{ Plugin }} from "@opencode/plugin";
import {{ spawn }} from "node:child_process";
import {{ win32 }} from "node:path";

// Share in-flight deliveries and successful logical events across Locations.
// Failed children release their key so an upstream replay can retry playback.
const delivery = globalThis[Symbol.for("agent-voice.native.delivery")] ??= {{recent:new Set(),pending:new Set()}};
const directoryKey = directory => win32.normalize(directory).replace(/[\\/]+$/, "").toLowerCase();

export default Plugin.define({{
  id: "agent-voice",
  setup(ctx) {{
    const abort = new AbortController();
    const directory = directoryKey(ctx.location.directory);
    void (async () => {{
      for await (const event of ctx.event.subscribe({{types:["session.execution.succeeded","session.execution.failed","permission.asked"],signal:abort.signal}})) {{
        if (abort.signal.aborted) break;
        const kind = {{"session.execution.succeeded":"done","session.execution.failed":"error","permission.asked":"ask"}}[event.type];
        if (!kind) continue;
        const source = event.location;
        if (source && directoryKey(source.directory) !== directory) continue;
        if (source?.workspaceID && ctx.location.workspaceID && source.workspaceID !== ctx.location.workspaceID) continue;

        // Use logical event IDs, not the spoken text or the session alone:
        // a later turn in the same session may legitimately use the same line.
        const key = kind === "ask" && event.data?.id
          ? "permission:" + event.data.id
          : event.durable
            ? JSON.stringify([event.type, event.durable.aggregateID, event.durable.seq])
            : event.id;
        const session = typeof event.data?.sessionID === "string" ? event.data.sessionID : "";
        const deliveryKey = key ? JSON.stringify([session, key]) : undefined;
        if (deliveryKey && (delivery.recent.has(deliveryKey) || delivery.pending.has(deliveryKey))) continue;
        const args = [...{args}, kind, "--agent-voice-native"];
        if (key) args.push("--event-id", key);
        if (session) args.push("--session-id", session);
        if (deliveryKey) delivery.pending.add(deliveryKey);
        let settled = false;
        const finish = success => {{
          if (settled) return;
          settled = true;
          if (!deliveryKey) return;
          delivery.pending.delete(deliveryKey);
          if (success) {{
            delivery.recent.add(deliveryKey);
            if (delivery.recent.size > 4096) delivery.recent.delete(delivery.recent.values().next().value);
          }}
        }};
        try {{
          const child=spawn({executable}, args, {{windowsHide:true,detached:true,stdio:"ignore"}});
          child.on("error", error => {{finish(false);console.error("[Agent Voice]",error.message);}});
          child.on("exit", code => finish(code === 0));
          child.unref();
        }} catch (error) {{
          finish(false);
          console.error("[Agent Voice]",error);
        }}
      }}
    }})().catch(error => {{if (!abort.signal.aborted) console.error("[Agent Voice]",error);}});
    return () => abort.abort();
  }}
}});
"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_host_writers_read_latest_and_preserve_other_settings() {
        use std::sync::mpsc;
        let home = std::env::temp_dir().join(crate::model::token());
        let path = target(&home, "claude").unwrap();
        atomic_write(&path, br#"{"model":"keep","hooks":{"Stop":[{"command":"other-tool"}]}}"#).unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first_path = path.clone();
        let first = std::thread::spawn(move || {
            update_settings(&first_path, |settings| {
                settings["other_tool"] = json!({"preserved":true});
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
        });
        entered_rx.recv().unwrap();
        let second_home = home.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let second = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let host = crate::model::builtins().remove(2);
            update(&second_home, Path::new("C:/app/AgentVoice.exe"), &host, true)
        });
        started_rx.recv().unwrap();
        release_tx.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        let settings = read_json(&path).unwrap();
        assert_eq!(settings["model"], "keep");
        assert_eq!(settings["other_tool"], json!({"preserved":true}));
        assert_eq!(settings["hooks"]["Stop"][0]["command"], "other-tool");
        assert_eq!(settings["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(installed(&home, &crate::model::builtins().remove(2)));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn unlocked_external_change_is_refused_before_backup_or_replacement() {
        let home = std::env::temp_dir().join(crate::model::token());
        let path = target(&home, "claude").unwrap();
        atomic_write(&path, br#"{"model":"original"}"#).unwrap();
        let external = br#"{"model":"external","other_tool":{"keep":true}}"#;
        let error = update_settings(&path, |settings| {
            settings["model"] = json!("ours");
            atomic_write(&path, external)
        }).unwrap_err();
        assert!(error.contains("已被其他程序修改"));
        assert_eq!(fs::read(&path).unwrap(), external);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn host_read_errors_do_not_create_or_replace_configuration() {
        let home = std::env::temp_dir().join(crate::model::token());
        let path = target(&home, "claude").unwrap();
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("keep"), "untouched").unwrap();
        let host = crate::model::builtins().remove(2);
        assert!(update(&home, Path::new("a.exe"), &host, true).is_err());
        assert_eq!(fs::read(path.join("keep")).unwrap(), b"untouched");
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn opencode_migrates_and_backs_up_both_plugin_locations() {
        let home = std::env::temp_dir().join(crate::model::token());
        let path = target(&home, "opencode").unwrap();
        let old = home.join(".config/opencode/plugins/agent-voice.js");
        let legacy = b"// AgentVoice legacy speak.cmd";
        let current = b"// agent-voice-native previous plugin";
        atomic_write(&old, legacy).unwrap();
        atomic_write(&path, current).unwrap();
        let host = crate::model::builtins().remove(3);
        update(&home, Path::new("C:/app/AgentVoice.exe"), &host, true).unwrap();
        assert!(!old.exists());
        assert!(installed(&home, &host));
        for (file, bytes) in [(&old, legacy.as_slice()), (&path, current.as_slice())] {
            let prefix = format!("{}.bak-", file.file_name().unwrap().to_string_lossy());
            let saved = fs::read_dir(file.parent().unwrap()).unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|entry| entry.file_name().unwrap().to_string_lossy().starts_with(&prefix))
                .unwrap();
            assert_eq!(fs::read(saved).unwrap(), bytes);
        }
        let generated = fs::read(&path).unwrap();
        update(&home, Path::new("C:/app/AgentVoice.exe"), &host, true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), generated);
        update(&home, Path::new("C:/app/AgentVoice.exe"), &host, false).unwrap();
        assert!(!path.exists());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn opencode_keeps_foreign_plugins_and_propagates_legacy_read_errors() {
        let home = std::env::temp_dir().join(crate::model::token());
        let path = target(&home, "opencode").unwrap();
        let old = home.join(".config/opencode/plugins/agent-voice.js");
        let host = crate::model::builtins().remove(3);
        atomic_write(&old, b"// other tool").unwrap();
        update(&home, Path::new("a.exe"), &host, true).unwrap();
        assert_eq!(fs::read(&old).unwrap(), b"// other tool");
        atomic_write(&path, b"// foreign current plugin").unwrap();
        assert!(update(&home, Path::new("a.exe"), &host, false).unwrap_err().contains("属于其他工具"));
        assert_eq!(fs::read(&path).unwrap(), b"// foreign current plugin");
        fs::remove_file(&path).unwrap();
        fs::remove_file(&old).unwrap();
        fs::create_dir(&old).unwrap();
        assert!(update(&home, Path::new("a.exe"), &host, true).is_err());
        assert!(!path.exists());
        assert!(old.is_dir());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn mixed_entries_preserve_other_commands() {
        let mut v = json!({"Stop":[{"matcher":"x","hooks":[{"command":"AgentVoice.exe --agent-voice-native"},{"command":"other"}]}]});
        clean(&mut v).unwrap();
        assert_eq!(v["Stop"][0]["matcher"], "x");
        assert_eq!(v["Stop"][0]["hooks"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn hooks_are_idempotent_and_isolated() {
        let home = std::env::temp_dir().join(crate::model::token());
        let host = crate::model::builtins().remove(1);
        let exe = Path::new("C:/app/AgentVoice.exe");
        update(&home, exe, &host, true).unwrap();
        let p = target(&home, "codex").unwrap();
        let a = read_json(&p).unwrap();
        update(&home, exe, &host, true).unwrap();
        assert_eq!(a, read_json(&p).unwrap());
        assert!(a["hooks"]["PermissionRequest"].is_array());
        update(&home, exe, &host, false).unwrap();
        assert_eq!(read_json(&p).unwrap()["hooks"], json!({}));
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn corrupt_host_never_overwritten() {
        let home = std::env::temp_dir().join(crate::model::token());
        fs::create_dir_all(home.join(".claude")).unwrap();
        let p = home.join(".claude/settings.json");
        fs::write(&p, "broken").unwrap();
        let host = crate::model::builtins().remove(2);
        assert!(update(&home, Path::new("a.exe"), &host, true).is_err());
        assert_eq!(fs::read_to_string(p).unwrap(), "broken");
        fs::remove_dir_all(home).unwrap();
    }
}
