use serde::Deserialize;
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::{fs, io};

/// 网关 DNS 转发地址默认值（pasta `--dns-forward` 与 slirp4netns 内建转发器同址）。
pub const DEFAULT_DNS: Ipv4Addr = Ipv4Addr::new(10, 0, 2, 3);

/// mark 引擎固定面（票 18 阶段 A proven 形态）：uid 4210（编译期钉进助手）、
/// 策略表 5182、规则 pref 15000。固定 = 单一 proven 常量面，doctor/plan/setup/
/// session/gc 共用；参数化 = 后续票声明化，不在本票扩轴。
pub const MARK_UID: u32 = 4210;
pub const MARK_TABLE: u32 = 5182;
pub const MARK_RULE_PREF: u32 = 15000;

/// 网关形态（设计稿 §1）：`pasta` spawn 模式会话根（默认）| `slirp4netns` 回退（attach + selfmap）。
/// 未知取值 = 配置错误（serde unknown variant，fail-loud #1）。
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetGateway {
    Pasta,
    Slirp4netns,
}

impl NetGateway {
    pub fn bin_name(self) -> &'static str {
        match self {
            NetGateway::Pasta => "pasta",
            NetGateway::Slirp4netns => "slirp4netns",
        }
    }
}

impl std::fmt::Display for NetGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.bin_name())
    }
}

/// 会话引擎（spec 变更（五）双候选并存，票 18）：
/// - `netns`（默认）：pasta spawn + tun2socks；结构性 fail-closed、端口空间隔离；
/// - `mark`：uid 策略路由（零 netns，localhost 双向零摩擦）；原语持久面由
///   清单 + doctor + gc 管理（setup-manifested，spec 变更（一）两级模型）。
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Netns,
    Mark,
}

impl std::fmt::Display for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Engine::Netns => write!(f, "netns"),
            Engine::Mark => write!(f, "mark"),
        }
    }
}

/// egress 引用（D4 修订两形态，工单 16）：
/// - `if:<接口名>`（默认形态）：pasta 直接以该接口为 outbound，出口 = 宿主出口；
/// - `socks5://<host>:<port>`：会话根仍是 pasta（outbound = 宿主默认路由接口），
///   netns 内由 tun2proxy worker（tun1）把全部出口流量送经宿主 SOCKS5
///   （mihomo mixed-port 类）。择型证据：/tmp/iso-cc-exp16/REPORT.md（阶段 A 实测
///   组合成立，embedded netstack 降级为远期）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Egress {
    If(String),
    Socks5 { host: String, port: u16 },
}

/// 解析 `socks5://<host>:<port>`（fail-loud：userinfo/缺端口/空 host/方括号外
/// 的裸 v6 一律拒绝；`[v6]:port` 括号形态支持）。返回 (host, port)。
pub fn parse_socks5_url(url: &str) -> Result<(String, u16), ConfigError> {
    let rest = url
        .strip_prefix("socks5://")
        .ok_or_else(|| ConfigError::EgressShape(url.to_string()))?;
    if rest.is_empty() {
        return Err(ConfigError::EgressShape(url.to_string()));
    }
    // IPv6 字面量括号形态：socks5://[::1]:1080
    if let Some(v6) = rest.strip_prefix('[') {
        let (host, port) = v6
            .split_once("]:")
            .ok_or_else(|| ConfigError::EgressShape(url.to_string()))?;
        let port: u16 = port
            .parse()
            .map_err(|_| ConfigError::EgressShape(url.to_string()))?;
        if host.is_empty() {
            return Err(ConfigError::EgressShape(url.to_string()));
        }
        return Ok((host.to_string(), port));
    }
    if rest.contains('[') || rest.contains(']') {
        return Err(ConfigError::EgressShape(url.to_string()));
    }
    // userinfo（user:pass@host:port）v1 不支持：出现 '@' 即拒绝（fail-loud，绝不静默丢弃凭据）。
    if rest.contains('@') {
        return Err(ConfigError::EgressShape(url.to_string()));
    }
    let (host, port) = rest
        .rsplit_once(':')
        .ok_or_else(|| ConfigError::EgressShape(url.to_string()))?;
    if host.is_empty() || host.contains(':') {
        // 裸 v6（多个 ':' 无括号）无法与 host:port 二义区分 → 拒绝，要求括号形态
        return Err(ConfigError::EgressShape(url.to_string()));
    }
    let port: u16 = port
        .parse()
        .map_err(|_| ConfigError::EgressShape(url.to_string()))?;
    Ok((host.to_string(), port))
}

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
    /// 出口引用（D4 修订两形态）：`if:<接口名>`（默认）| `socks5://<host>:<port>`。
    pub egress: String,
    #[serde(default)]
    pub locale: Locale,
    #[serde(default)]
    pub net: Net,
    /// bash 执行位置轴（票 15 / spec 变更（四））：与 net.scope 正交的独立声明面。
    #[serde(default)]
    pub exec: Exec,
    #[serde(default)]
    pub agent: Agent,
    /// 额外声明式路径重定向（R4 机制扩展），格式 `src=dst`。
    #[serde(default)]
    pub redirect: Vec<String>,
    /// 额外注入 env（如 HISTFILE），opt-in。
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// `exec.bash` 声明（票 15）：`sandbox`（默认，bash 工具会话内执行，行为与现状等价）
/// | `host`（经 exec.sock RPC 通道宿主侧执行，US9 真 localhost；身份一致性让位 =
/// 显式声明的代价，spec 变更（四））。
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExecBash {
    Sandbox,
    Host,
}

