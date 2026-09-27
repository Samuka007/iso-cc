//! 网关 provider 层（设计稿 §1/§5）：pasta spawn 转正 + slirp4netns 回退。
//! argv 展开是 plan 打印与实际 spawn 的单一事实源（§7 D5：`--print-plan` 终点 = 等价 CLI 组合）。

pub mod pasta;
pub mod slirp;
pub mod socks;

use crate::config::NetGateway;
use anyhow::{anyhow, bail, Context};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// pasta `-I` 目标 ns 内 tap 名（issue 10 §Answer 硬规则：恒显式指定，
/// 绝不回落「默认取 outbound 接口名」的命名规则）。
pub const NS_IFNAME: &str = "tap0";

pub(crate) fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|cand| cand.is_file())
}

/// 多候选名 which（首个命中即返回）：tun2proxy 上游叫 `tun2proxy`、nixpkgs 叫
/// `tun2proxy-bin`——清单 key 恒 [`socks::WORKER_KEY`]，与 bin 名解耦。
pub(crate) fn which_any(bins: &[&str]) -> Option<PathBuf> {
    bins.iter().find_map(|b| which(b))
}

/// 宿主默认路由接口（`/proc/net/route` 目的地址 00000000 的首条；socks 形态下即
/// pasta 的 outbound —— 宿主 egress 语义保持「跟随宿主路由事实」）。缺失 = fail-loud。
pub fn host_default_iface() -> anyhow::Result<String> {
    let text = std::fs::read_to_string("/proc/net/route")
        .map_err(|e| anyhow!("#3 读取 /proc/net/route 失败（宿主默认路由接口不可得）: {e}"))?;
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 3 && cols[1] == "00000000" {
            return Ok(cols[0].to_string());
        }
    }
    bail!("#3 /proc/net/route 无默认路由（目的 00000000）；宿主无出口，R8：绝不回落");
}
/// provider `--version` 输出（trim 后；setup 登记 / doctor 漂移比对的版本事实源）。
pub(crate) fn version_output(bin: &Path) -> anyhow::Result<String> {
    let out = std::process::Command::new(bin)
        .arg("--version")
        .output()
        .with_context(|| format!("执行 {} --version", bin.display()))?;
    if !out.status.success() {
        anyhow::bail!(
            "{} --version 失败（exit {:?}）",
            bin.display(),
            out.status.code()
        );
    }
    // 版本钉定取首个非空行（pasta 的 license 尾块是常量噪声；构建事实变化仍会漂移）
    let v = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string();
    if v.is_empty() {
        anyhow::bail!(
            "{} --version 无 stdout（版本事实缺失，fail-loud）",
            bin.display()
        );
    }
    Ok(v)
}

/// helper 目录环境变量（工单 08 设计 #3，对标 podman `CONTAINERS_HELPER_BINARY_DIR`）：
/// bundle 形态（tar.gz 解包后的 `libexec/`）无需 setup 即可解析 provider。
pub const HELPER_DIR_ENV: &str = "ISO_CC_HELPER_DIR";

/// provider 清单 key → 可接受的二进制名候选（env 目录与 PATH 两层共用）。
fn bin_candidates(name: &str) -> Vec<&str> {
    if name == crate::provider::socks::WORKER_KEY {
        crate::provider::socks::BIN_CANDIDATES.to_vec()
    } else {
        vec![name]
    }
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// 第二层：`ISO_CC_HELPER_DIR` 指向的目录内按候选名找可执行。
fn env_dir_bin(name: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os(HELPER_DIR_ENV)?);
    bin_candidates(name).into_iter().find_map(|b| {
        let cand = dir.join(b);
        is_executable(&cand).then_some(cand)
    })
}

