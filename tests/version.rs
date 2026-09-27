//! 工单 08 版本注入断言：`--version` 输出 == 构建期注入的 `ISO_CC_VERSION`，
//! 且注入值呈两种合法形态之一（git describe 全量版 / 无 git 回落 Cargo.toml）。
//!
//! 断言的是「注入管道端到端一致」，不钉具体版本号（版本号随 tag 漂移是有意行为）。

use std::process::Command;

#[test]
fn version_flag_matches_injected_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_iso-cc"))
        .arg("--version")
        .output()
        .expect("spawn iso-cc --version");
    assert!(out.status.success(), "--version 退出码非 0");
    let stdout = String::from_utf8(out.stdout).expect("--version 输出为 UTF-8");
    let injected = env!("ISO_CC_VERSION");
    assert_eq!(
        stdout.trim(),
        format!("iso-cc {injected}"),
        "--version 输出必须等于 build.rs 注入的 ISO_CC_VERSION"
    );
}

#[test]
fn injected_version_is_describe_or_pkg_fallback() {
    let injected = env!("ISO_CC_VERSION");
    let describe_form = injected.starts_with('v') && injected.len() > 1;
    let fallback_form = injected == env!("CARGO_PKG_VERSION");
    assert!(
        describe_form || fallback_form,
        "ISO_CC_VERSION={injected} 非法：应为 git describe（v 前缀 tag+commits+dirty）\
         或无 git 回落的 Cargo.toml 版本 {}",
        env!("CARGO_PKG_VERSION")
    );
}
