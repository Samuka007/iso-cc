use anyhow::Context;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub id: String,
    pub verdict: Verdict,
    pub detail: String,
}

impl Probe {
    fn new(id: &str, verdict: Verdict, detail: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            verdict,
            detail: detail.into(),
        }
    }
}

fn sh_out(cmd: &str, args: &[&str]) -> Option<(i32, String)> {
    eprintln!("[iso-cc probe] run {cmd} {}", args.join(" "));
    let out = Command::new(cmd).args(args).output().ok()?;
    Some((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    ))
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// glibc 的 %z 形如 +0800 / +0530 → 秒（东正西负）
fn parse_zz(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 5 || (b[0] != b'+') {
        return None;
    }
    let h: i64 = s.get(1..3)?.parse().ok()?;
    let m: i64 = s.get(3..5)?.parse().ok()?;
    Some(h * 3600 + m * 60)
}

/// P8 写集白名单（票 05 / R4 写集例外登记）：cc 已知写在本工具声明重定向集之外的
/// `$HOME` 路径。`autoInstallIdeExtension` 写 `~/.vscode/extensions`；Chrome native
/// messaging manifest 写浏览器配置目录（R4 取证登记）。`/etc/claude-code/managed-
/// settings.json` 是只读渗入（非写点），不入列。发现新写点 = 登记 + 补默认绑定。
pub const P8_ALLOW_UNDER: &[&str] = &[
    ".vscode/extensions",
    ".config/google-chrome",
    ".config/chromium",
    ".config/microsoft-edge",
    ".mozilla",
];

/// P8 判定核（纯函数，票 05 单测锚）：实测写集中落在任何允许根之外的路径。
/// 允许语义 = 路径等于根或为根的子孙（Path 组件级前缀，杜绝字符串前缀误放行
/// `.vscode/extensions-evil` 这类邻居）。
pub fn p8_violations(write_set: &[PathBuf], allowed_roots: &[PathBuf]) -> Vec<PathBuf> {
    write_set
        .iter()
        .filter(|p| !allowed_roots.iter().any(|root| p.starts_with(root)))
        .cloned()
        .collect()
}

/// P8 实测写集（会话内）：`find $HOME -newer <marker>`。marker 由调用方创建于
/// `$HOME` 之外（/tmp），find 输出逐行 = 路径。None = find 不可用（Skip 语义）。
fn p8_write_set(home: &Path, marker: &Path) -> Option<Vec<PathBuf>> {
    let (code, out) = sh_out(
        "find",
        &[home.to_str()?, "-newer", marker.to_str()?, "-print"],
    )?;
    (code == 0).then(|| {
        out.lines()
            .filter(|l| !l.trim().is_empty())
            .map(PathBuf::from)
            .collect()
    })
}

/// 会话内纯净度探针 v0.2（T5 ①子集）。上下文：netns+mountns+locale 注入齐备。
pub fn run(declared_tz: &str, allow_under: &[PathBuf]) -> Vec<Probe> {
    let mut out = Vec::new();

    // R4 探针断言（票 05 spec#3）：会话内 CLAUDE_CONFIG_DIR 必须已 unset——
    // 防宿主环境把 cc 引向声明重定向集之外的第三处（ADR 0006）。
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        None => out.push(Probe::new(
            "R4/claudedir-env",
            Verdict::Pass,
            "CLAUDE_CONFIG_DIR 已 unset（会话 env）",
        )),
        Some(v) => out.push(Probe::new(
            "R4/claudedir-env",
            Verdict::Fail,
            format!("CLAUDE_CONFIG_DIR={v:?} 未 unset（R4：会话 env 必须摘除）"),
        )),
    }

    // P8 marker（票 05 spec#4）：先于全部探针落 /tmp（$HOME 之外），find 在套件
    // 收尾执行——写集观察窗覆盖整个探针期（结构不变式的实测面）。
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/root".into()));
    let marker = std::env::temp_dir().join(format!("iso-cc-p8-marker-{}", std::process::id()));
    let marker_ok = std::fs::write(&marker, b"").is_ok();

    // 期望偏移：由内嵌 tzdb 独立推导（与 glibc 视线互为交叉验证）
    let expect_offset = tzdb::tz_by_name(declared_tz).and_then(|tz| {
        tz.find_local_time_type(now_secs())
            .ok()
            .map(|lt| lt.ut_offset() as i64)
    });

    // P6a glibc 视线：date +%z
    match sh_out("date", &["+%z"]) {
        Some((0, zz)) => match (parse_zz(&zz), expect_offset) {
            (Some(got), Some(want)) if got == want => {
                out.push(Probe::new(
                    "P6a/date",
                    Verdict::Pass,
                    format!("{zz} == tzdb 期望"),
                ));
            }
            (Some(got), want) => out.push(Probe::new(
                "P6a/date",
                Verdict::Fail,
                format!("{zz}（{got}s）≠ tzdb 期望偏移 {want:?}"),
            )),
            (None, _) => out.push(Probe::new(
                "P6a/date",
                Verdict::Fail,
                format!("date +%z 输出不可解析：{zz:?}"),
            )),
        },
        _ => out.push(Probe::new("P6a/date", Verdict::Skip, "date 不可用")),
    }

    // P6b Intl 视线（node 可用时）
    match sh_out(
        "node",
        &[
            "-e",
            "console.log(Intl.DateTimeFormat().resolvedOptions().timeZone)",
        ],
    ) {
        Some((0, tz)) if tz == declared_tz => {
            out.push(Probe::new(
                "P6b/intl",
                Verdict::Pass,
                format!("Intl = {tz}"),
            ));
        }
        Some((0, tz)) => out.push(Probe::new(
            "P6b/intl",
            Verdict::Fail,
            format!("Intl = {tz:?} ≠ 声明 {declared_tz:?}"),
        )),
        _ => out.push(Probe::new("P6b/intl", Verdict::Skip, "node 不可用")),
    }

    // P6c bind 生效证据：mounts 含 /etc/localtime 且文件为 TZif
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let bind_active = mounts
        .lines()
        .any(|l| l.split_whitespace().nth(1) == Some("/etc/localtime"));
    let magic = std::fs::read("/etc/localtime")
        .map(|b| b.starts_with(b"TZif"))
        .unwrap_or(false);
    out.push(Probe::new(
        "P6c/localtime-bind",
        if bind_active && magic {
            Verdict::Pass
        } else {
            Verdict::Skip
        },
        format!(
            "bind={bind_active} TZif 魔数={magic}（未 bind = 宿主 /etc 不可预创建，TZ env 已覆盖）"
        ),
    ));

    // P1 出口 IPv4
    let v4 = sh_out("curl", &["-4", "--max-time", "10", "-s", "ifconfig.me"]);
    let egress_ip = match &v4 {
        Some((0, ip)) if !ip.is_empty() => Some(ip.clone()),
        _ => None,
    };
    out.push(Probe::new(
        "P1/egress-v4",
        if egress_ip.is_some() {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
        format!("egress = {:?}", egress_ip.as_deref().unwrap_or("无响应")),
    ));

    // P2 IPv6 必须 fail（v6 off）
    match sh_out("curl", &["-6", "--max-time", "8", "-s", "ifconfig.me"]) {
        Some((0, ip)) if !ip.is_empty() => out.push(Probe::new(
            "P2/egress-v6",
            Verdict::Fail,
            format!("v6 竟然可用：{ip}（泄漏）"),
        )),
        Some(_) => out.push(Probe::new(
            "P2/egress-v6",
            Verdict::Pass,
            "v6 如预期失败/为空",
        )),
        None => out.push(Probe::new("P2/egress-v6", Verdict::Skip, "curl 缺失")),
    }

    // P13 区域归属：出口 IP 的 geo-timezone 必须 == 声明 tz
    match sh_out(
        "curl",
        &[
            "--max-time",
            "10",
            "-s",
            "http://ip-api.com/json?fields=status,countryCode,timezone,query",
        ],
    ) {
        Some((0, body)) => match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(v) => {
                let geo_tz = v["timezone"].as_str().unwrap_or("").to_string();
                let cc = v["countryCode"].as_str().unwrap_or("?").to_string();
                let ip = v["query"].as_str().unwrap_or("?").to_string();
                if geo_tz.is_empty() {
                    out.push(Probe::new(
                        "P13/region",
                        Verdict::Warn,
                        format!("geo 无 timezone 字段：{body}"),
                    ));
                } else if geo_tz == declared_tz {
                    out.push(Probe::new(
                        "P13/region",
                        Verdict::Pass,
                        format!("geo={geo_tz}({cc}) == 声明；exit-ip={ip}"),
                    ));
                } else {
                    out.push(Probe::new(
                        "P13/region",
                        Verdict::Fail,
                        format!("geo={geo_tz}({cc}) ≠ 声明 {declared_tz}；exit-ip={ip}"),
                    ));
                }
            }
            Err(e) => out.push(Probe::new(
                "P13/region",
                Verdict::Warn,
                format!("geo 响应不可解析：{e}"),
            )),
        },
        _ => out.push(Probe::new(
            "P13/region",
            Verdict::Warn,
            "ip-api 不可达（留档）",
        )),
    }

    // P15 出口恒定（两次采样一致）
    let second = sh_out("curl", &["-4", "--max-time", "10", "-s", "ifconfig.me"]);
    match (&egress_ip, &second) {
        (Some(a), Some((0, b))) if a == b => {
            out.push(Probe::new(
                "P15/constancy",
                Verdict::Pass,
                format!("{a} 恒定"),
            ));
        }
        (Some(a), Some((0, b))) => out.push(Probe::new(
            "P15/constancy",
            Verdict::Fail,
            format!("漂移：{a} → {b}"),
        )),
        _ => out.push(Probe::new("P15/constancy", Verdict::Skip, "首样本缺失")),
    }

    // P3b getaddrinfo 路径（功能性 DNS；nsswitch 防御属第二批）
    match sh_out("getent", &["ahostsv4", "api.anthropic.com"]) {
        Some((0, line)) => out.push(Probe::new(
            "P3b/dns-getaddrinfo",
            Verdict::Pass,
            format!(
                "api.anthropic.com → {}",
                line.split_whitespace().next().unwrap_or("?")
            ),
        )),
        _ => out.push(Probe::new(
            "P3b/dns-getaddrinfo",
            Verdict::Fail,
            "getaddrinfo 解析 api.anthropic.com 失败",
        )),
    }

    // P8 写集探针判定（套件收尾执行 find）：实测写集 ⊆ 声明重定向集 ∪ 白名单
    // （允许根由 spawn 侧按声明推导注入）。白名单外出现即 Fail（R4 核心不变式）。
    match (marker_ok, p8_write_set(&home, &marker)) {
        (true, Some(write_set)) => {
            let violations = p8_violations(&write_set, allow_under);
            if violations.is_empty() {
                out.push(Probe::new(
                    "P8/write-set",
                    Verdict::Pass,
                    format!(
                        "写集 {} 条 ⊆ 允许根 {} 条（声明重定向集 ∪ R4 白名单 ∪ 工具状态根）",
                        write_set.len(),
                        allow_under.len()
                    ),
                ));
            } else {
                let shown: Vec<String> = violations
                    .iter()
                    .take(5)
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                out.push(Probe::new(
                    "P8/write-set",
                    Verdict::Fail,
                    format!(
                        "白名单外写集 ×{}：{}{}（R4 不变式：先补默认绑定 + 白名单登记）",
                        violations.len(),
                        shown.join(", "),
                        if violations.len() > 5 { " …" } else { "" }
                    ),
                ));
            }
        }
        (true, None) => out.push(Probe::new("P8/write-set", Verdict::Skip, "find 不可用")),
        (false, _) => out.push(Probe::new(
            "P8/write-set",
            Verdict::Skip,
            "marker 创建失败（/tmp 不可写）",
        )),
    }
    let _ = std::fs::remove_file(&marker);

    eprintln!("[iso-cc probe] suite done");
    out
}

