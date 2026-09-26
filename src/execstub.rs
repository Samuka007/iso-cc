//! execstub.rs —— multi-call stub 转发模式（票 15 / ADR 0008 附 2）。
//!
//! 同一二进制以 argv0 basename == `bash` 被调用 = 转发模式：连 `ISO_CC_EXEC_SOCK`，
//! 发 `{script, cwd, env}` + SCM_RIGHTS(0,1,2)，回传退出码/信号后以其退出。
//! 「bind 上去的是 iso-cc 自身；无独立 shim 文件、无递归（转发执行发生在宿主侧
//! mountns 外）」——本文件即该裁决的实现面。
//!
//! cc 的三种调用形态（ADR 0008 附 / 附 3 取证 + 官方 env-vars 文档）：
//! - L1（`SHELL=<shim>`）：`$SHELL -c <整段脚本>` → `[shim, "-c", script]`
//! - L2（PATH shim）：PATH 解析到 shim 的 `bash -c <script>` → 同上
//! - L1.5（`CLAUDE_CODE_SHELL_PREFIX=<shim>`）：prefix 以「完整组装调用串」单载荷
//!   传入（官方文档：命令行整体作为 $1）→ `[shim, "<invocation string>"]`
//!
//! fail-loud 纪律：任何通道失败 = stderr 报告 + 退出 126，绝不回落本地 bash
//! （fail-open 会静默破坏 `exec.bash=host` 的声明语义）。

use crate::execrpc::{read_frame, send_request, ExecRequest, ExecResponse};
use std::ffi::{OsStr, OsString};
use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::path::Path;

/// multi-call 判别键（附 2：argv0 basename == `bash` → 转发模式）。
pub const STUB_BASENAME: &str = "bash";

pub fn is_stub_invocation(argv0: Option<&OsStr>) -> bool {
    match argv0 {
        Some(a) => Path::new(a).file_name().is_some_and(|b| b == STUB_BASENAME),
        None => false,
    }
}

/// 从 argv（已去 argv0）解析转发脚本。见模块注释的三形态；无法识别 = Err（126）。
pub fn parse_script(args: &[OsString]) -> Result<OsString, String> {
    // L1/L2 形态：取最后一个 `-c`（容忍 `-l -c` 一类 flag 前置），其后恰一个参数。
    if let Some(i) = args.iter().rposition(|a| a == "-c") {
        if args.len() == i + 2 {
            return Ok(args[i + 1].clone());
        }
        return Err(format!(
            "`-c` 后参数数异常（{}）：仅接受单段脚本",
            args.len() - i - 1
        ));
    }
    // L1.5 形态：单载荷 = cc 组装的完整调用串（整体转宿主执行，含其内嵌 bash -c）。
    if args.len() == 1 {
        return Ok(args[0].clone());
    }
    Err(format!(
        "无法识别的调用形态（{} 个参数）：期望 `bash -c <script>` 或 prefix 单载荷",
        args.len()
    ))
}

/// 转发模式主体：不返回（exit）。
pub fn forward() -> ! {
    std::process::exit(forward_code())
}

fn err126(msg: &str) -> i32 {
    let _ = std::io::stderr().write_all(format!("[iso-cc exec stub] {msg}\n").as_bytes());
    126
}

fn forward_code() -> i32 {
    let Some(sock) = std::env::var_os("ISO_CC_EXEC_SOCK") else {
        return err126(
            "ISO_CC_EXEC_SOCK 未设置——bash stub 在无 exec 通道的上下文中被调用（配置与注入不一致）",
        );
    };
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let script = match parse_script(&args) {
        Ok(s) => s,
        Err(e) => return err126(&e),
    };
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return err126(&format!("cwd 不可解析: {e}")),
    };
    // JSON 协议 = UTF-8 域；非 UTF-8 字节按 lossy 收敛（cc 侧 script 本为 JS 字符串）。
    let req = ExecRequest {
        script: script.to_string_lossy().into_owned(),
        cwd: cwd.to_string_lossy().into_owned(),
        // v1 恒空表：env 策略归 server（宿主基底 + locale 注入，ADR 附）。
        env: Default::default(),
    };
    let mut stream = match UnixStream::connect(&sock) {
        Ok(s) => s,
        Err(e) => {
            return err126(&format!(
                "连 exec.sock 失败（{}）: {e}",
                Path::new(&sock).display()
            ))
        }
    };
    if let Err(e) = send_request(&stream, &req, &[0, 1, 2]) {
        return err126(&format!("发送请求失败: {e}"));
    }
    let resp = match read_frame(&mut stream) {
        Ok(r) => r,
        Err(e) => return err126(&format!("读取响应失败: {e}")),
    };
    match resp {
        ExecResponse::Code { code } => code,
        // bash 对致命信号的退出约定：128 + sig。
        ExecResponse::Signal { signal } => 128 + signal,
        ExecResponse::Error { error } => err126(&error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn argv0_basename_discriminates() {
        assert!(is_stub_invocation(Some("/x/.local/state/iso-cc/sessions/s1/bin/bash".as_ref())));
        assert!(is_stub_invocation(Some("/x/shims/bash".as_ref())));
        assert!(is_stub_invocation(Some("/usr/bin/bash".as_ref())));
        assert!(!is_stub_invocation(Some("/x/target/debug/iso-cc".as_ref())));
        assert!(!is_stub_invocation(Some("bashful".as_ref())));
        assert!(!is_stub_invocation(None));
    }

    #[test]
    fn parse_l1_l2_shape() {
        assert_eq!(parse_script(&os(&["-c", "echo hi"])).unwrap(), "echo hi");
        // flag 前置（-l login shell 类）：取最后一个 -c
        assert_eq!(parse_script(&os(&["-l", "-c", "true"])).unwrap(), "true");
        assert!(parse_script(&os(&["-c"])).is_err(), "-c 后无脚本 = Err");
        assert!(parse_script(&os(&["-c", "a", "b"])).is_err(), "-c 后多参 = Err");
    }

    #[test]
    fn parse_l15_prefix_single_payload_shape() {
        // 官方 prefix 语义：完整调用串单载荷传入
        let payload = "/bin/bash -c 'npm test' 2>&1";
        assert_eq!(
            parse_script(&os(&[payload])).unwrap(),
            payload,
            "单载荷 = 完整调用串整体转宿主"
        );
    }

    #[test]
    fn parse_unrecognized_shape_rejected() {
        assert!(parse_script(&os(&[])).is_err(), "零参数 = Err（126）");
        assert!(parse_script(&os(&["a", "b"])).is_err(), "双裸参数非任何已知形态");
    }
}
