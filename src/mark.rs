//! mark.rs —— mark 引擎路由面 + file-cap 助手（票 18；spec 变更（五）第二候选引擎）。
//!
//! 阶段 A 实证（/tmp/iso-cc-exp18/REPORT.md，parent 亲验）：uid 4210（零 netns）→
//! `ip rule uidrange` → 表 5182 → mihomo-tun → SG 出口；`unreachable` 兜底为
//! fail-closed 必要件（表空 fall-through main = 静默 fail-open，阶段 A 发现 1）。
//!
//! 职责面（阶段 B 产品化）：
//! - 助手（[`HELPER_SRC`]，include_str! 随二进制发行）：setup 期 cc 编译 +
//!   content-hash 幂等（[`ensure_helper`]）；安装 = rootful 步骤（setcap），
//!   本工具零 sudo（票 18 约束：setup = 命令构建 + dry-run 输出，rootful 逐条
//!   可审计、可回滚）。
//! - 路由面命令序列（[`rootful_steps`] / [`teardown_commands`]）：与阶段 A
//!   proven 形态同源（del-add/replace 幂等 + unreachable 兜底 + v6 恒 fail-closed）。
//! - 现实解析（[`parse_rule_show`] / [`parse_route_table`]）：`ip` 输出纯函数
//!   （doctor / run 预检共用判定核）。
//!
//! 固定常量面：uid/table/pref 全部编译期常量（[`crate::config::MARK_UID`] 等）
//! = 单一 proven 形态；参数化 = 后续票声明化，不在本票扩轴。

use crate::config::{MARK_RULE_PREF, MARK_TABLE, MARK_UID};
use crate::manifest::{self, EntryKind};
use anyhow::{anyhow, Context};
use std::path::{Path, PathBuf};
use std::process::Command;

/// 助手二进制名（state_dir/mark/ 下）。
pub const HELPER_NAME: &str = "uidrun";

/// mark 引擎的会话可见状态根（票 18 宿主矩阵实测：宿主 $HOME 通常 0700——
/// uid 4210 连 traverse 都 EACCES，exec.sock/shim/HOME backing 全不可达）。
/// 会话侧可见面（exec 通道 + mark HOME）统一落 /var/tmp/iso-cc-mark/（世界可
/// 遍历，生命周期仍受两级模型管辖：sessions/ 随会话消亡 + sweep 对账，profiles/
/// 由 profile-state 清单登记、gc 按显式 --profile 回收）。helper/manifest 等
/// 仅宿主面（uid 1000 访问）留在 state_dir——capability 助手的可执行面借此
/// 收敛到属主用户。
pub const STATE_ROOT: &str = "/var/tmp/iso-cc-mark";

/// mark 会话可见状态根。
pub fn state_root() -> PathBuf {
    PathBuf::from(STATE_ROOT)
}

/// mark 会话资产根（exec.sock/shim 等；Session::wait 回收 + sweep 对账）。
pub fn sessions_root() -> PathBuf {
    state_root().join("sessions")
}

/// mark HOME backing 根（uid 4210 属主，rootful chown 应用）。
pub fn profiles_root() -> PathBuf {
    state_root().join("profiles")
}

/// mark 会话的 HOME（cc 无感走默认 ~/.claude 路径的载体）。
pub fn home_root(profile_name: &str) -> PathBuf {
    profiles_root().join(profile_name)
}

/// shim/probe 执行器二进制的供给（current_exe → <mark_root>/bin/iso-cc）。
/// uid 4210 对宿主工具链路径（~/.cargo、crate target/ 等 0700 链）不可达——
/// exec.bash=host 的 multi-call shim 与 verify 的 probe-json 执行器都必须从
/// mark 根取二进制。内容 diff 才拷贝（幂等：重跑零写入）。
pub fn ensure_shim_binary() -> anyhow::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;
    let exe = std::env::current_exe().context("current_exe 不可用")?;
    let dir = state_root().join("bin");
    std::fs::create_dir_all(&dir).with_context(|| format!("创建 {}", dir.display()))?;
    let dst = dir.join("iso-cc");
    let src = std::fs::read(&exe).context("读取自身二进制")?;
    let stale = std::fs::read(&dst).map(|d| d != src).unwrap_or(true);
    if stale {
        let tmp = dir.join(".iso-cc.copy.tmp");
        std::fs::write(&tmp, &src).with_context(|| format!("写 {}", tmp.display()))?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("chmod 0755 {}", tmp.display()))?;
        std::fs::rename(&tmp, &dst).with_context(|| format!("生效 {}", dst.display()))?;
    }
    Ok(dst)
}