/// `[exec]` 声明节（票 15 两轴解耦的第二轴）。未知键 = 配置错误（deny_unknown_fields）。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exec {
    pub bash: Option<ExecBash>,
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
    /// `netns`（默认）| `mark`（票 18）：会话引擎轴（身份/隔离机制择型）。
    #[serde(default)]
    pub engine: Option<Engine>,
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
    /// 网关形态：`pasta`（默认，spawn 模式会话根）| `slirp4netns`（回退，attach + selfmap）。
    #[serde(default)]
    pub gateway: Option<NetGateway>,
    /// 网关 DNS 转发地址（pasta `--dns-forward`；未声明 = 10.0.2.3）。
    #[serde(default)]
    pub dns: Option<Ipv4Addr>,
    /// MCP loopback 快照缺口兜底轴（票 20）：true（默认）= bootstrap 期对声明
    /// http/sse loopback MCP 端口探活，探活失败（pasta 镜像 = attach 时刻快照）
    /// 起 socat 转发器补齐；false = 无兜底（缺口由 P-MCP 探针红显）。
    #[serde(default)]
    pub mcp_fallback: Option<bool>,
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
    /// CC 配置隔离轴（票 05 / R4 / D6）：true（默认）= 会话自动前置内置重定向对
    /// （`~/.claude/`、`~/.claude.json(.backup)` → `<state>/profiles/<profile>/`，
    /// rw bind + 会话内 unset CLAUDE_CONFIG_DIR）；false = 无内置对，与宿主共享
    /// cc 默认路径（显式声明的共享语义，行为与票 05 之前等价）。
    #[serde(default)]
    pub cc_isolation: Option<bool>,
}

/// CC 内置重定向对（票 05 / R4 / D6 声明集）：会话内 view 路径（cc 默认路径，
/// 完全无感）由 backing 路径（profile 持久态，登录态/会话史跨会话存续）rw bind
/// 支撑；clean-room 默认不继承宿主 `~/.claude`。`key` = manifest profile-state
/// 登记名（gc 仅随显式 `--profile` 回收）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CcBuiltinPair {
    pub view: PathBuf,
    pub backing: PathBuf,
    /// backing 形态（setup mkdir/touch 依据；声明即事实，不探测宿主）。
    pub backing_is_dir: bool,
    pub key: &'static str,
}

/// 内置对全集（声明集唯一出处：session 前置 bind / setup 收敛 / doctor 核验 /
/// plan 展开共用；新增路径 = 补此处 + 测试夹具，ADR 0006 泄漏捕获路径）。
pub fn cc_builtin_pairs(profile_name: &str) -> Vec<CcBuiltinPair> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let home = Path::new(&home);
    let base = crate::manifest::state_dir()
        .join("profiles")
        .join(profile_name);
    vec![
        CcBuiltinPair {
            view: home.join(".claude"),
            backing: base.join("claude"),
            backing_is_dir: true,
            key: "claude",
        },
        CcBuiltinPair {
            view: home.join(".claude.json"),
            backing: base.join("claude.json"),
            backing_is_dir: false,
            key: "claude.json",
        },
        CcBuiltinPair {
            view: home.join(".claude.json.backup"),
            backing: base.join("claude.json.backup"),
            backing_is_dir: false,
            key: "claude.json.backup",
        },
    ]
}