pub fn any_fail(probes: &[Probe]) -> bool {
    probes.iter().any(|p| p.verdict == Verdict::Fail)
}

pub fn render_human(probes: &[Probe]) -> String {
    let mut s = String::new();
    for p in probes {
        let tag = match p.verdict {
            Verdict::Pass => "PASS",
            Verdict::Warn => "WARN",
            Verdict::Fail => "FAIL",
            Verdict::Skip => "SKIP",
        };
        s.push_str(&format!("[{tag}] {} — {}\n", p.id, p.detail));
    }
    let f = probes.iter().filter(|p| p.verdict == Verdict::Fail).count();
    let w = probes.iter().filter(|p| p.verdict == Verdict::Warn).count();
    let s2 = probes.iter().filter(|p| p.verdict == Verdict::Skip).count();
    s.push_str(&format!(
        "\n{} probes: {} pass, {w} warn, {f} fail, {s2} skip\n",
        probes.len(),
        probes.len() - f - w - s2
    ));
    s
}

/// JSON 序列化入口（verify 的机器可读输出）
pub fn to_json(probes: &[Probe]) -> anyhow::Result<String> {
    serde_json::to_string_pretty(probes).context("序列化探针结果")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(base: &Path) -> Vec<PathBuf> {
        vec![
            base.join(".claude"),
            base.join(".claude.json"),
            base.join(".local/state/iso-cc"),
            base.join(".vscode/extensions"),
        ]
    }

    #[test]
    fn p8_passes_write_inside_declared_or_whitelist() {
        let home = PathBuf::from("/home/u");
        let ws = vec![
            home.join(".claude/foo"),                                // 内置对 view 内
            home.join(".claude/projects/x.jsonl"),                   // view 孙路径
            home.join(".claude.json"),                               // 等于根
            home.join(".local/state/iso-cc/sessions/s/gateway.log"), // 工具状态根
            home.join(".vscode/extensions/pub.ext"),                 // R4 白名单
        ];
        assert!(p8_violations(&ws, &roots(&home)).is_empty());
    }

    #[test]
    fn p8_fails_write_outside_allowlist() {
        let home = PathBuf::from("/home/u");
        let ws = vec![
            home.join(".config/some-tool/conf.toml"), // 白名单外
            home.join(".ssh/known_hosts"),            // 白名单外
        ];
        let v = p8_violations(&ws, &roots(&home));
        assert_eq!(v, ws, "白名单外写集必须逐条上报（R4 核心不变式）");
    }

    #[test]
    fn p8_component_boundary_no_string_prefix_escape() {
        // 组件级前缀：`.vscode/extensions-evil` 不是 `.vscode/extensions` 的子孙
        let home = PathBuf::from("/home/u");
        let ws = vec![
            home.join(".vscode/extensions-evil/x"),
            home.join(".claudej"),
        ];
        let v = p8_violations(&ws, &roots(&home));
        assert_eq!(v.len(), 2, "{v:?}");
    }

    #[test]
    fn p8_empty_write_set_passes() {
        let home = PathBuf::from("/home/u");
        assert!(p8_violations(&[], &roots(&home)).is_empty());
    }
}
