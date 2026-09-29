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

/// 目录内按候选名找可执行（env 目录与 exe-relative libexec 两层共用）。
fn dir_bin(dir: &Path, name: &str) -> Option<PathBuf> {
    bin_candidates(name).into_iter().find_map(|b| {
        let cand = dir.join(b);
        is_executable(&cand).then_some(cand)
    })
}

/// 第二层：`ISO_CC_HELPER_DIR` 指向的目录内按候选名找可执行。
fn env_dir_bin(name: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os(HELPER_DIR_ENV)?);
    dir_bin(&dir, name)
}

/// 第三层（工单 27）：exe-relative libexec —— `<exe_dir>/../lib/iso-cc/libexec`。
/// exe 路径先 canonicalize 解析符号链接：`PREFIX/bin/iso-cc` 即便被包管理器/用户
/// symlink 转发，也命中真实安装前缀的 libexec（对应 install.sh 布局
/// `PREFIX/bin/iso-cc` + `PREFIX/lib/iso-cc/libexec/`）。dev/nix 构建该层自然
/// 不存在，回落 PATH 无害。
fn exe_relative_bin(exe: &Path, name: &str) -> Option<PathBuf> {
    let resolved = std::fs::canonicalize(exe).ok()?;
    let exe_dir = resolved.parent()?;
    let dir = exe_dir.parent()?.join("lib/iso-cc/libexec");
    dir_bin(&dir, name)
}

/// manifest 之后的回落序（次序即优先级）：`ISO_CC_HELPER_DIR` > exe-relative
/// libexec > PATH。exe 路径由调用方传入（pinned_bin 给 current_exe；测试给伪造
/// 布局）。
fn fallback_after_manifest(exe: Option<&Path>, name: &str) -> Option<PathBuf> {
    env_dir_bin(name)
        .or_else(|| exe.and_then(|e| exe_relative_bin(e, name)))
        .or_else(|| which_any(&bin_candidates(name)))
}