// ===== MCP loopback 声明扫描（票 20）=====
//
// 扫描源 = cc 视线的 claude.json（netns 内即 ADR 0006 重定向后路径：
// cc_isolation=true 时 `$HOME/.claude.json` 就是 profile backing 的 rw bind view）
// + 项目 `.mcp.json`。解析只取 (scheme, host, port) 三元组（exp19 §1：token 常见
// 于 path/query/header ⇒ 其余一律不解析不落盘）：scheme ∈ {http, https, sse} 且
// host ∈ {127.0.0.1, localhost, ::1}；port 缺省按 scheme（http/sse=80、https=443）。
// stdio/command 型条目无 url，天然不命中。文件缺失/不可读/JSON 畸形/条目畸形 =
// 无声明（0 条），不报错——「声明缺席」与「声明损坏」同归无条目，探针层 SKIP。

/// 声明端口全集（排序去重）。消费方：bootstrap 缺口兜底（session）、P-MCP 探针
///（probe）、doctor 声明端口汇总。
pub fn mcp_loopback_ports(claude_json: &Path, project_mcp_json: Option<&Path>) -> Vec<u16> {
    let mut ports = Vec::new();
    for path in std::iter::once(claude_json).chain(project_mcp_json) {
        let Ok(raw) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        mcp_collect(&v, &mut ports);
    }
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// 根对象收面：root `mcpServers` + `projects.<path>.mcpServers`（claude.json 两层
/// 形态；`.mcp.json` 只有前者，同函数无害共用）。
fn mcp_collect(root: &serde_json::Value, out: &mut Vec<u16>) {
    if let Some(servers) = root["mcpServers"].as_object() {
        for entry in servers.values() {
            if let Some(port) = entry["url"].as_str().and_then(url_loopback_port) {
                out.push(port);
            }
        }
    }
    if let Some(projects) = root["projects"].as_object() {
        for proj in projects.values() {
            if let Some(servers) = proj["mcpServers"].as_object() {
                for entry in servers.values() {
                    if let Some(port) = entry["url"].as_str().and_then(url_loopback_port) {
                        out.push(port);
                    }
                }
            }
        }
    }
}

/// URL → loopback 端口判定核（纯函数，票 20 单测锚）。None = 非本机制范围
/// （非 http/https/sse、host 非 loopback、端口非法）。v6 仅认 `[::1]` 括号形态。
pub(crate) fn url_loopback_port(url: &str) -> Option<u16> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https" | "sse") {
        return None;
    }
    // authority = 首个 path/query/fragment 分隔符之前
    let authority = rest.split(['/', '?', '#']).next()?;
    // 去 userinfo（host 面不含裸 '@'，取最后一个 '@' 之后）
    let authority = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    let default_port: u16 = if scheme == "https" { 443 } else { 80 };
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse::<u16>().ok()?,
            None => default_port,
        };
        (h, port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().ok()?),
            None => (authority, default_port),
        }
    };
    if port == 0 {
        return None;
    }
    matches!(host.to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "::1").then_some(port)
}

impl Profile {
    /// 解析 egress 引用（D4 修订：`if:<name>` | `socks5://<host>:<port>` 两形态）。
    pub fn egress(&self) -> Result<Egress, ConfigError> {
        if self.egress.starts_with("socks5://") {
            let (host, port) = parse_socks5_url(&self.egress)?;
            return Ok(Egress::Socks5 { host, port });
        }
        self.egress_iface().map(|name| Egress::If(name.to_string()))
    }

    /// `if:` 形态专用访问器（socks 形态 = 配置错误：该调用点只接受接口引用）。
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

    /// 生效引擎（未声明 = netns，行为与票 18 之前等价）。
    pub fn engine(&self) -> Engine {
        self.net.engine.unwrap_or(Engine::Netns)
    }

    /// 生效 bash 执行位置（未声明 = sandbox，行为与现状等价）。
    pub fn exec_bash(&self) -> ExecBash {
        self.exec.bash.unwrap_or(ExecBash::Sandbox)
    }

    /// 生效 CC 配置隔离（未声明 = true，票 05 默认内置重定向对）。
    pub fn cc_isolation(&self) -> bool {
        self.agent.cc_isolation.unwrap_or(true)
    }