/// mark 版 CC 内置对：view（cc 默认路径）不变，backing 根改为 mark 状态根
/// （uid DAC 属主面）；声明集与 netns 同源（票 05），仅承载位置随引擎迁移。
pub fn cc_builtin_pairs_mark(profile_name: &str) -> Vec<crate::config::CcBuiltinPair> {
    crate::config::cc_builtin_pairs(profile_name)
        .into_iter()
        .map(|pair| crate::config::CcBuiltinPair {
            backing: profiles_root().join(profile_name).join(pair.key),
            ..pair
        })
        .collect()
}

/// 助手 C 源（唯一事实源，随 iso-cc 二进制发行；setup 期 cc 编译）。
pub const HELPER_SRC: &str = include_str!("mark-uidrun.c");

/// cap-bin 清单条目 key（profile 定界，gc --profile 同构 mountpoint）。
pub fn cap_bin_key(profile_name: &str) -> String {
    format!("{profile_name}:{HELPER_NAME}")
}

/// route-rule 清单条目 key（profile 定界）。
pub fn route_rule_key(profile_name: &str) -> String {
    format!("{profile_name}:route-rule")
}

/// 助手安装路径（派生自 state_dir；file caps 随文件，gc rm 即回收）。
pub fn helper_path() -> PathBuf {
    manifest::state_dir().join("mark").join(HELPER_NAME)
}

/// FNV-1a 64（无新依赖的 content 指纹；用途 = 幂等比对，非安全面）。
fn fnv64(data: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 宿主 tzdb 根（NixOS = /etc/zoneinfo；FHS 布局 = 无此目录，glibc 默认
/// /usr/share/zoneinfo 即可，无需 TZDIR）。存在 = 编译期钉入助手（secure-exec
/// 过滤剥 TZDIR 的重建源，见 mark-uidrun.c restore_tzdir）。
fn host_tzdir() -> Option<&'static str> {
    Path::new("/etc/zoneinfo")
        .is_dir()
        .then_some("/etc/zoneinfo")
}

fn helper_meta_want() -> String {
    format!(
        "uid={MARK_UID}\ntzdir={:?}\nsrc={:016x}\n",
        host_tzdir(),
        fnv64(HELPER_SRC)
    )
}

fn helper_meta_current(path: &Path) -> Option<String> {
    std::fs::read_to_string(path.with_extension("meta")).ok()
}