/// provider 可执行解析门（工单 08 设计 #3 优先级链）：
/// **manifest 钉路径 > `ISO_CC_HELPER_DIR` env > `<exe_dir>/../lib/iso-cc/libexec` >
/// PATH**（对标 podman `CONTAINERS_HELPER_BINARY_DIR`；14 实现了首尾两层，
/// 08 补 env 层，27 补 install.sh bundle 安装布局层）。
///
/// - manifest 可读且有本 provider 条目 → 钉定路径唯一裁决：path 缺失（清单损坏）
///   或不可执行 = Fail（漂移不静默穿透，提示 setup 收敛）；
/// - 清单缺失（bundle 新装）或无本条目 → 依次落 env 目录、exe-relative libexec、
///   PATH（bundle 经 install.sh 装到 PREFIX 后零配置命中）；
/// - 四层全空 = Fail：提示 setup / install.sh / env 目录修复路径（仍无任何静默回落）。
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
    let exe = std::env::current_exe().ok();
    if let Some(path) = fallback_after_manifest(exe.as_deref(), name) {
        return Ok(path);
    }
    anyhow::bail!(
        "provider {name} 未解析（fail-loud #2）：优先级链 清单钉路径 > {HELPER_DIR_ENV} > \
         <exe_dir>/../lib/iso-cc/libexec > PATH 全部落空——`iso-cc setup`（发行版原生包形态）、\
         install.sh bundle 安装（PREFIX/bin + PREFIX/lib/iso-cc/libexec），或设 \
         {HELPER_DIR_ENV} 指向 bundle 解包目录的 libexec/"
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 伪造 install.sh 安装布局：`<root>/bin/iso-cc`（exe 占位）+
    /// `<root>/lib/iso-cc/libexec/<helper>`（可执行占位）。返回伪 exe 路径。
    fn fake_install_layout(root: &Path, helper: &str) -> PathBuf {
        let exe = root.join("bin/iso-cc");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"#!/bin/sh\n").unwrap();
        let helper_path = root.join("lib/iso-cc/libexec").join(helper);
        std::fs::create_dir_all(helper_path.parent().unwrap()).unwrap();
        std::fs::write(&helper_path, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(
            &helper_path,
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        exe
    }

    #[test]
    fn exe_relative_layer_hits_installed_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = fake_install_layout(tmp.path(), "pasta");
        assert_eq!(
            exe_relative_bin(&exe, "pasta").unwrap(),
            tmp.path().join("lib/iso-cc/libexec/pasta")
        );
    }

    #[test]
    fn exe_relative_layer_resolves_symlinked_exe() {
        let tmp = tempfile::tempdir().unwrap();
        // 真实安装前缀 opt/iso-cc-0.1.0；usr/local/bin/iso-cc 为 symlink 转发
        //（包管理器版本化目录形态）——canonicalize 后命中真实前缀的 libexec。
        let real = tmp.path().join("opt/iso-cc-0.1.0");
        let exe = fake_install_layout(&real, "slirp4netns");
        let link = tmp.path().join("usr/local/bin/iso-cc");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&exe, &link).unwrap();
        assert_eq!(
            exe_relative_bin(&link, "slirp4netns").unwrap(),
            real.join("lib/iso-cc/libexec/slirp4netns")
        );
    }

    #[test]
    fn exe_relative_layer_misses_without_layout() {
        let tmp = tempfile::tempdir().unwrap();
        // 无 libexec 目录 → None
        let exe = tmp.path().join("bin/iso-cc");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"").unwrap();
        assert!(exe_relative_bin(&exe, "pasta").is_none());
        // exe 无父前缀（根直下）→ None
        let flat = tmp.path().join("flat-iso-cc");
        std::fs::write(&flat, b"").unwrap();
        assert!(exe_relative_bin(&flat, "pasta").is_none());
    }

    #[test]
    fn env_layer_beats_exe_relative_then_path() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path().join("inst");
        let exe = fake_install_layout(&inst, "pasta"); // exe-relative 层可命中

        let env_dir = tmp.path().join("envdir");
        std::fs::create_dir_all(&env_dir).unwrap();
        let env_pasta = env_dir.join("pasta");
        std::fs::write(&env_pasta, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&env_pasta, std::fs::Permissions::from_mode(0o755)).unwrap();

        let path_dir = tmp.path().join("pathdir");
        std::fs::create_dir_all(&path_dir).unwrap();
        let path_pasta = path_dir.join("pasta");
        std::fs::write(&path_pasta, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path_pasta, std::fs::Permissions::from_mode(0o755)).unwrap();

        let old_env = std::env::var_os(HELPER_DIR_ENV);
        let old_path = std::env::var_os("PATH");
        std::env::set_var(HELPER_DIR_ENV, &env_dir);
        std::env::set_var("PATH", &path_dir);

        // 三层同时可命中：env 优先（票面优先级序断言）
        assert_eq!(
            fallback_after_manifest(Some(&exe), "pasta").unwrap(),
            env_pasta
        );

        // env 目录存在但无候选名 → 落 exe-relative（次优先）
        let empty_env = tmp.path().join("empty-env");
        std::fs::create_dir_all(&empty_env).unwrap();
        std::env::set_var(HELPER_DIR_ENV, &empty_env);
        assert_eq!(
            fallback_after_manifest(Some(&exe), "pasta").unwrap(),
            inst.join("lib/iso-cc/libexec/pasta")
        );

        // exe-relative 布局缺失 → PATH 兜底
        let bare = tmp.path().join("bare");
        let bare_exe = bare.join("bin/iso-cc");
        std::fs::create_dir_all(bare_exe.parent().unwrap()).unwrap();
        std::fs::write(&bare_exe, b"").unwrap();
        assert_eq!(
            fallback_after_manifest(Some(&bare_exe), "pasta").unwrap(),
            path_pasta
        );

        // 恢复进程全局 env（nextest 每 test 一进程本无此虑；cargo test 同进程跑时自愈）
        match old_env {
            Some(v) => std::env::set_var(HELPER_DIR_ENV, v),
            None => std::env::remove_var(HELPER_DIR_ENV),
        }
        match old_path {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
    }
}
