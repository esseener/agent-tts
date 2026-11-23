use crate::model::{self, Config, Host};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigureRequest {
    pub host_id: String,
    pub name: Option<String>,
    pub profile_id: Option<String>,
    pub texts: Vec<(String, String)>,
}

pub fn parse_configure_args(args: &[String]) -> model::Result<ConfigureRequest> {
    if args.first().map(String::as_str) != Some("--configure-host") {
        return Err("内部参数错误：预期 --configure-host".into());
    }
    let host_id = args
        .get(1)
        .filter(|value| !value.starts_with("--"))
        .cloned()
        .ok_or("--configure-host 缺少宿主标识")?;
    let mut name = None;
    let mut profile_id = None;
    let mut texts = Vec::new();
    let mut index = 2;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .cloned();
        match flag {
            "--name" => {
                if name.is_some() {
                    return Err("--name 参数重复".into());
                }
                name = Some(value.ok_or("--name 缺少名称")?);
            }
            "--profile" => {
                if profile_id.is_some() {
                    return Err("--profile 参数重复".into());
                }
                profile_id = Some(value.ok_or("--profile 缺少声音标识")?);
            }
            "--text" => {
                let event = value.ok_or("--text 缺少状态")?;
                if !model::EVENTS.iter().any(|(key, _)| *key == event) {
                    return Err(format!("--text 状态无效：{event}"));
                }
                let text = args.get(index + 2).cloned().ok_or("--text 缺少台词")?;
                if text.trim().is_empty() || text.len() > 8192 {
                    return Err("--text 台词应为 1～8192 字节".into());
                }
                // Repeatable per event: each occurrence adds one line to the pool.
                texts.push((event, text));
                index += 1;
            }
            other => return Err(format!("未知参数：{other}")),
        }
        index += 2;
    }
    Ok(ConfigureRequest {
        host_id,
        name,
        profile_id,
        texts,
    })
}
fn validate_host_name(name: &str) -> model::Result<()> {
    if name.trim().is_empty() || name.chars().count() > 100 {
        return Err("宿主名称应为 1～100 个字符".into());
    }
    Ok(())
}

/// Seed announcement lines into route text fields that are still empty or that
/// we previously seeded; user-authored lines are never overwritten. A project
/// rename re-seeds its pool since the text is still ours.
fn seed_route_texts(host: &mut Host, texts: &[(String, String)]) -> bool {
    let mut seeded = false;
    for (event, _) in model::EVENTS {
        let lines: Vec<&str> = texts
            .iter()
            .filter(|(key, _)| *key == event)
            .map(|(_, text)| text.as_str())
            .collect();
        if lines.is_empty() {
            continue;
        }
        let Some(route) = host.routes.get_mut(event) else {
            continue;
        };
        let next = lines.join("\n");
        if (route.text.trim().is_empty() || route.seeded) && route.text != next {
            route.text = next;
            route.seeded = true;
            seeded = true;
        }
    }
    seeded
}

fn selected_profile<'a>(config: &'a Config, requested: Option<&str>) -> model::Result<&'a model::Profile> {
    if let Some(id) = requested {
        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .ok_or_else(|| format!("找不到声音：{id}"))?;
        if !profile.enabled {
            return Err(format!("声音已禁用：{id}"));
        }
        return Ok(profile);
    }
    config
        .profiles
        .iter()
        .find(|profile| profile.id == "default" && profile.enabled)
        .or_else(|| config.profiles.iter().find(|profile| profile.enabled))
        .ok_or_else(|| "没有启用的声音，无法配置宿主".into())
}

fn route_profiles(host: &Host) -> Value {
    let routes = model::EVENTS
        .iter()
        .filter_map(|(event, _)| {
            host.routes
                .get(*event)
                .map(|route| ((*event).to_string(), Value::String(route.profile.clone())))
        })
        .collect::<serde_json::Map<_, _>>();
    Value::Object(routes)
}

