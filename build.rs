//! 版本注入（工单 08 设计 #1）：git tag（`v*`）为版本单一事实源。
//!
//! - `git describe --tags --dirty` → `ISO_CC_VERSION`（编译期 env）。
//!   clap `#[command(version)]` 由 clap_derive 在 rustc 进程环境展开
//!   `CARGO_PKG_VERSION`——本脚本以 `cargo:rustc-env=CARGO_PKG_VERSION=…`
//!   覆盖之（实测 proc-macro 与 env! 同读 rustc 进程环境，覆盖对两者同时生效），
//!   因此 `iso-cc --version` 输出 tag+commits+dirty 全量版本，`main.rs` 零改动。
//! - 同步校验（fail-loud）：存在 tag 时，describe 基线版本必须等于 Cargo.toml
//!   `version`，否则构建失败——tag v* 与 Cargo.toml 必须同步 bump。
//! - 无 git 环境（nix tarball / crane cleanCargoSource 剥离 `.git`）或仓库尚无
//!   tag：回落 Cargo.toml 版本，绝不失败（nix 构建必须可走通）。
//!
//! 故意不写 `cargo:rerun-if-*`：保留 cargo 默认「包内任一文件变更即重跑」；
//! 纯 `.git` 变化不触发重跑，tag 发版的正式路径（release workflow）为全新
//! checkout，天然带新鲜 describe，无陈旧窗口。

use std::process::Command;

fn main() {
    let pkg_version = std::env::var("CARGO_PKG_VERSION").expect("cargo 恒注入 CARGO_PKG_VERSION");
    match git_describe() {
        Some(desc) => {
            // describe 形态：vX.Y.Z / vX.Y.Z-N-g<hash> / 尾缀 -dirty。
            // 基线 = 去 v 前缀后首个 `-` 之前的部分，与 Cargo.toml 严格比对。
            let base = desc.strip_prefix('v').unwrap_or(&desc);
            let base = base.split('-').next().unwrap_or(base);
            assert_eq!(
                base, pkg_version,
                "版本不一致（fail-loud，工单 08 #1）：git tag 基线 {base} != Cargo.toml \
                 version {pkg_version}——tag v* 与 Cargo.toml version 必须同步 bump（发版清单：\
                 ①bump Cargo.toml ②git tag v<version>，两步缺一即此错）"
            );
            emit(&desc);
        }
        None => emit(&pkg_version),
    }
}

/// `git describe --tags --dirty`（只读；工作目录 = Cargo.toml 所在目录）。
/// 非 git 仓库 / 无 tag / git 不可用 → None（回落路径）。
fn git_describe() -> Option<String> {
    let out = Command::new("git")
        .args(["describe", "--tags", "--dirty"])
        .current_dir(std::env::var("CARGO_MANIFEST_DIR").ok()?)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn emit(version: &str) {
    // ISO_CC_VERSION：面向测试与人工断言的显式注入面。
    println!("cargo:rustc-env=ISO_CC_VERSION={version}");
    // 覆盖 clap_derive 展开期读取的 CARGO_PKG_VERSION（见模块注释）。
    println!("cargo:rustc-env=CARGO_PKG_VERSION={version}");
}
