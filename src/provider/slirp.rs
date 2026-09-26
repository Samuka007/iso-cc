//! slirp4netns 回退 provider（设计稿 §1.3 下树）：attach + `-c` self-config。

use std::ffi::OsString;

/// attach 实参：`<bootstrap-pid> tap0 -c`。
/// `-c` 自配 lo/tap/MTU/IP/默认路由（slirp4netns main.c:149-227），bootstrap 期零配网命令。
pub fn attach_args(pid: u32) -> Vec<OsString> {
    vec![
        pid.to_string().into(),
        crate::provider::NS_IFNAME.into(),
        "-c".into(),
    ]
}