pub fn configure_host(config: &mut Config, request: ConfigureRequest) -> model::Result<Value> {
    if !model::valid_id(&request.host_id) {
        return Err("宿主标识无效".into());
    }
    let texts = request.texts.clone();
    if let Some(name) = request.name.as_deref() {
        validate_host_name(name)?;
    }

    if let Some(index) = config.hosts.iter().position(|host| host.id == request.host_id) {
        if config.hosts[index].adapter != "custom" {
            return Err(format!("只能配置自定义宿主：{}", request.host_id));
        }
        // Sync-supplied names and texts re-seed on rename; user-edited ones stay.
        let mut renamed = false;
        if let Some(name) = request.name.as_deref() {
            if config.hosts[index].name != name && config.hosts[index].name_seeded {
                config.hosts[index].name = name.to_string();
                renamed = true;
            }
        }
        let host = &config.hosts[index];
        let profile_id = if let Some(id) = request.profile_id.as_deref() {
            Some(selected_profile(config, Some(id))?.id.clone())
        } else {
            None
        };
        if let Some(profile_id) = profile_id.as_deref() {
            for (event, _) in model::EVENTS {
                if !host.routes.contains_key(event) {
                    return Err(format!("宿主缺少状态路由：{event}"));
                }
            }
            let changed = model::EVENTS
                .iter()
                .any(|(event, _)| host.routes[*event].profile != profile_id);
            if changed {
                let host = &mut config.hosts[index];
                for (event, _) in model::EVENTS {
                    host.routes.get_mut(event).expect("route was checked above").profile =
                        profile_id.to_string();
                }
            }
            let seeded = seed_route_texts(&mut config.hosts[index], &texts);
            let host = &config.hosts[index];
            return Ok(json!({
                "host": host.id,
                "name": host.name,
                "created": false,
                "changed": changed || renamed,
                "seeded": seeded,
                "routes": route_profiles(host),
            }));
        }
        let seeded = seed_route_texts(&mut config.hosts[index], &texts);
        let host = &config.hosts[index];
        return Ok(json!({
            "host": host.id,
            "name": host.name,
            "created": false,
            "changed": renamed,
            "seeded": seeded,
            "routes": route_profiles(host),
        }));
    }

    let profile = selected_profile(config, request.profile_id.as_deref())?;
    let profile_id = profile.id.clone();
    let name = request.name.unwrap_or_else(|| request.host_id.clone());
    validate_host_name(&name)?;
    let mut host = Host {
        id: request.host_id,
        name,
        name_seeded: true,
        ..Host::default()
    };
    for (event, _) in model::EVENTS {
        let route = host.routes.get_mut(event).ok_or_else(|| format!("默认宿主缺少状态路由：{event}"))?;
        route.profile = profile_id.clone();
    }
    let seeded = seed_route_texts(&mut host, &texts);
    let result = json!({
        "host": host.id,
        "name": host.name,
        "created": true,
        "changed": true,
        "seeded": seeded,
        "routes": route_profiles(&host),
    });
    config.hosts.push(host);
    Ok(result)
}

#[cfg(test)]
pub fn configure_from_args(config: &mut Config, args: &[String]) -> model::Result<Value> {
    let request = parse_configure_args(args)?;
    configure_host(config, request)
}

pub fn profiles_json(config: &Config) -> Value {
    json!({
        "profiles": config.profiles.iter().map(|profile| json!({
            "id": profile.id,
            "name": profile.name,
            "voice": profile.voice,
            "enabled": profile.enabled,
        })).collect::<Vec<_>>()
    })
}