/// setup 期收敛：助手缺失或源/uid 漂移时重编译（幂等：重跑 diff = 0）。
/// 返回 true = 本轮发生了编译动作。
///
/// 编译走 setup 期 cc 而非 build.rs（裁决留档）：① 助手是宿主态资源
/// （setup-manifested），不是 iso-cc 的构建产物——二进制自包含 C 源
/// （include_str!）+ 目标机现编，Nix 打包保持单一静态工件（US16）且不被
/// 原生工具链耦合；② 失败形态与 provider 纪律同构（cc 不在 PATH = 报包名，
/// 不代装）；③ content-hash 幂等与清单逻辑同栖一处。
pub fn ensure_helper() -> anyhow::Result<bool> {
    let dir = manifest::state_dir().join("mark");
    std::fs::create_dir_all(&dir).with_context(|| format!("创建 {}", dir.display()))?;
    let path = helper_path();
    let want = helper_meta_want();
    if path.is_file()
        && helper_meta_current(&path).as_deref() == Some(want.as_str())
        && is_executable(&path)
    {
        return Ok(false);
    }
    let cc = crate::provider::which_any(&["cc", "gcc", "clang"]).ok_or_else(|| {
        anyhow!(
            "setup：cc 不在 PATH（mark 助手编译）——缺失报包名：NixOS gcc / clang；setup 不代装（票 18：助手 = setup-manifested，编译期钉定 uid {MARK_UID}）"
        )
    })?;
    // 源文件落盘（审计面：清单路径旁恒有与二进制同源的 C 源）。
    let src_path = dir.join(format!("{HELPER_NAME}.c"));
    std::fs::write(&src_path, HELPER_SRC).with_context(|| format!("写 {}", src_path.display()))?;
    // 同目录临时名编译 → rename 原子生效（半成品不可见）。
    let tmp = dir.join(format!(".{HELPER_NAME}.compile.tmp"));
    let _ = std::fs::remove_file(&tmp);
    let out = Command::new(&cc)
        .args([
            "-O2",
            "-std=c11",
            "-Wall",
            &format!("-DMARK_UID={MARK_UID}"),
        ])
        .args(host_tzdir().map(|d| format!("-DMARK_TZDIR=\"{d}\"")))
        .arg("-o")
        .arg(&tmp)
        .arg(&src_path)
        .output()
        .with_context(|| format!("运行 cc（{}）编译 mark 助手", cc.display()))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        bail_helper_compile(&cc, &out.stderr);
    }
    std::fs::rename(&tmp, &path).with_context(|| format!("生效助手二进制 {}", path.display()))?;
    std::fs::write(path.with_extension("meta"), &want)
        .with_context(|| format!("写 {}", path.with_extension("meta").display()))?;
    Ok(true)
}