    /// 生效 IPv6 策略（未声明 = off，fail-closed）。
    pub fn ipv6(&self) -> NetIpv6 {
        self.net.ipv6.unwrap_or(NetIpv6::Off)
    }

    /// 生效网关形态（未声明 = pasta，spawn 会话根）。
    pub fn gateway(&self) -> NetGateway {
        self.net.gateway.unwrap_or(NetGateway::Pasta)
    }

    /// 生效 DNS 转发地址（未声明 = 10.0.2.3）。
    pub fn dns(&self) -> Ipv4Addr {
        self.net.dns.unwrap_or(DEFAULT_DNS)
    }

    /// 生效 MCP 快照缺口兜底（未声明 = true，票 20 默认开启）。
    pub fn mcp_fallback(&self) -> bool {
        self.net.mcp_fallback.unwrap_or(true)
    }

    /// 结构校验：返回错误清单（空 = 通过）。宿主事实检查归 doctor，不在这里。
    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        if let Err(e) = self.egress() {
            errs.push(e.to_string());
        }
        if matches!(self.egress(), Ok(Egress::Socks5 { .. }))
            && self.gateway() == NetGateway::Slirp4netns
        {
            errs.push(
                "socks5 egress 仅支持 net.gateway=pasta（slirp 回退组合未经工单 16 实测，fail-loud 拒绝落地）"
                    .to_string(),
            );
        }
        // mark 引擎声明性拒绝（票 18：provider/* 语义不变；netns 机制字段在
        // mark 下不适用 = fail-loud，绝不静默忽略）。
        if self.engine() == Engine::Mark {
            if matches!(self.egress(), Ok(Egress::Socks5 { .. })) {
                errs.push(
                    "mark 引擎不适用 socks5 egress（零 netns：无 tun2proxy 承载面）——将 mihomo TUN 化后用 egress=if:<tun 设备>（票 18）"
                        .to_string(),
                );
            }
            if !self.redirect.is_empty() {
                errs.push(
                    "mark 引擎零 mountns：redirect 挂载面不适用（bind 无处落地，声明性拒绝；票 18）"
                        .to_string(),
                );
            }
            if self.net.scope.is_some() {
                errs.push(
                    "net.scope 是 netns 身份轴；mark 引擎身份 = uid（整树同 uid），轴不适用（票 18）"
                        .to_string(),
                );
            }
            if self.net.ipv6.is_some() {
                errs.push(
                    "net.ipv6 是 netns 轴；mark 引擎 v6 恒 fail-closed（表内 unreachable），不可声明（票 18）"
                        .to_string(),
                );
            }
            if !self.net.localhost_forward.is_empty() {
                errs.push(
                    "net.localhost_forward 是 netns 转发面；mark 引擎 localhost 双向零摩擦（结构成立），轴不适用（票 18）"
                        .to_string(),
                );
            }
            if self.net.private.is_some() {
                errs.push(
                    "net.private 是 netns 路由策略轴；mark 引擎不适用（uidrange 表内无 RFC1918 策略面，票 18）"
                        .to_string(),
                );
            }
            if self.net.gateway.is_some() {
                errs.push(
                    "net.gateway 是 netns 网关形态轴；mark 引擎会话根 = file-cap 助手（无网关进程），轴不适用（票 18）"
                        .to_string(),
                );
            }
            if self.net.dns.is_some() {
                errs.push(
                    "net.dns 是 netns DNS 转发地址；mark 引擎 DNS 元数据走宿主解析器（票 18 已登记例外，resolv bind 无 mountns 承载），轴不适用"
                        .to_string(),
                );
            }
            if self.net.mcp_fallback.is_some() {
                errs.push(
                    "net.mcp_fallback 是 netns 快照缺口兜底轴（票 20）；mark 引擎零 netns（127.0.0.1 天然直达宿主），机制不适用（票 19 §6）"
                        .to_string(),
                );
            }
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
    #[error("egress 契约支持 `if:<接口名>` 或 `socks5://<host>:<port>`，收到 {0:?}")]
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
        assert_eq!(p.gateway(), NetGateway::Pasta);
        assert_eq!(p.dns(), DEFAULT_DNS);
        assert!(p.net.localhost_forward.is_empty());
    }