/// provider 可执行解析门（工单 08 设计 #3 优先级链）：
/// **manifest 钉路径 > `ISO_CC_HELPER_DIR` env > PATH**（对标 podman
/// `CONTAINERS_HELPER_BINARY_DIR`；14 实现了首尾两层，08 补 env 层）。
///
/// - manifest 可读且有本 provider 条目 → 钉定路径唯一裁决：path 缺失（清单损坏）
///   或不可执行 = Fail（漂移不静默穿透，提示 setup 收敛）；
/// - 清单缺失（bundle 新装）或无本条目 → 依次落 env 目录、PATH；
/// - 三层全空 = Fail：提示 setup / env 目录两条修复路径（仍无任何静默回落）。
///
/// gateway（pasta/slirp4netns）与 socks worker（tun2proxy）共用本门。
pub fn pinned_bin(name: &str) -> anyhow::Result<PathBuf> {
    let manifest = crate::manifest::read().map_err(|e| {
        anyhow!(
            "清单不可读（fail-loud #2）：{e}；修复或删除 {} 后重跑 `iso-cc setup`",
            crate::manifest::manifest_path().display()
        )
    })?;
    if let Some(m) = manifest {
        if let Some(entry) = m.find(crate::manifest::EntryKind::Provider, name) {
            let path = entry.path.clone().ok_or_else(|| {
                anyhow!("清单 {name} 条目缺 path（清单损坏）——重跑 `iso-cc setup` 收敛")
            })?;
            if !is_executable(&path) {
                anyhow::bail!(
                    "provider 钉定路径不可执行：{}（fail-loud #2）——重跑 `iso-cc setup` 收敛",
                    path.display()
                );
            }
            return Ok(path);
        }
    }
    if let Some(path) = env_dir_bin(name) {
        return Ok(path);
    }
    if let Some(path) = which_any(&bin_candidates(name)) {
        return Ok(path);
    }
    anyhow::bail!(
        "provider {name} 未解析（fail-loud #2）：优先级链 清单钉路径 > {HELPER_DIR_ENV} > PATH \
         全部落空——先运行 `iso-cc setup`（发行版原生包形态），或设 {HELPER_DIR_ENV} 指向 \
         bundle 解包目录的 libexec/（bundle 形态）"
    )
}

/// gateway 形态的钉定路径（[`pinned_bin`] 的 NetGateway 便捷封装）。
pub fn gateway_bin(gateway: NetGateway) -> anyhow::Result<PathBuf> {
    pinned_bin(gateway.bin_name())
}

/// fail-loud #12：`-I` 撞名断言——outbound 名与目标 ns 既有接口撞名类直接拒绝。
///
/// - `lo`：目标 netns 内恒存在的接口；outbound=lo 必然撞名（issue 10 §Answer 根因），
///   且语义上不可作 egress——宿主 loopback 可达由 pasta `--map-host-loopback` 默认提供。
/// - `tap0`（[`NS_IFNAME`]）：与 pasta `-I` 请求在 ns 内创建的 tap 同名。
pub fn validate_egress_iface(name: &str) -> anyhow::Result<()> {
    if name == "lo" {
        bail!("#12 egress 'lo' 被拒绝：loopback 恒存在于目标 netns（-I 撞名类），且语义上不可作 egress；宿主 loopback 可达由 pasta --map-host-loopback 默认提供");
    }
    if name == NS_IFNAME {
        bail!("#12 egress '{NS_IFNAME}' 被拒绝：与 pasta `-I {NS_IFNAME}` 的 ns 内 tap 名冲突");
    }
    Ok(())
}

/// fail-loud #3：宿主 egress 接口存在且 UP（sysfs `flags` 的 IFF_UP 位，spawn 前断言）。
/// R8：绝不回落。用 IFF_UP（管理态）而非 operstate——WireGuard 类隧道接口恒报 unknown。
/// 边界：sysfs 断言仅宿主侧合法；ns 内就绪断言走 [`crate::netcfg::wait_ready`]
/// （netlink，provider 无关）——09 取证 #5：ns 内 /sys/class/net 呈宿主视图伪影。
pub fn host_iface_up(name: &str) -> anyhow::Result<()> {
    let base = std::path::Path::new("/sys/class/net").join(name);
    if !base.exists() {
        bail!("#3 宿主 egress 接口 {name} 不存在（/sys/class/net）；R8：绝不回落");
    }
    let flags_raw = std::fs::read_to_string(base.join("flags"))
        .with_context(|| format!("#3 读取 /sys/class/net/{name}/flags"))?;
    let hex = flags_raw.trim().trim_start_matches("0x");
    let flags = i64::from_str_radix(hex, 16)
        .with_context(|| format!("#3 解析接口 {name} 的 flags（{flags_raw:?}）"))?;
    if flags & (libc::IFF_UP as i64) == 0 {
        bail!(
            "#3 宿主 egress 接口 {name} 非 UP（flags={flags_raw:?}，IFF_UP 未置位）；R8：绝不回落"
        );
    }
    Ok(())
}
