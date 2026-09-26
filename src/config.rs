use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::{fs, io};

/// 顶层配置：`~/.config/iso-cc/config.toml`（全局）+ 可选项目级 `.iso-cc.toml` 覆盖。
///
/// 纪律（ADR 0008 / pm-spec 教训）：版本号与结构由本文件唯一裁决；
/// 未知键 = 配置错误（deny_unknown_fields，fail-loud，绝不静默忽略）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u8,
    #[serde(default)]
    pub profile: BTreeMap<String, Profile>,
}

/// 会话声明单元（CONTEXT.md 词汇）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// 出口引用，v1 契约仅 `if:<接口名>`（D4）。
    pub egress: String,
    #[serde(default)]
    pub locale: Locale,
    #[serde(default)]
    pub net: Net,
    #[serde(default)]
    pub agent: Agent,
    /// 额外声明式路径重定向（R4 机制扩展），格式 `src=dst`。
    #[serde(default)]
    pub redirect: Vec<String>,
    /// 额外注入 env（如 HISTFILE），opt-in。
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Locale {
    pub tz: Option<String>,
    pub lang: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Net {
    /// `tree`（默认，整树进 netns）| `self`（cc 本体 netns + bash RPC 转发宿主）。
    #[serde(default)]
    pub scope: Option<NetScope>,
    /// `off`（默认，fail-closed）| `tunnel`。
    #[serde(default)]
    pub ipv6: Option<NetIpv6>,
    /// netns 内以 127.0.0.1 预占并转发宿主 loopback 的端口（R3 localhost 分层）。
    #[serde(default)]
    pub localhost_forward: Vec<u16>,
    /// RFC1918 目的地路由策略：`tunnel`（默认）| `host`（宿主 LAN 直连，显式 opt-in）。
    #[serde(default)]
    pub private: Option<NetPrivate>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetScope {
    Tree,
    #[serde(rename = "self")]
    Self_,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetIpv6 {
    Off,
    Tunnel,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetPrivate {
    Tunnel,
    Host,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub command: Option<String>,
    /// advisory：不匹配仅警告（D3：宿主管理 + 声明式定义）。
    pub version: Option<String>,
}

impl Profile {
    /// 解析 egress 引用。v1 仅 `if:<name>`（D4：隧道侧自出 TUN）。
    pub fn egress_iface(&self) -> Result<&str, ConfigError> {
        self.egress
            .strip_prefix("if:")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ConfigError::EgressShape(self.egress.clone()))
    }

    /// 生效 scope（未声明 = tree）。
    pub fn scope(&self) -> NetScope {
        self.net.scope.unwrap_or(NetScope::Tree)
    }

    /// 生效 IPv6 策略（未声明 = off，fail-closed）。
    pub fn ipv6(&self) -> NetIpv6 {
        self.net.ipv6.unwrap_or(NetIpv6::Off)
    }

    /// 结构校验：返回错误清单（空 = 通过）。宿主事实检查归 doctor，不在这里。
    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        if let Err(e) = self.egress_iface() {
            errs.push(e.to_string());
        }
        for r in &self.redirect {
            if !r.contains('=') || r.split('=').count() != 2 {
                errs.push(format!("redirect 必须是 `src=dst` 形态：{r:?}"));
            }
        }
        errs
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("配置 {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("配置解析失败 {path}:\n{source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("config.version 必须为 1，当前为 {0}")]
    Version(u8),
    #[error("egress 契约仅支持 `if:<接口名>`，收到 {0:?}")]
    EgressShape(String),
    #[error("未找到任何配置（查过 {paths}）；需要至少一个 profile")]
    NoConfig { paths: String },
    #[error("profile {name:?} 不存在；可用：{available}")]
    NoSuchProfile { name: String, available: String },
    #[error("存在多个 profile（{available}），须用 --profile 指定")]
    AmbiguousProfile { available: String },
    #[error("profile 校验失败：\n{0}")]
    Invalid(String),
}

/// 加载并合并：全局（可选缺失）← 项目覆盖（可选缺失，同名 profile 整体替换）。
/// 返回 (config, warnings)。
pub fn load(global: &Path, project: Option<&Path>) -> Result<(Config, Vec<String>), ConfigError> {
    let mut warnings = Vec::new();
    let mut cfg = match read_one(global)? {
        Some(c) => c,
        None => Config {
            version: 1,
            profile: BTreeMap::new(),
        },
    };
    if let Some(p) = project {
        if let Some(overlay) = read_one(p)? {
            for (name, prof) in overlay.profile {
                if cfg.profile.insert(name.clone(), prof).is_some() {
                    warnings.push(format!("profile {name:?} 被项目配置覆盖"));
                } else {
                    warnings.push(format!("profile {name:?} 来自项目配置"));
                }
            }
        }
    }
    if cfg.version != 1 {
        return Err(ConfigError::Version(cfg.version));
    }
    Ok((cfg, warnings))
}

/// 解析目标 profile：显式指定必须存在；未指定时唯一 profile 自动当选。
pub fn resolve_profile<'a>(
    cfg: &'a Config,
    requested: Option<&'a str>,
) -> Result<(&'a str, &'a Profile), ConfigError> {
    let available = cfg
        .profile
        .keys()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if cfg.profile.is_empty() {
        return Err(ConfigError::NoConfig { paths: available });
    }
    match requested {
        Some(name) => cfg
            .profile
            .get(name)
            .map(|p| (name, p))
            .ok_or(ConfigError::NoSuchProfile {
                name: name.to_string(),
                available,
            }),
        None => {
            if cfg.profile.len() == 1 {
                Ok(cfg
                    .profile
                    .iter()
                    .next()
                    .map(|(k, v)| (k.as_str(), v))
                    .unwrap())
            } else {
                Err(ConfigError::AmbiguousProfile { available })
            }
        }
    }
}

/// 要求 profile 通过结构校验（fail-loud）。
pub fn require_valid(profile: &Profile) -> Result<(), ConfigError> {
    let errs = profile.validate();
    if errs.is_empty() {
        Ok(())
    } else {
        Err(ConfigError::Invalid(errs.join("\n")))
    }
}

fn read_one(path: &Path) -> Result<Option<Config>, ConfigError> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(ConfigError::Io {
                path: path.to_path_buf(),
                source: e,
            })
        }
    };
    toml::from_str(&text)
        .map(Some)
        .map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
}