fn bail_helper_compile(cc: &Path, stderr: &[u8]) -> anyhow::Error {
    anyhow!(
        "mark 助手编译失败（cc={}）：\n{}",
        cc.display(),
        String::from_utf8_lossy(stderr).trim()
    )
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// 钉定路径（fail-loud #2 清单门同构）：从清单 CapBin 条目取助手绝对路径。
pub fn pinned_helper(profile_name: &str) -> anyhow::Result<PathBuf> {
    let m = manifest::read()?.unwrap_or_else(manifest::Manifest::empty);
    let key = cap_bin_key(profile_name);
    match m.find(EntryKind::CapBin, &key) {
        Some(e) => e
            .path
            .clone()
            .ok_or_else(|| anyhow!("mark 助手清单条目缺 path（清单损坏）：重跑 `iso-cc setup`")),
        None => Err(anyhow!(
            "mark 助手未登记（清单无 {key} 条目，fail-loud #2 清单态）——先运行 `iso-cc setup`"
        )),
    }
}

/// spawn 前 file-cap 在位断言：真实降权一次（--probe，无副作用即退）。
/// EPERM = setcap 未应用（rootful 步骤缺失），fail-loud。
pub fn probe_helper(path: &Path) -> anyhow::Result<()> {
    let out = Command::new(path)
        .arg("--probe")
        .output()
        .with_context(|| format!("运行 mark 助手 {}", path.display()))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(anyhow!(
        "mark 助手 capability 断言失败（exit={}）：{}——setcap 未应用？运行 `iso-cc setup` 并逐条应用其 rootful 步骤（setcap cap_setuid,cap_setgid=ep …）",
        out.status.code().unwrap_or(-1),
        stderr.trim()
    ))
}

fn ip_output(args: &[&str]) -> Option<String> {
    let out = Command::new("ip").args(args).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// 策略路由现实（v4+v6 五元判定核；doctor 与 run 预检共用解析，`ip` 输出注入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RouteFace {
    pub v4_rule: bool,
    pub v4_dev: bool,
    pub v4_backstop: bool,
    pub v6_rule: bool,
    pub v6_backstop: bool,
}

impl RouteFace {
    /// 缺失项人类可读清单（空 = 全绿）。
    pub fn missing(&self, iface: &str) -> Vec<String> {
        let mut v = Vec::new();
        if !self.v4_rule {
            v.push("v4 uidrange 规则".to_string());
        }
        if !self.v4_dev {
            v.push(format!("v4 default dev {iface} 路由"));
        }
        if !self.v4_backstop {
            v.push("v4 unreachable 兜底".to_string());
        }
        if !self.v6_rule {
            v.push("v6 uidrange 规则".to_string());
        }
        if !self.v6_backstop {
            v.push("v6 unreachable 兜底".to_string());
        }
        v
    }
}

/// 读取真实路由面（`ip` 不可用 = 全假 → 调用方 fail-loud 提示）。
pub fn route_face(iface: &str) -> RouteFace {
    let rules = ip_output(&["rule", "show"]).unwrap_or_default();
    let routes =
        ip_output(&["route", "show", "table", &MARK_TABLE.to_string()]).unwrap_or_default();
    let rules6 = ip_output(&["-6", "rule", "show"]).unwrap_or_default();
    let routes6 =
        ip_output(&["-6", "route", "show", "table", &MARK_TABLE.to_string()]).unwrap_or_default();
    RouteFace {
        v4_rule: parse_rule_show(&rules),
        v4_dev: parse_route_table(&routes, iface).0,
        v4_backstop: parse_route_table(&routes, iface).1,
        v6_rule: parse_rule_show(&rules6),
        v6_backstop: parse_route_table(&routes6, iface).1,
    }
}

/// 纯解析：`ip rule show` 文本中存在 pref/uidrange/table 三元齐备的行。
pub fn parse_rule_show(text: &str) -> bool {
    // `ip rule show` 渲染 pref 为行首 `<n>:`（add 输入用 `pref <n>`；解析两态皆收）。
    let want_pref_colon = format!("{MARK_RULE_PREF}:");
    let want_pref_word = format!("pref {MARK_RULE_PREF}");
    let want_range = format!("uidrange {MARK_UID}-{MARK_UID}");
    // iproute2 渲染 `table` 为 `lookup`（add 输入两者同义；解析同时接受）。
    let want_table = format!("lookup {MARK_TABLE}");
    let want_table_alt = format!("table {MARK_TABLE}");
    text.lines().any(|l| {
        (l.trim_start().starts_with(&want_pref_colon) || l.contains(&want_pref_word))
            && l.contains(&want_range)
            && (l.contains(&want_table) || l.contains(&want_table_alt))
    })
}

/// 纯解析：`ip route show table <T>` 文本 → (default dev <iface> 在位, unreachable 兜底在位)。
pub fn parse_route_table(text: &str, iface: &str) -> (bool, bool) {
    let want_dev = format!("default dev {iface} ");
    let want_dev_eol = format!("default dev {iface}");
    let dev = text.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with(&want_dev) || t == want_dev_eol
    });
    let backstop = text
        .lines()
        .any(|l| l.trim_start().starts_with("unreachable default"));
    (dev, backstop)
}

/// setup 输出的 rootful 步骤（dry-run 审计块；幂等 replace/del-add；票 18 约束：
/// 本工具永不 sudo，rootful 只出现在「用户逐条审计应用」的 setup 形态）。
///
/// 顺序语义：助手 setcap → backing chown → 路由面（须在 mihomo TUN 形态启动后；
/// tun down/up 抖动永久丢 dev 路由（阶段 A 发现 2）→ 重跑本块即自愈）。
pub fn rootful_steps(profile_name: &str, iface: &str, chown_backing: bool) -> Vec<String> {
    let u = MARK_UID;
    let (t, p) = (MARK_TABLE, MARK_RULE_PREF);
    let mut v = vec![
        "# 1) mark 助手 file-cap（持久 capability = setup-manifested；doctor getcap 断言 / gc rm 回收）".to_string(),
        format!(
            "sudo -n setcap cap_setuid,cap_setgid=ep {}",
            helper_path().display()
        ),
    ];
    if chown_backing {
        let base = home_root(profile_name);
        v.push(
            "# 2) CC backing 属主（mark 版隔离 = uid DAC + HOME 重写；不 chown 则 cc 无法写状态）"
                .to_string(),
        );
        v.push(format!("sudo -n chown -R {u}:{u} {}", base.display()));
    }
    v.push(
        "# 3) 路由面（幂等；必须在 mihomo TUN 形态启动后执行；tun down/up 抖动后重跑本块即自愈）"
            .to_string(),
    );
    v.push(format!(
        "sudo -n ip rule del pref {p} uidrange {u}-{u} table {t} 2>/dev/null || true"
    ));
    v.push(format!(
        "sudo -n ip rule add pref {p} uidrange {u}-{u} table {t}"
    ));
    v.push(format!(
        "sudo -n ip route replace default dev {iface} table {t} metric 100"
    ));
    v.push(format!(
        "sudo -n ip route replace unreachable default table {t} metric 2048"
    ));
    v.push(format!(
        "sudo -n ip -6 rule del pref {p} uidrange {u}-{u} table {t} 2>/dev/null || true"
    ));
    v.push(format!(
        "sudo -n ip -6 rule add pref {p} uidrange {u}-{u} table {t}"
    ));
    v.push(format!(
        "sudo -n ip -6 route replace unreachable default table {t} metric 2048"
    ));
    v
}

