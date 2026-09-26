//! 网关 provider 层（设计稿 §1/§5）：pasta spawn 转正 + slirp4netns 回退。
//! argv 展开是 plan 打印与实际 spawn 的单一事实源（§7 D5：`--print-plan` 终点 = 等价 CLI 组合）。

pub mod pasta;
pub mod slirp;

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
        anyhow::bail!("{} --version 无 stdout（版本事实缺失，fail-loud）", bin.display());
    }
    Ok(v)
}

/// fail-loud #2（票 14 清单态，替换 09 期过渡态 which 解析）：所选 gateway 的可执行
/// 从清单钉定（绝对路径 + 版本登记）。清单缺失 / 条目缺失 / 路径不可执行 = Fail，
/// 提示 setup；run/doctor 的会话上树一律消费钉定路径（稀疏 PATH 生效的前提）。
pub fn gateway_bin(gateway: NetGateway) -> anyhow::Result<PathBuf> {
    let name = gateway.bin_name();
    let manifest = crate::manifest::read().map_err(|e| {
        anyhow!("清单不可读（fail-loud #2）：{e}；修复或删除 {} 后重跑 `iso-cc setup`",
            crate::manifest::manifest_path().display())
    })?;
    let Some(m) = manifest else {
        anyhow::bail!(
            "清单缺失（fail-loud #2）：net.gateway={gateway} 绝对路径未钉定——先运行 `iso-cc setup`"
        )
    };
    let entry = m.find(crate::manifest::EntryKind::Provider, name).ok_or_else(|| {
        anyhow!(
            "清单无 {name} 条目（fail-loud #2）：net.gateway={gateway} 未登记——先运行 `iso-cc setup --profile <n>`"
        )
    })?;
    let path = entry.path.clone().ok_or_else(|| {
        anyhow!("清单 {name} 条目缺 path（清单损坏）——重跑 `iso-cc setup` 收敛")
    })?;
    let executable = std::fs::metadata(&path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false);
    if !executable {
        anyhow::bail!(
            "provider 钉定路径不可执行：{}（fail-loud #2）——重跑 `iso-cc setup` 收敛",
            path.display()
        );
    }
    Ok(path)
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