    #[test]
    fn gateway_and_dns_parse() {
        let cfg: Config = toml::from_str(
            "version = 1\n[profile.x]\negress = 'if:wg0'\nnet.gateway = 'slirp4netns'\nnet.dns = '1.1.1.1'",
        )
        .unwrap();
        let p = cfg.profile.get("x").unwrap();
        assert_eq!(p.gateway(), NetGateway::Slirp4netns);
        assert_eq!(p.gateway().bin_name(), "slirp4netns");
        assert_eq!(p.dns(), Ipv4Addr::new(1, 1, 1, 1));
    }

    #[test]
    fn unknown_gateway_value_rejected() {
        let err = toml::from_str::<Config>(
            "version = 1\n[profile.x]\negress = 'if:w'\nnet.gateway = 'vpn'",
        )
        .expect_err("未知 gateway 取值必须 fail-loud（#1）");
        assert!(err.to_string().contains("unknown variant"), "{err}");
    }

    #[test]
    fn exec_bash_axis_independent_of_net_scope() {
        // 两轴独立声明（spec 变更（四））：任一组合都必须可解析，互不牵连。
        for (scope, bash) in [
            ("tree", "sandbox"),
            ("tree", "host"),
            ("self", "host"),
            ("self", "sandbox"),
        ] {
            let cfg: Config = toml::from_str(&format!(
                "version = 1\n[profile.x]\negress = 'if:wg0'\nnet.scope = '{scope}'\nexec.bash = '{bash}'"
            ))
            .unwrap();
            let p = cfg.profile.get("x").unwrap();
            assert_eq!(
                p.exec_bash(),
                if bash == "host" {
                    ExecBash::Host
                } else {
                    ExecBash::Sandbox
                },
                "{scope}+{bash}"
            );
            assert_eq!(
                p.scope(),
                if scope == "self" {
                    NetScope::Self_
                } else {
                    NetScope::Tree
                },
                "{scope}+{bash}"
            );
        }
    }

    #[test]
    fn exec_bash_defaults_to_sandbox() {
        let cfg: Config = toml::from_str("version = 1\n[profile.x]\negress = 'if:wg0'").unwrap();
        assert_eq!(cfg.profile.get("x").unwrap().exec_bash(), ExecBash::Sandbox);
    }

    #[test]
    fn unknown_exec_bash_value_rejected() {
        let err = toml::from_str::<Config>(
            "version = 1\n[profile.x]\negress = 'if:wg0'\nexec.bash = 'remote'",
        )
        .expect_err("未知 exec.bash 取值必须 fail-loud（#1）");
        assert!(err.to_string().contains("unknown variant"), "{err}");
    }

