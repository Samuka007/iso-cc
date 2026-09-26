//! 网关 provider 层（设计稿 §1/§5）：pasta spawn 转正 + slirp4netns 回退。
//! argv 展开是 plan 打印与实际 spawn 的单一事实源（§7 D5：`--print-plan` 终点 = 等价 CLI 组合）。

pub mod pasta;
pub mod slirp;

use crate::config::NetGateway;
use anyhow::{anyhow, bail, Context};
use std::path::PathBuf;

/// pasta `-I` 目标 ns 内 tap 名（issue 10 §Answer 硬规则：恒显式指定，
/// 绝不回落「默认取 outbound 接口名」的命名规则）。
pub const NS_IFNAME: &str = "tap0";

pub(crate) fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|cand| cand.is_file())
}

/// fail-loud #2（09 期过渡态）：所选 gateway 的可执行 which 解析，缺失显式报错。
/// 清单化定位（绝对路径 + 版本钉定）归票 14。
pub fn gateway_bin(gateway: NetGateway) -> anyhow::Result<PathBuf> {
    let name = gateway.bin_name();
    which(name).ok_or_else(|| {
        anyhow!("net.gateway={gateway}：{name} 不在 PATH（fail-loud #2；先运行 `iso-cc doctor` 检查 provider）")
    })
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