pub fn global_path() -> PathBuf {
    let xdg = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    xdg.unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/root".into())))
        .join(".config/iso-cc/config.toml")
}

pub fn project_path() -> PathBuf {
    PathBuf::from(".iso-cc.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
version = 1
[profile.sg]
egress = "if:wg0"
locale.tz = "Asia/Singapore"
locale.lang = "en_SG.UTF-8"
net.scope = "self"
net.ipv6 = "off"
net.localhost_forward = [5432]
net.private = "tunnel"
agent.command = "claude"
"#;

    #[test]
    fn parse_full_profile() {
        let cfg: Config = toml::from_str(FULL).unwrap();
        let p = cfg.profile.get("sg").unwrap();
        assert_eq!(p.egress_iface().unwrap(), "wg0");
        assert_eq!(p.locale.tz.as_deref(), Some("Asia/Singapore"));
        assert_eq!(p.scope(), NetScope::Self_);
        assert_eq!(p.ipv6(), NetIpv6::Off);
        assert_eq!(p.net.localhost_forward, vec![5432u16]);
    }

    #[test]
    fn defaults_are_fail_closed() {
        let cfg: Config = toml::from_str("version = 1\n[profile.x]\negress = 'if:wg0'").unwrap();
        let p = cfg.profile.get("x").unwrap();
        assert_eq!(p.scope(), NetScope::Tree);
        assert_eq!(p.ipv6(), NetIpv6::Off);
        assert!(p.net.localhost_forward.is_empty());
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = toml::from_str::<Config>("version = 1\n[profile.x]\negresss = 'if:wg0'")
            .expect_err("typo must fail");
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn wrong_egress_shape_rejected() {
        let cfg: Config = toml::from_str("version = 1\n[profile.x]\negress = 'socks:1'").unwrap();
        let errs = cfg.profile.get("x").unwrap().validate();
        assert!(errs.iter().any(|e| e.contains("if:")), "{errs:?}");
    }

    #[test]
    fn bad_redirect_shape_rejected() {
        let cfg: Config =
            toml::from_str("version = 1\n[profile.x]\negress='if:w'\nredirect = ['/a /b']")
                .unwrap();
        assert!(!cfg.profile.get("x").unwrap().validate().is_empty());
    }

    #[test]
    fn project_overlay_replaces_profile_and_warns() {
        let dir = tempfile::tempdir().unwrap();
        let g = dir.path().join("config.toml");
        let p = dir.path().join(".iso-cc.toml");
        std::fs::write(&g, "version = 1\n[profile.a]\negress = 'if:w0'").unwrap();
        std::fs::write(&p, "version = 1\n[profile.a]\negress = 'if:w1'").unwrap();
        let (cfg, warnings) = load(&g, Some(&p)).unwrap();
        assert_eq!(cfg.profile["a"].egress, "if:w1");
        assert!(warnings.iter().any(|w| w.contains("覆盖")));
    }

    #[test]
    fn resolve_profile_semantics() {
        let cfg: Config = toml::from_str(FULL).unwrap();
        assert!(resolve_profile(&cfg, Some("nope")).is_err());
        let (name, _) = resolve_profile(&cfg, None).unwrap();
        assert_eq!(name, "sg");
    }
}