    #[test]
    fn unknown_exec_field_rejected() {
        let err = toml::from_str::<Config>(
            "version = 1\n[profile.x]\negress = 'if:wg0'\nexec.mode = 'host'",
        )
        .expect_err("exec 节未知键必须 fail-loud");
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn cc_isolation_defaults_true() {
        // 票 05：默认内置重定向对——未声明 = true
        let cfg: Config = toml::from_str("version = 1\n[profile.x]\negress = 'if:wg0'").unwrap();
        assert!(cfg.profile.get("x").unwrap().cc_isolation());
    }

    #[test]
    fn cc_isolation_false_axis_parses() {
        let cfg: Config = toml::from_str(
            "version = 1\n[profile.x]\negress = 'if:wg0'\nagent.cc_isolation = false",
        )
        .unwrap();
        assert!(!cfg.profile.get("x").unwrap().cc_isolation());
        let cfg: Config = toml::from_str(
            "version = 1\n[profile.x]\negress = 'if:wg0'\nagent.cc_isolation = true",
        )
        .unwrap();
        assert!(cfg.profile.get("x").unwrap().cc_isolation());
    }

    #[test]
    fn unknown_agent_field_rejected() {
        let err = toml::from_str::<Config>(
            "version = 1\n[profile.x]\negress = 'if:wg0'\nagent.pinned = true",
        )
        .expect_err("agent 节未知键必须 fail-loud");
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn cc_builtin_pairs_shape() {
        // 声明集唯一出处的形态锚：三对 view/backing/key/backing_is_dir
        let pairs = cc_builtin_pairs("ccx");
        assert_eq!(pairs.len(), 3);
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        let base = crate::manifest::state_dir().join("profiles").join("ccx");
        assert_eq!(
            pairs[0].view,
            std::path::PathBuf::from(&home).join(".claude")
        );
        assert_eq!(pairs[0].backing, base.join("claude"));
        assert!(pairs[0].backing_is_dir);
        assert_eq!(pairs[0].key, "claude");
        assert_eq!(
            pairs[1].view,
            std::path::PathBuf::from(&home).join(".claude.json")
        );
        assert_eq!(pairs[1].backing, base.join("claude.json"));
        assert!(!pairs[1].backing_is_dir);
        assert_eq!(
            pairs[2].view,
            std::path::PathBuf::from(&home).join(".claude.json.backup")
        );
        assert_eq!(pairs[2].backing, base.join("claude.json.backup"));
        assert!(!pairs[2].backing_is_dir);
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
    fn socks5_egress_parses_host_and_port() {
        let cfg: Config =
            toml::from_str("version = 1\n[profile.x]\negress = 'socks5://127.0.0.1:7891'").unwrap();
        let p = cfg.profile.get("x").unwrap();
        assert!(p.validate().is_empty());
        assert_eq!(
            p.egress().unwrap(),
            Egress::Socks5 {
                host: "127.0.0.1".into(),
                port: 7891
            }
        );
    }

    #[test]
    fn socks5_egress_with_slirp_gateway_rejected() {
        let cfg: Config = toml::from_str(
            "version = 1\n[profile.x]\negress = 'socks5://127.0.0.1:7891'\nnet.gateway = 'slirp4netns'",
        )
        .unwrap();
        let errs = cfg.profile.get("x").unwrap().validate();
        assert!(
            errs.iter().any(|e| e.contains("仅支持 net.gateway=pasta")),
            "{errs:?}"
        );
    }

    #[test]
    fn socks6_bracket_form_parses() {
        let cfg: Config =
            toml::from_str("version = 1\n[profile.x]\negress = 'socks5://[::1]:1080'").unwrap();
        assert_eq!(
            cfg.profile["x"].egress().unwrap(),
            Egress::Socks5 {
                host: "::1".into(),
                port: 1080
            }
        );
    }

    #[test]
    fn socks5_malformed_urls_rejected() {
        for bad in [
            "socks5://",            // 空
            "socks5://host",        // 缺端口
            "socks5://host:port",   // 端口非数字
            "socks5://host:99999",  // 端口越界
            "socks5://:7891",       // 空 host
            "socks5://user@host:1", // userinfo v1 不支持（凭据不静默丢弃）
            "socks5://::1:7891",    // 裸 v6（二义）必须括号形态
            "socks5://[::1]",       // 括号缺端口
        ] {
            let cfg: Config =
                toml::from_str(&format!("version = 1\n[profile.x]\negress = {bad:?}")).unwrap();
            let errs = cfg.profile.get("x").unwrap().validate();
            assert!(
                errs.iter().any(|e| e.contains("socks5://<host>:<port>")),
                "{bad:?} 应被拒绝：{errs:?}"
            );
        }
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

    #[test]
    fn engine_defaults_to_netns() {
        // 未声明 = netns（票 18 之前的行为等价）；netns 配置面零回归
        let p: Profile = toml::from_str("egress = 'if:wg0'").unwrap();
        assert_eq!(p.engine(), Engine::Netns);
        assert!(p.validate().is_empty());
    }

    #[test]
    fn engine_mark_parses_and_netns_full_profile_unaffected() {
        let p: Profile = toml::from_str(
            "egress = 'if:mihomo-tun'\nnet.engine = 'mark'\nlocale.tz = 'Asia/Singapore'",
        )
        .unwrap();
        assert_eq!(p.engine(), Engine::Mark);
        assert!(p.validate().is_empty(), "{:?}", p.validate());
    }

    #[test]
    fn engine_unknown_value_rejected() {
        let err = toml::from_str::<Profile>("egress = 'if:w'\nnet.engine = 'bogus'");
        assert!(err.is_err(), "未知 engine 必须拒绝（fail-loud 同 gateway）");
    }

    #[test]
    fn mark_rejects_netns_only_declaratively() {
        let cases = [
            "egress = 'socks5://127.0.0.1:7891'".to_string(),
            "egress = 'if:w'\nredirect = ['/a=/b']".to_string(),
            "egress = 'if:w'\nnet.scope = 'tree'".to_string(),
            "egress = 'if:w'\nnet.ipv6 = 'off'".to_string(),
            "egress = 'if:w'\nnet.localhost_forward = [5432]".to_string(),
            "egress = 'if:w'\nnet.private = 'host'".to_string(),
            "egress = 'if:w'\nnet.gateway = 'pasta'".to_string(),
            "egress = 'if:w'\nnet.dns = '10.0.2.3'".to_string(),
            "egress = 'if:w'\nnet.mcp_fallback = false".to_string(),
        ];
        for body in cases {
            let p: Profile = toml::from_str(&format!("{body}\nnet.engine = 'mark'")).unwrap();
            let errs = p.validate();
            assert!(
                errs.iter()
                    .any(|e| e.contains("mark 引擎") || e.contains("票 18") || e.contains("mark")),
                "{body} 在 mark 下应被声明性拒绝：{errs:?}"
            );
        }
    }

    #[test]
    fn mcp_fallback_defaults_on_and_parses() {
        // 票 20：默认开启（未声明 = true）；显式 true/false 均可解析。
        let p: Profile = toml::from_str("egress = 'if:wg0'").unwrap();
        assert!(p.mcp_fallback(), "未声明 = 默认开启（票 20）");
        let off: Profile =
            toml::from_str("egress = 'if:wg0'\nnet.mcp_fallback = false").unwrap();
        assert!(!off.mcp_fallback());
        let on: Profile = toml::from_str("egress = 'if:wg0'\nnet.mcp_fallback = true").unwrap();
        assert!(on.mcp_fallback());
    }

    #[test]
    fn url_loopback_port_classifies() {
        // 命中面：scheme ∈ {http, https, sse} × host ∈ {127.0.0.1, localhost, ::1}
        let hit = [
            ("http://127.0.0.1:8907/mcp", 8907),
            ("http://localhost:8908/sse", 8908),
            ("https://127.0.0.1:9443/mcp", 9443),
            ("sse://[::1]:9907", 9907),
            ("http://[::1]", 80),
            ("https://localhost", 443),
            ("HTTP://127.0.0.1:8909", 8909), // scheme 大小写不敏感
            ("http://user:pw@127.0.0.1:8910/mcp?token=x", 8910), // userinfo/query 丢弃
        ];
        for (url, want) in hit {
            assert_eq!(url_loopback_port(url), Some(want), "{url}");
        }
        // 拒绝面：非 loopback、非本机制 scheme、畸形、端口 0
        let miss = [
            "http://example.com:8907/mcp",
            "http://10.0.0.5:8907",
            "stdio://127.0.0.1:8907",
            "ftp://127.0.0.1:21",
            "127.0.0.1:8907",     // 无 scheme
            "http://127.0.0.1:0", // 端口 0 无意义
            "http://127.0.0.1:port",
            "",
        ];
        for url in miss {
            assert_eq!(url_loopback_port(url), None, "{url}");
        }
    }

    #[test]
    fn mcp_loopback_ports_scans_claude_and_project_json() {
        let dir = tempfile::tempdir().unwrap();
        // claude.json：root + projects 两层形态；混入 stdio 条目（无 url，不命中）
        let claude = dir.path().join("claude.json");
        std::fs::write(
            &claude,
            r#"{
                "mcpServers": {
                    "local": {"type": "http", "url": "http://127.0.0.1:8907/mcp"},
                    "tools": {"type": "stdio", "command": "mcp-server-tools"},
                    "v6": {"url": "http://[::1]:8908"}
                },
                "projects": {
                    "/home/u/proj": {"mcpServers": {
                        "proj": {"type": "sse", "url": "http://localhost:8909/sse"}
                    }},
                    "/home/u/other": {"mcpServers": {}}
                }
            }"#,
        )
        .unwrap();
        // 项目 .mcp.json：同端口去重 + 新端口
        let mcp = dir.path().join(".mcp.json");
        std::fs::write(
            &mcp,
            r#"{"mcpServers": {
                "again": {"url": "http://127.0.0.1:8907/mcp"},
                "extra": {"url": "https://127.0.0.1:9443"}
            }}"#,
        )
        .unwrap();
        let ports = mcp_loopback_ports(&claude, Some(&mcp));
        assert_eq!(ports, vec![8907, 8908, 8909, 9443]);
        // 只给 claude.json
        assert_eq!(mcp_loopback_ports(&claude, None), vec![8907, 8908, 8909]);
        // 缺失/畸形 = 0 条（不报错）
        assert!(mcp_loopback_ports(&dir.path().join("nope.json"), None).is_empty());
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "not json").unwrap();
        assert!(mcp_loopback_ports(&bad, None).is_empty());
    }
}