/// gc 的路由面回滚命令（与 /tmp/iso-cc-exp18/rollback.sh 同源：flush 表 + del 规则）。
/// rootful = 打印由用户应用；助手文件由 gc 直接 rm（uid 属主，file caps 随文件消亡）。
pub fn teardown_commands() -> Vec<String> {
    let u = MARK_UID;
    let (t, p) = (MARK_TABLE, MARK_RULE_PREF);
    vec![
        format!("sudo -n ip rule del pref {p} uidrange {u}-{u} table {t} 2>/dev/null || true"),
        format!("sudo -n ip -6 rule del pref {p} uidrange {u}-{u} table {t} 2>/dev/null || true"),
        format!("sudo -n ip route flush table {t}"),
        format!("sudo -n ip -6 route flush table {t}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULE_OK: &str = "\
0:	from all lookup local
15000:	from all uidrange 4210-4210 lookup 5182
32766:	from all lookup main
32767:	from all lookup default
";
    const RULE_ABSENT: &str = "\
0:	from all lookup local
32766:	from all lookup main
";
    const RULE_WRONG_UID: &str = "15000:	from all uidrange 4211-4211 lookup 5182\n";

    const TABLE_OK: &str = "\
default dev mihomo-tun metric 100
unreachable default metric 2048
";
    const TABLE_DEV_ONLY: &str = "default dev mihomo-tun metric 100\n";
    const TABLE_BACKSTOP_ONLY: &str = "unreachable default metric 2048\n";
    const TABLE_EMPTY: &str = "";

    #[test]
    fn parse_rule_show_accepts_proven_form() {
        assert!(parse_rule_show(RULE_OK));
        assert!(!parse_rule_show(RULE_ABSENT));
        assert!(!parse_rule_show(RULE_WRONG_UID));
    }

    #[test]
    fn parse_route_table_separates_dev_and_backstop() {
        assert_eq!(parse_route_table(TABLE_OK, "mihomo-tun"), (true, true));
        assert_eq!(
            parse_route_table(TABLE_DEV_ONLY, "mihomo-tun"),
            (true, false)
        );
        assert_eq!(
            parse_route_table(TABLE_BACKSTOP_ONLY, "mihomo-tun"),
            (false, true)
        );
        assert_eq!(parse_route_table(TABLE_EMPTY, "mihomo-tun"), (false, false));
        // 接口名必须整词命中（前缀邻居不误报）
        assert_eq!(
            parse_route_table("default dev mihomo-tun2 metric 100\n", "mihomo-tun"),
            (false, false)
        );
    }

    #[test]
    fn missing_lists_each_broken_leg() {
        let full = RouteFace {
            v4_rule: true,
            v4_dev: true,
            v4_backstop: true,
            v6_rule: true,
            v6_backstop: true,
        };
        assert!(full.missing("mihomo-tun").is_empty());
        let none = RouteFace::default();
        let m = none.missing("mihomo-tun");
        assert_eq!(m.len(), 5, "{m:?}");
        let no_backstop = RouteFace {
            v4_rule: true,
            v4_dev: true,
            v4_backstop: false,
            v6_rule: true,
            v6_backstop: true,
        };
        assert_eq!(
            no_backstop.missing("x"),
            vec!["v4 unreachable 兜底".to_string()]
        );
    }

    #[test]
    fn rootful_steps_shape() {
        let steps = rootful_steps("sg", "mihomo-tun", true);
        let joined = steps.join("\n");
        assert!(joined.contains("setcap cap_setuid,cap_setgid=ep "));
        assert!(joined.contains("chown -R 4210:4210 "));
        assert!(joined.contains("ip rule add pref 15000 uidrange 4210-4210 table 5182"));
        assert!(joined.contains("ip route replace default dev mihomo-tun table 5182 metric 100"));
        assert!(joined.contains("ip route replace unreachable default table 5182 metric 2048"));
        assert!(joined.contains("ip -6 rule add pref 15000 uidrange 4210-4210 table 5182"));
        assert!(joined.contains("ip -6 route replace unreachable default table 5182 metric 2048"));
        // chown 免除面：cc_isolation=false 时无 chown
        let no_chown = rootful_steps("sg", "mihomo-tun", false).join("\n");
        assert!(!no_chown.contains("chown"));
    }

    #[test]
    fn teardown_shape_matches_rollback_sh() {
        let t = teardown_commands().join("\n");
        assert!(t.contains("ip rule del pref 15000 uidrange 4210-4210 table 5182"));
        assert!(t.contains("ip -6 rule del"));
        assert!(t.contains("ip route flush table 5182"));
    }

    #[test]
    fn fnv64_stable() {
        assert_eq!(fnv64("abc"), fnv64("abc"));
        assert_ne!(fnv64("abc"), fnv64("abd"));
    }

    #[test]
    fn cc_builtin_pairs_mark_backings_land_in_mark_root() {
        // 回归锚（票 18 集成缺口）：mark 版内置对 backing 必须落 mark 状态根
        //（/var/tmp/iso-cc-mark/profiles/<p>），view 保持 cc 默认路径不变——
        // 防 run 期任何 callsite 退回 netns 派生（宿主 state 根）。
        let pairs = cc_builtin_pairs_mark("mp");
        assert_eq!(pairs.len(), 3);
        for pair in &pairs {
            assert!(
                pair.backing.starts_with(profiles_root().join("mp")),
                "backing 必须在 mark 根：{}",
                pair.backing.display()
            );
            assert!(!pair.backing.starts_with(manifest::state_dir()));
        }
        let netns_pairs = crate::config::cc_builtin_pairs("mp");
        for (m, n) in pairs.iter().zip(netns_pairs.iter()) {
            assert_eq!(m.key, n.key);
            assert_eq!(m.view, n.view);
            assert_eq!(m.backing_is_dir, n.backing_is_dir);
        }
    }

    #[test]
    fn helper_source_compiles_and_self_reports() {
        // 真编译（devshell cc）：C 源有效性 + 编译期 uid 钉定 + --check 输出契约。
        // hermetic：全部产物在 std::env::temp_dir() 唯一子目录，测试尾清理。
        let cc = match crate::provider::which_any(&["cc", "gcc", "clang"]) {
            Some(c) => c,
            None => return, // 无 cc 环境跳过（setup 主路径会 fail-loud）
        };
        let dir = std::env::temp_dir().join(format!("iso-cc-mark-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("mark-uidrun.c");
        let bin = dir.join("uidrun");
        std::fs::write(&src, HELPER_SRC).unwrap();
        let out = Command::new(&cc)
            .args(["-O2", "-std=c11", "-Wall", "-DMARK_UID=4210", "-o"])
            .arg(&bin)
            .arg(&src)
            .output()
            .expect("cc 可运行");
        assert!(
            out.status.success(),
            "编译失败：{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let check = Command::new(&bin).arg("--check").output().unwrap();
        assert!(check.status.success());
        assert_eq!(
            String::from_utf8_lossy(&check.stdout).trim(),
            "uid=4210 gid=4210"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