/// Snapshot of every host's routing table for tools like the Orca settings panel.
pub fn hosts_json(config: &Config) -> Value {
    json!({
        "hosts": config.hosts.iter().map(|host| json!({
            "id": host.id,
            "name": host.name,
            "adapter": host.adapter,
            "enabled": host.enabled,
            "routes": host.routes.iter().map(|(event, route)| (event.clone(), json!({
                "enabled": route.enabled,
                "profile": route.profile,
                "text": route.text,
                "audio": route.audio,
            }))).collect::<std::collections::BTreeMap<_, _>>(),
        })).collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn snapshot(config: &Config) -> Value {
        serde_json::to_value(config).unwrap()
    }

    #[test]
    fn sync_without_profile_preserves_manual_routes_mute_and_name() {
        let mut config = Config::default();
        let mut manual = model::Profile::default();
        manual.id = "manual".into();
        config.profiles.push(manual);
        let mut host = Host {
            id: "orca_project".into(),
            name: "Hand configured".into(),
            ..Host::default()
        };
        let done = host.routes.get_mut("done").unwrap();
        done.profile = "manual".into();
        done.enabled = false;
        done.text = "custom line".into();
        done.audio = "custom.wav".into();
        config.hosts.push(host);
        let before = snapshot(&config);

        for _ in 0..2 {
            let result = configure_from_args(
                &mut config,
                &args(&["--configure-host", "orca_project", "--name", "Synced name"]),
            )
            .unwrap();
            assert_eq!(result["created"], false);
            assert_eq!(result["changed"], false);
            assert_eq!(result["name"], "Hand configured");
            assert_eq!(snapshot(&config), before);
        }
    }

    #[test]
    fn explicit_binding_changes_only_requested_custom_host_profiles() {
        let mut config = Config::default();
        let mut alternate = model::Profile::default();
        alternate.id = "alternate".into();
        alternate.name = "Alternate".into();
        config.profiles.push(alternate);

        let mut target = Host {
            id: "orca_target".into(),
            name: "Existing name".into(),
            ..Host::default()
        };
        let done = target.routes.get_mut("done").unwrap();
        done.enabled = false;
        done.text = "Keep this text".into();
        done.audio = "keep.wav".into();
        target.routes.insert(
            "custom_event".into(),
            model::Route {
                profile: "default".into(),
                enabled: false,
                text: "extra".into(),
                seeded: false,
                audio: "extra.wav".into(),
            },
        );
        let other = Host {
            id: "orca_other".into(),
            ..Host::default()
        };
        let other_before = serde_json::to_value(&other).unwrap();
        config.hosts.extend([target, other]);

        let result = configure_from_args(
            &mut config,
            &args(&[
                "--configure-host",
                "orca_target",
                "--name",
                "Ignored name",
                "--profile",
                "alternate",
            ]),
        )
        .unwrap();
        assert_eq!(result["created"], false);
        assert_eq!(result["changed"], true);
        assert_eq!(result["name"], "Existing name");
        let target = config.hosts.iter().find(|host| host.id == "orca_target").unwrap();
        for (event, _) in model::EVENTS {
            assert_eq!(target.routes[event].profile, "alternate");
        }
        assert!(!target.routes["done"].enabled);
        assert_eq!(target.routes["done"].text, "Keep this text");
        assert_eq!(target.routes["done"].audio, "keep.wav");
        assert_eq!(target.routes["custom_event"].profile, "default");
        let other = config.hosts.iter().find(|host| host.id == "orca_other").unwrap();
        assert_eq!(serde_json::to_value(other).unwrap(), other_before);

        let second = configure_from_args(
            &mut config,
            &args(&["--configure-host", "orca_target", "--profile", "alternate"]),
        )
        .unwrap();
        assert_eq!(second["changed"], false);
    }

    #[test]
    fn invalid_arguments_leave_pure_config_unchanged() {
        let mut config = Config::default();
        let before = snapshot(&config);
        let error = configure_from_args(
            &mut config,
            &args(&[
                "--configure-host",
                "orca_new",
                "--profile",
                "default",
                "--profile",
                "default",
            ]),
        )
        .unwrap_err();
        assert!(error.contains("重复"));
        assert_eq!(snapshot(&config), before);

        let error = configure_from_args(
            &mut config,
            &args(&["--configure-host", "bad/id", "--profile", "default"]),
        )
        .unwrap_err();
        assert!(error.contains("无效"));
        assert_eq!(snapshot(&config), before);
    }

    #[test]
    fn text_seeds_only_empty_routes_and_never_overrides_user_lines() {
        let mut config = Config::default();
        let result = configure_from_args(
            &mut config,
            &args(&[
                "--configure-host",
                "orca_new",
                "--name",
                "Orca · 示例",
                "--text",
                "done",
                "示例项目收工了",
                "--text",
                "error",
                "示例项目出错了",
            ]),
        )
        .unwrap();
        assert_eq!(result["created"], true);
        assert_eq!(result["seeded"], true);
        let host = config.hosts.iter().find(|host| host.id == "orca_new").unwrap();
        assert_eq!(host.routes["done"].text, "示例项目收工了");
        assert_eq!(host.routes["error"].text, "示例项目出错了");
        assert!(host.routes["ask"].text.is_empty());

        // The seeded done line is marked as a seed; a user-edited ask line is not.
        let host = config.hosts.iter_mut().find(|host| host.id == "orca_new").unwrap();
        host.routes.get_mut("done").unwrap().seeded = false;

        // Re-seeding refreshes empty slots and leaves user lines untouched.
        let result = configure_from_args(
            &mut config,
            &args(&[
                "--configure-host",
                "orca_new",
                "--name",
                "Orca · 改名",
                "--text",
                "done",
                "别的台词",
                "--text",
                "ask",
                "等你处理",
            ]),
        )
        .unwrap();
        assert_eq!(result["created"], false);
        assert_eq!(result["seeded"], true);
        let host = config.hosts.iter().find(|host| host.id == "orca_new").unwrap();
        assert_eq!(host.routes["done"].text, "示例项目收工了");
        assert_eq!(host.routes["ask"].text, "等你处理");

        assert!(configure_from_args(
            &mut config,
            &args(&["--configure-host", "orca_new", "--text", "bogus", "台词"]),
        )
        .unwrap_err()
        .contains("状态无效"));
    }

    #[test]
    fn repeated_text_joins_into_random_pool() {
        let mut config = Config::default();
        let result = configure_from_args(
            &mut config,
            &args(&[
                "--configure-host",
                "orca_pool",
                "--name",
                "Orca · 池",
                "--text",
                "done",
                "第一句",
                "--text",
                "done",
                "第二句",
                "--text",
                "ask",
                "等你",
            ]),
        )
        .unwrap();
        assert_eq!(result["seeded"], true);
        let host = config.hosts.iter().find(|host| host.id == "orca_pool").unwrap();
        assert_eq!(host.routes["done"].text, "第一句\n第二句");
        assert_eq!(host.routes["ask"].text, "等你");
        // Playback resolves one non-empty line at random.
        for _ in 0..4 {
            let playback = model::resolve(
                &config,
                &model::Event {
                    host: "orca_pool".into(),
                    event: "done".into(),
                    ..model::Event::default()
                },
            )
            .unwrap()
            .unwrap();
            assert!(["第一句", "第二句"].contains(&playback.text.as_str()));
        }
    }

    #[test]
    fn rename_reseeds_only_seeded_lines_and_name() {
        let mut config = Config::default();
        let args_for = |name: &str, done: &str| {
            args(&[
                "--configure-host", "orca_proj", "--name", name,
                "--text", "done", done, "--text", "ask", "等你",
            ])
        };
        configure_from_args(&mut config, &args_for("Orca · 旧名", "旧名，搞定了")).unwrap();
        // User edits the ask line and the display name; both stop being seeds.
        let host = config.hosts.iter_mut().find(|h| h.id == "orca_proj").unwrap();
        host.routes.get_mut("ask").unwrap().text = "手写台词".into();
        host.routes.get_mut("ask").unwrap().seeded = false;
        host.name = "我起的名".into();
        host.name_seeded = false;

        let result = configure_from_args(&mut config, &args_for("Orca · 新名", "新名，搞定了")).unwrap();
        assert_eq!(result["seeded"], true);
        let host = config.hosts.iter().find(|h| h.id == "orca_proj").unwrap();
        // Manual name and line survive; the seeded done pool re-seeded.
        assert_eq!(host.name, "我起的名");
        assert_eq!(host.routes["ask"].text, "手写台词");
        assert!(host.routes["done"].text.contains("新名，搞定了"));
        assert!(host.routes["done"].seeded);
    }
}
