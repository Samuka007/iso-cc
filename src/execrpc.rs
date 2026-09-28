//! execrpc.rs —— exec RPC 通道 server 端（票 15 / ADR 0008 附、附 2）。
//!
//! 传输（票 15 spec#1）：parent bind `<state>/<sess>/exec.sock`（unix socket，共享 fs
//! 可达；0600）；协议 = 单条 `sendmsg`：4B LE 长度 + JSON `{script, cwd, env}` +
//! SCM_RIGHTS(stdin, stdout, stderr)。server 侧 fork 宿主侧 `/bin/bash -c <script>`
//! （cwd 同路径零映射；env = 宿主基底 + locale 注入项 + 会话标记，ADR 附「env =
//! 宿主原 PATH + 注入 locale 项」），stdio 由 SCM_RIGHTS 直通（fd 级零拷贝，实现
//! 正本「stdio 双向泵」的观测语义且少两条泵线程）；退出码/信号经控制流回传。
//!
//! SIGINT/中断转发（spec#1）：cc 击杀 stub（或 stub 随 cc 死亡）→ socket EOF →
//! watcher 线程 SIGKILL 宿主 worker；终端前台 Ctrl-C 命中 iso-cc 进程组 = worker 同组，
//! 由终端直接送达。宿主 worker 带 ISO_CC_SESSION 标记（spec#4「会话标记 = 13 收割面」）：
//! Session::wait 收尾 shutdown → kill 在途 worker（后台任务存续至会话结束）→
//! reap_adopted 收编已 reparent 的标记后裔（worker 的后台子进程）。
//!
//! 本模块零 unsafe 词法：fd 所有权经 `OwnedFd::from(RawFd)`（safe，取得所有权），
//! dup2 经 nix 安全封装，pre_exec 安装收敛于 [`crate::ns::install_pre_exec`]。

use crate::config::Profile;
use crate::ns;
use anyhow::Context as _;
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::io::{self, IoSlice, IoSliceMut, Read, Write};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use nix::sys::socket::{recvmsg, sendmsg, ControlMessage, ControlMessageOwned, MsgFlags, UnixAddr};
/// 宿主侧执行体默认值（票 15 spec#1 正本措辞：`/bin/bash -c`）。
const HOST_SHELL: &str = "/bin/bash";

/// 宿主侧执行体解析（票 21 测试支撑面）：env `ISO_CC_HOST_SHELL` 注入时覆盖，
/// 未设 = 正本 [`HOST_SHELL`]，运行时语义零变化。nix 沙箱无 FHS `/bin/bash`
/// 路径，测试（[`tests::test_host_shell`]）据此注入沙箱内实际可用的 bash/sh。
fn host_shell() -> PathBuf {
    std::env::var_os("ISO_CC_HOST_SHELL")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(HOST_SHELL))
}

/// 帧上限：脚本为 cc Bash 工具的整段 argv（ADR 附「拦截粒度 = bash -c 整段 argv」），
/// 2 MiB 覆盖全部合理负载；超界 = 协议错误（fail-loud）。
pub(crate) const FRAME_MAX: usize = 2 * 1024 * 1024;

/// 请求（协议正本形态 `{script, cwd, env}`）。deny_unknown_fields = #5 纪律。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecRequest {
    pub script: String,
    pub cwd: String,
    /// 每请求 env 覆盖（键值）。v1 stub 恒发空表：env 策略归 server 侧（宿主基底 +
    /// locale 注入，ADR 附）；字段按协议正本保留，供未来 per-call 覆盖。
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// argv 直通模式（票 26 spec#3）：`Some(argv)` = 宿主侧按原 argv execvp（宿主
    /// PATH 解析；首元素 = 程序名）。`None` = 正本 `HOST_SHELL -c script`。stub 的
    /// shell 形态（argv0 ∈ {bash, sh}）恒 `None`。同 binary 同 wire，无跨版本面。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argv: Option<Vec<String>>,
}

/// 响应：退出码 | 致命信号 | 通道错误。internally-tagged（`{"status":"code","code":7}`）：
/// untagged 变体不尊重 deny_unknown_fields（serde#1600），tagged 形态保住 #5 fail-loud
/// 纪律且 wire 自描述。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "status", rename_all = "lowercase", deny_unknown_fields)]
pub enum ExecResponse {
    Code { code: i32 },
    Signal { signal: i32 },
    Error { error: String },
}

/// 写一帧（4B LE 长度 + JSON）。
pub(crate) fn write_frame(s: &mut UnixStream, resp: &ExecResponse) -> io::Result<()> {
    let body =
        serde_json::to_vec(resp).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    s.write_all(&(body.len() as u32).to_le_bytes())?;
    s.write_all(&body)
}

/// 读一帧（stub 与测试共用）。
pub(crate) fn read_frame(s: &mut UnixStream) -> io::Result<ExecResponse> {
    let mut len_buf = [0u8; 4];
    s.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > FRAME_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("响应帧超界：{len} > {FRAME_MAX}"),
        ));
    }
    let mut body = vec![0u8; len];
    s.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// 发送请求 + stdio fd（SCM_RIGHTS）。stub 调用形态传 `[0, 1, 2]`。
pub(crate) fn send_request(s: &UnixStream, req: &ExecRequest, fds: &[RawFd]) -> io::Result<()> {
    let body =
        serde_json::to_vec(req).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = (body.len() as u32).to_le_bytes();
    let iov = [IoSlice::new(&len), IoSlice::new(&body)];
    sendmsg::<()>(
        s.as_raw_fd(),
        &iov,
        &[ControlMessage::ScmRights(fds)],
        MsgFlags::empty(),
        None,
    )
    .map_err(io::Error::from)?;
    Ok(())
}

/// 宿主 worker env 注入项（纯函数，可单测）。镜像 session::apply_env 的 locale/显式 env
/// 语义（ADR 附：「env = 宿主原 PATH + 注入 locale 项」——基底 env 由 server 进程自带，
/// 此处只给注入项）+ 会话标记（spec#4）+ 通道指针（票 25 G1b）。
pub(crate) fn worker_env_vars(
    profile: &Profile,
    session_id: &str,
    sock_path: &Path,
) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Some(tz) = &profile.locale.tz {
        m.insert("TZ".into(), tz.clone());
    }
    if let Some(lang) = &profile.locale.lang {
        m.insert("LANG".into(), lang.clone());
        m.insert("LC_ALL".into(), lang.clone());
    }
    for (k, v) in &profile.env {
        m.insert(k.clone(), v.clone());
    }
    // 会话标记：宿主侧派生进程入 13 收割面（marked → teardown SIGKILL + reap；
    // 后台任务存续至会话结束，票 15 spec#4）。
    m.insert("ISO_CC_SESSION".into(), session_id.to_string());
    // 通道指针（票 25 G1b）：Bash 工具/hooks/statusline 的载荷 = Yhe 产物
    // `'<shim>' '<script>'`，宿主 worker bash 执行它时第二跳再进 shim（L1.5
    // 单载荷形态）——第二跳 stub 依赖继承的 ISO_CC_EXEC_SOCK 经同一通道转发，
    // 一次额外 RPC 跳，终态语义不变（脚本仍由宿主 `/bin/bash -c` 执行）。
    m.insert(
        "ISO_CC_EXEC_SOCK".into(),
        sock_path.to_string_lossy().into_owned(),
    );
    m
}

/// worker env 注入项的线程共享形态（serve 前一次冻结，避免 Profile 生命周期跟随线程）。
#[derive(Debug, Clone)]
struct WorkerSpec {
    vars: Arc<BTreeMap<String, String>>,
}

/// 已装配未启动的通道（spawn 早期调用 [`prepare`]，网关 spawn 成功后调
/// [`ExecChannel::serve`]）。
pub struct ExecChannel {
    /// L1/L1.5 注入的 shell 路径 = `<sess>/bin/bash`（multi-call shim 符号链接 →
    /// iso-cc；票 26 起 env 值面恒写经典路径 `/bin/bash`，经 L3 bind 即本 shim）。
    pub shell_path: PathBuf,
    /// L2 PATH shim 目录 = `<sess>/bin`。
    pub bin_dir: PathBuf,
    /// RPC socket 路径（ISO_CC_EXEC_SOCK 注入值）。
    pub sock_path: PathBuf,
    listener: UnixListener,
    spec: Arc<WorkerSpec>,
}

/// 运行中的 server（accept 循环 + 在途 worker 登记簿）。
pub struct ExecServer {
    sock_path: PathBuf,
    stop: Arc<AtomicBool>,
    registry: Arc<Mutex<Vec<u32>>>,
    accept: JoinHandle<()>,
}

/// 装配（fallible，须在网关 spawn 前完成——env 注入需要 shell/sock 路径）：
/// ① `<sess>/bin/bash` 符号链接 → iso-cc 自身（附 2：bind 上去的是 iso-cc 自身，
/// argv0 basename 判别；无独立 shim 文件）；② bind `exec.sock`：0600（netns，
/// 附⑦：同 uid 任意宿主执行面的最小暴露面）| 0666（mark，票 18：会话树 uid 4210
/// 对宿主 0600 sock connect EACCES——文件位让位于 accept 期 SO_PEERCRED 凭证
/// 白名单（[`peer_allowed_for`]），强制力不降反升）。
pub fn prepare(
    sess_dir: &Path,
    profile: &Profile,
    session_id: &str,
    cross_uid: bool,
) -> anyhow::Result<ExecChannel> {
    let exe = std::env::current_exe().context("current_exe 不可用")?;
    let bin_dir = sess_dir.join("bin");
    std::fs::create_dir_all(&bin_dir)
        .with_context(|| format!("创建 L2 shim 目录 {}", bin_dir.display()))?;
    let shell_path = bin_dir.join("bash");
    if cross_uid {
        // mark（票 18）：uid 4210 对宿主 $HOME 链路不可达（0700 实测）——symlink
        // 解析路径必须可 traverse——shim 指向 mark 根的供给二进制（内容 diff
        // 供给，幂等；会话作用域：symlink 随 sess_dir 在 Session::wait 回收）。
        let shim_bin = crate::mark::ensure_shim_binary()?;
        std::os::unix::fs::symlink(&shim_bin, &shell_path).with_context(|| {
            format!(
                "创建 multi-call shim 符号链接 {} -> {}",
                shell_path.display(),
                shim_bin.display()
            )
        })?;
    } else {
        std::os::unix::fs::symlink(&exe, &shell_path).with_context(|| {
            format!(
                "创建 multi-call shim 符号链接 {} -> {}",
                shell_path.display(),
                exe.display()
            )
        })?;
    }
    // 票 26：`sh` 同 shim（hooks/REPL 硬编码 /bin/sh → L3 bind 面）。
    let sh_path = bin_dir.join("sh");
    let shim_src = if cross_uid {
        crate::mark::ensure_shim_binary()?
    } else {
        exe.clone()
    };
    std::os::unix::fs::symlink(&shim_src, &sh_path).with_context(|| {
        format!(
            "创建 multi-call shim 符号链接 {} -> {}",
            sh_path.display(),
            shim_src.display()
        )
    })?;
    let sock_path = sess_dir.join("exec.sock");
    // 防御性清理（同 id 重跑不发生——id 含纳秒；崩溃残留交 13 sweep）。
    let _ = std::fs::remove_file(&sock_path);
    let listener = UnixListener::bind(&sock_path)
        .with_context(|| format!("bind exec.sock {}", sock_path.display()))?;
    let sock_mode = if cross_uid { 0o666 } else { 0o600 };
    std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(sock_mode))
        .with_context(|| format!("chmod {sock_mode:o} {}", sock_path.display()))?;
    let vars = Arc::new(worker_env_vars(profile, session_id, &sock_path));
    Ok(ExecChannel {
        shell_path,
        bin_dir,
        sock_path,
        listener,
        spec: Arc::new(WorkerSpec { vars }),
    })
}

/// 对端凭证白名单（纯函数，可单测）：宿主 iso-cc 自身（shutdown dummy-connect +
/// netns 树经 pasta/slirp 映射后的对端 uid = 宿主发起者）∪ mark uid（票 18：
/// 会话树 uid 4210 的 shim → exec.sock）。其余本地用户一律拒绝（强制点在
/// accept 期，文件位只是第一道）。
fn peer_allowed_for(peer_uid: u32, euid: u32) -> bool {
    peer_uid == euid || peer_uid == crate::config::MARK_UID
}

impl ExecChannel {
    /// 启动 accept 循环（启动本身不失败；网关 spawn 成功后调用）。
    pub fn serve(self) -> ExecServer {
        let stop = Arc::new(AtomicBool::new(false));
        let registry: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
        let sock_path = self.sock_path.clone();
        let stop2 = stop.clone();
        let reg2 = registry.clone();
        let accept = thread::spawn(move || {
            let listener = self.listener;
            while !stop2.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if stop2.load(Ordering::SeqCst) {
                            break;
                        }
                        let spec = self.spec.clone();
                        let reg = reg2.clone();
                        thread::spawn(move || handle_conn(stream, spec, reg));
                    }
                    Err(e) => {
                        eprintln!(
                            "[iso-cc] exec-rpc accept 失败，通道关闭（后续会话内 bash 调用将报 126）: {e}"
                        );
                        break;
                    }
                }
            }
        });
        ExecServer {
            sock_path,
            stop,
            registry,
            accept,
        }
    }
}

impl ExecServer {
    /// 会话收尾（Session::wait 序：root wait → 本函数 → reap_adopted）：
    /// 停 accept（dummy-connect 唤醒）→ kill 在途 worker（spec#4：后台任务存续至
    /// 会话结束，不越界）→ 清 socket 文件。worker 的后台子进程（已 reparent 到
    /// subreaper、带标记）交 reap_adopted 收编。
    pub fn shutdown(self) {
        self.stop.store(true, Ordering::SeqCst);
        // cc 已死（root.wait 返回）→ 无新 stub 连接；本连接仅用于唤醒 accept。
        let _ = UnixStream::connect(&self.sock_path);
        let _ = self.accept.join();
        for pid in self.registry.lock().drain(..) {
            if ns::kill_pid(pid, libc::SIGKILL) {
                eprintln!("[iso-cc] exec-rpc teardown: 在途 worker pid={pid} → SIGKILL");
            }
        }
        let _ = std::fs::remove_file(&self.sock_path);
    }
}

/// 单连接处理：收 fd + 请求 → 宿主侧 fork 宿主 shell `-c`（正本 `/bin/bash`，
/// 票 21 测试支撑面 env `ISO_CC_HOST_SHELL` 可覆盖；dup2 接管 stdio）→
/// waiter（wait + 回传状态）+ watcher（EOF/中断 → SIGKILL）双线程。
fn handle_conn(mut stream: UnixStream, spec: Arc<WorkerSpec>, registry: Arc<Mutex<Vec<u32>>>) {
    // accept 期凭证白名单（票 18）：0666 文件位下任何本地用户可 connect，
    // 非白名单 uid 在此被拒（无执行面；0666 仅 mark 引擎会话树需要）。
    let peer_uid =
        nix::sys::socket::getsockopt(&stream, nix::sys::socket::sockopt::PeerCredentials)
            .map(|c| c.uid())
            .unwrap_or(u32::MAX);
    if !peer_allowed_for(peer_uid, unsafe { libc::geteuid() }) {
        eprintln!("[iso-cc] exec-rpc: 对端 uid={peer_uid} 不在白名单（宿主发起者/mark uid）——拒绝");
        return;
    }
    let (fds, req) = match recv_request(&mut stream) {
        Ok(x) => x,
        Err(e) => {
            // shutdown dummy-connect 落到此处的 EOF 亦走本出口（无副作用）。
            eprintln!("[iso-cc] exec-rpc 请求读取失败: {e}");
            return;
        }
    };
    if !Path::new(&req.cwd).is_dir() {
        let msg = format!("cwd 不存在（共享 fs 同路径零映射被破坏）: {}", req.cwd);
        eprintln!("[iso-cc] exec-rpc: {msg}");
        let _ = write_frame(&mut stream, &ExecResponse::Error { error: msg });
        return;
    }

    // 票 26：argv 直通（Some(argv)）= 宿主 PATH execvp 原形态；None = 正本
    // `HOST_SHELL -c script`（shell 形态，票 15 spec#1）。
    let shell = host_shell();
    let mut cmd = match &req.argv {
        Some(argv) if !argv.is_empty() => {
            let mut c = std::process::Command::new(&argv[0]);
            c.args(&argv[1..]);
            c
        }
        _ => {
            let mut c = std::process::Command::new(&shell);
            c.arg("-c").arg(&req.script);
            c
        }
    };
    cmd.current_dir(&req.cwd)
        // stdio 由 pre_exec dup2 接管（fd 所有权随闭包存活至 exec）
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.envs(spec.vars.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    // 每请求覆盖（v1 stub 恒空表；置于 spec 之后 = 覆盖语义）。
    cmd.envs(req.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let [f0, f1, f2] = fds;
    ns::install_pre_exec(&mut cmd, move || {
        ns::adopt_stdio(&[f0.as_raw_fd(), f1.as_raw_fd(), f2.as_raw_fd()])
    });
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("宿主侧 spawn {} 失败: {e}", shell.display());
            eprintln!("[iso-cc] exec-rpc: {msg}");
            let _ = write_frame(&mut stream, &ExecResponse::Error { error: msg });
            return;
        }
    };
    let pid = child.id();
    registry.lock().push(pid);

    // watcher（可降级）：MSG_PEEK 阻塞探测（不消费流字节）——EOF/错误 = 对端已死
    // （cc 击杀 stub / stub 随 cc 退出）→ SIGKILL worker（中断转发，spec#1）。正常
    // 完成路径 waiter 先 unregister，watcher 的登记簿校验拦住 pid 复用误杀。
    if let Ok(peer) = stream.try_clone() {
        let reg = registry.clone();
        thread::spawn(move || {
            // std UnixStream::peek 为 nightly（unix_socket_peek）；nix recv + MSG_PEEK
            // 同语义（不消费流字节，阻塞至首字节/EOF）。
            let fd = peer.as_raw_fd();
            let mut b = [0u8; 1];
            let interrupted = loop {
                match nix::sys::socket::recv(fd, &mut b, MsgFlags::MSG_PEEK) {
                    Ok(0) => break true,
                    Ok(_) => break false, // 响应字节已在队列（waiter 已 unregister）= 正常完成
                    Err(nix::errno::Errno::EINTR) => continue,
                    Err(_) => break true,
                }
            };
            if interrupted && take_from_registry(&reg, pid) {
                ns::kill_pid(pid, libc::SIGKILL);
                eprintln!("[iso-cc] exec-rpc: 对端断开 → 中断转发 worker pid={pid}（SIGKILL）");
            }
        });
    } else {
        eprintln!("[iso-cc] exec-rpc: peer 克隆失败，watcher 缺席（中断转发降级为 teardown）");
    }

    // waiter：wait → 先 unregister 再回传（顺序保证 watcher 不对已完成 worker 误杀）。
    if let Ok(mut out) = stream.try_clone() {
        thread::spawn(move || {
            let resp = finish_worker(&mut child);
            take_from_registry(&registry, pid);
            // EPIPE（stub 已死）静默：状态无处可投，worker 已收尾。
            let _ = write_frame(&mut out, &resp);
        });
    } else {
        // 响应无法投递（stub 侧必然已死）：内联收尾，不留 zombie。
        let _ = finish_worker(&mut child);
        take_from_registry(&registry, pid);
    }
}

/// wait worker → 响应态映射（signal 优先于 code，bash 语义 128+sig 由 stub 侧换算）。
fn finish_worker(child: &mut std::process::Child) -> ExecResponse {
    match child.wait() {
        Ok(st) => match st.signal() {
            Some(sig) => ExecResponse::Signal { signal: sig },
            None => ExecResponse::Code {
                code: st.code().unwrap_or(1),
            },
        },
        Err(e) => ExecResponse::Error {
            error: format!("wait worker 失败: {e}"),
        },
    }
}

/// 收单连接：首条 recvmsg 取 SCM_RIGHTS（附着于首字节）+ 长度前缀段，补齐后读 JSON。
fn recv_request(stream: &mut UnixStream) -> io::Result<([OwnedFd; 3], ExecRequest)> {
    let mut len_buf = [0u8; 4];
    let mut iov = [IoSliceMut::new(&mut len_buf)];
    #[allow(unused_imports)]
    use nix::cmsg_space; // #[macro_export] → crate 根；同名的 doc-hidden fn 返回 usize（空间字节数）
    let mut cmsg_buf = cmsg_space!([RawFd; 3]);
    let msg = recvmsg::<UnixAddr>(
        stream.as_raw_fd(),
        &mut iov,
        Some(cmsg_buf.as_mut_slice()),
        MsgFlags::empty(),
    )
    .map_err(io::Error::from)?;
    if msg.bytes == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "对端在请求前关闭",
        ));
    }
    let mut fds: Vec<OwnedFd> = Vec::new();
    for c in msg.cmsgs().map_err(io::Error::from)? {
        if let ControlMessageOwned::ScmRights(v) = c {
            // ns::own_scm_rights_fd：接收方取得所有权（RAII close，泄漏面收编）。
            fds.extend(v.into_iter().map(ns::own_scm_rights_fd));
        }
    }
    if fds.len() != 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("SCM_RIGHTS 数量异常：{}（期望 3 = stdio 三 fd）", fds.len()),
        ));
    }
    // 长度前缀可能被截在首条消息之后（iov 上限 4B），补齐。
    let mut got = msg.bytes;
    while got < 4 {
        let n = stream.read(&mut len_buf[got..])?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "长度前缀残缺"));
        }
        got += n;
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > FRAME_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("请求帧超界：{len} > {FRAME_MAX}"),
        ));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    let req: ExecRequest =
        serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    // Vec<OwnedFd> → [OwnedFd; 3]（保序：stdin, stdout, stderr）。
    let arr: [OwnedFd; 3] = fds.try_into().expect("长度已在上方断言为 3");
    Ok((arr, req))
}

/// 登记簿原子取出（false = 已被 waiter/teardown 处理 → 调用方不得再 kill，防 pid 复用误杀）。
fn take_from_registry(reg: &Mutex<Vec<u32>>, pid: u32) -> bool {
    let mut g = reg.lock();
    match g.iter().position(|p| *p == pid) {
        Some(i) => {
            g.swap_remove(i);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 票 21：nix 沙箱无 FHS `/bin/bash`——宿主 shell 解析序：
    /// ① env `ISO_CC_HOST_SHELL` 显式注入（CI/nix 构建面定向）；
    /// ② 正本 `/bin/bash`（宿主常态）；
    /// ③ PATH 探测 `bash` → `sh`（沙箱内 = nix store bash）。
    /// 全部落空 = None → 调用方按 fail-loud 语义跳过（eprintln 注明原因，
    /// 非静默通过）。
    fn test_host_shell() -> Option<PathBuf> {
        let mut cands: Vec<PathBuf> = Vec::new();
        if let Some(p) = std::env::var_os("ISO_CC_HOST_SHELL") {
            cands.push(PathBuf::from(p));
        }
        cands.push(PathBuf::from(HOST_SHELL));
        cands.push(PathBuf::from("bash"));
        cands.push(PathBuf::from("sh"));
        cands.into_iter().find(|cand| {
            std::process::Command::new(cand)
                .arg("-c")
                .arg("true")
                .status()
                .is_ok_and(|st| st.success())
        })
    }

    /// 把解析到的 shell 经 env 注入 exec-rpc 宿主执行面（[`host_shell`]）。
    /// nextest 每测试独立进程，set_var 无跨测试竞争；cargo test 多线程下两处
    /// 注入值恒相同，最坏回落正本 `/bin/bash`（宿主常态），断言不受影响。
    fn inject_host_shell(shell: &Path) {
        std::env::set_var("ISO_CC_HOST_SHELL", shell);
    }

    fn prof(toml_str: &str) -> Profile {
        toml::from_str(toml_str).unwrap()
    }

    #[test]
    fn request_roundtrip_and_deny_unknown() {
        let req = ExecRequest {
            script: "echo hi".into(),
            cwd: "/tmp".into(),
            env: Default::default(),
            argv: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"script\":\"echo hi\""), "{json}");
        assert!(json.contains("\"env\":{}"), "{json}");
        let back: ExecRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
        let err = serde_json::from_str::<ExecRequest>(
            r#"{"script":"s","cwd":"/tmp","env":{},"extra":1}"#,
        );
        assert!(err.is_err(), "deny_unknown_fields 必须拒绝未知键（#5）");
    }

    #[test]
    fn response_tagged_shapes() {
        let c: ExecResponse = serde_json::from_str(r#"{"status":"code","code":7}"#).unwrap();
        assert_eq!(c, ExecResponse::Code { code: 7 });
        let s: ExecResponse = serde_json::from_str(r#"{"status":"signal","signal":2}"#).unwrap();
        assert_eq!(s, ExecResponse::Signal { signal: 2 });
        let e: ExecResponse = serde_json::from_str(r#"{"status":"error","error":"boom"}"#).unwrap();
        assert_eq!(
            e,
            ExecResponse::Error {
                error: "boom".into()
            }
        );
        assert!(
            serde_json::from_str::<ExecResponse>(r#"{"status":"code","code":7,"signal":2}"#)
                .is_err(),
            "跨变体混合键必须拒绝（deny_unknown_fields）"
        );
        assert!(
            serde_json::from_str::<ExecResponse>(r#"{"status":"meh"}"#).is_err(),
            "未知 status 必须拒绝（#5）"
        );
    }

    #[test]
    fn worker_env_mirrors_apply_env_plus_marker() {
        let p = prof(
            "egress='if:wg0'\nlocale.tz='Asia/Singapore'\nlocale.lang='en_SG.UTF-8'\nenv.HISTFILE='/tmp/h'",
        );
        let sock = Path::new("/s/t/exec.sock");
        let vars = worker_env_vars(&p, "sg-1", sock);
        assert_eq!(vars.get("TZ").unwrap(), "Asia/Singapore");
        assert_eq!(vars.get("LANG").unwrap(), "en_SG.UTF-8");
        assert_eq!(vars.get("LC_ALL").unwrap(), "en_SG.UTF-8");
        assert_eq!(vars.get("HISTFILE").unwrap(), "/tmp/h");
        assert_eq!(
            vars.get("ISO_CC_SESSION").unwrap(),
            "sg-1",
            "spec#4 会话标记"
        );
        assert_eq!(
            vars.get("ISO_CC_EXEC_SOCK").unwrap(),
            "/s/t/exec.sock",
            "票 25 G1b 通道指针：第二跳 shim 经同一通道转发"
        );
        assert_eq!(vars.len(), 6);
        // 无 locale 声明 = 仅会话标记
        let p2 = prof("egress='if:wg0'");
        let vars2 = worker_env_vars(&p2, "x-2", sock);
        assert_eq!(
            vars2,
            BTreeMap::from([
                ("ISO_CC_EXEC_SOCK".into(), "/s/t/exec.sock".into()),
                ("ISO_CC_SESSION".into(), "x-2".into())
            ])
        );
    }

    /// 端到端通道（进程内）：serve 线程 + 真实 unix socket（sendmsg + SCM_RIGHTS 完整
    /// 链路）→ 宿主 worker stdout 直通 + cwd/env 注入 + 退出码回传。
    #[test]
    fn channel_roundtrip_stdio_env_and_exit_code() {
        // 票 21：宿主 shell 参数化（沙箱内 = nix store bash；全缺 = fail-loud 跳过）。
        let Some(shell) = test_host_shell() else {
            eprintln!(
                "[skip] 票 21: 无可用宿主 shell（/bin/bash 与 PATH bash/sh 均不可探活）——跳过"
            );
            return;
        };
        inject_host_shell(&shell);
        let dir = tempfile::tempdir().unwrap();
        let p = prof("egress='if:wg0'\nlocale.tz='Asia/Singapore'");
        let ch = prepare(dir.path(), &p, "t-chan", false).expect("prepare");
        let server = ch.serve();
        let sock_path = dir.path().join("exec.sock");

        // 管道：worker stdout → 写端经 SCM_RIGHTS 传入；测试侧持读端断言。
        let (r, w) = nix::unistd::pipe().expect("pipe");
        let devnull = OwnedFd::from(std::fs::File::open("/dev/null").unwrap());
        let fds = [devnull.as_raw_fd(), w.as_raw_fd(), w.as_raw_fd()];

        let req = ExecRequest {
            script: "echo chan-ok; exit 7".into(),
            cwd: dir.path().to_string_lossy().into_owned(),
            env: Default::default(),
            argv: None,
        };
        let mut conn = UnixStream::connect(&sock_path).expect("connect");
        send_request(&conn, &req, &fds).expect("send_request");
        let resp = read_frame(&mut conn).expect("read_frame");
        drop(conn);
        assert_eq!(resp, ExecResponse::Code { code: 7 }, "{resp:?}");
        let mut buf = [0u8; 4096];
        let n = nix::unistd::read(&r, &mut buf).expect("read pipe");
        assert!(
            String::from_utf8_lossy(&buf[..n]).contains("chan-ok"),
            "{:?}",
            &buf[..n]
        );

        // cwd + locale env 断言（A2「cwd/env/locale 一致」的通道面证据）。
        let req2 = ExecRequest {
            script: "pwd; echo TZ=$TZ".into(),
            cwd: dir.path().to_string_lossy().into_owned(),
            env: Default::default(),
            argv: None,
        };
        let mut conn2 = UnixStream::connect(&sock_path).expect("connect2");
        send_request(&conn2, &req2, &fds).expect("send2");
        let resp2 = read_frame(&mut conn2).expect("read2");
        drop(conn2);
        assert_eq!(resp2, ExecResponse::Code { code: 0 });
        let n2 = nix::unistd::read(&r, &mut buf).expect("read pipe2");
        let s2 = String::from_utf8_lossy(&buf[..n2]).into_owned();
        assert!(s2.contains(dir.path().to_str().unwrap()), "{s2}");
        assert!(s2.contains("TZ=Asia/Singapore"), "{s2}");

        // 错误路径：cwd 不存在 → Error 响应（fail-loud，不静默回落）。
        let req3 = ExecRequest {
            script: "true".into(),
            cwd: "/nonexistent-iso-cc-t15".into(),
            env: Default::default(),
            argv: None,
        };
        let mut conn3 = UnixStream::connect(&sock_path).expect("connect3");
        send_request(&conn3, &req3, &fds).expect("send3");
        let resp3 = read_frame(&mut conn3).expect("read3");
        drop(conn3);
        assert!(matches!(resp3, ExecResponse::Error { .. }), "{resp3:?}");

        server.shutdown();
    }

    /// 中断转发（spec#1）：对端断开 → 在途 worker 被 SIGKILL（sleep 不遗留为 residue）。
    #[test]
    fn peer_eof_interrupts_worker() {
        // 票 21：宿主 shell 参数化（worker 未 spawn = 未登记，见 channel 测试注释）。
        let Some(shell) = test_host_shell() else {
            eprintln!(
                "[skip] 票 21: 无可用宿主 shell（/bin/bash 与 PATH bash/sh 均不可探活）——跳过"
            );
            return;
        };
        inject_host_shell(&shell);
        let dir = tempfile::tempdir().unwrap();
        let p = prof("egress='if:wg0'");
        let ch = prepare(dir.path(), &p, "t-intr", false).expect("prepare");
        let server = ch.serve();
        let sock_path = dir.path().join("exec.sock");

        let devnull = OwnedFd::from(std::fs::File::open("/dev/null").unwrap());
        let fds = [
            devnull.as_raw_fd(),
            devnull.as_raw_fd(),
            devnull.as_raw_fd(),
        ];
        let req = ExecRequest {
            script: "sleep 30".into(),
            cwd: dir.path().to_string_lossy().into_owned(),
            env: Default::default(),
            argv: None,
        };
        let conn = UnixStream::connect(&sock_path).expect("connect");
        send_request(&conn, &req, &fds).expect("send");
        // worker 登记后克隆 pid（不动登记簿——watcher/waiter 自行收尾）。
        let mut tries = 0;
        let pid = loop {
            if let Some(p) = server.registry.lock().first().copied() {
                break p;
            }
            tries += 1;
            assert!(tries < 200, "worker 未登记");
            thread::sleep(std::time::Duration::from_millis(10));
        };
        drop(conn); // stub 死亡 = EOF

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut gone = false;
        while std::time::Instant::now() < deadline {
            if !Path::new(&format!("/proc/{pid}")).exists() {
                gone = true;
                break;
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(gone, "对端断开后 worker pid={pid} 未被中断转发击杀");
        server.shutdown();
    }

    #[test]
    fn peer_allowlist_covers_euid_and_mark_uid_only() {
        let euid = unsafe { libc::geteuid() };
        assert!(peer_allowed_for(euid, euid), "宿主自身/shutdown 必须放行");
        assert!(
            peer_allowed_for(crate::config::MARK_UID, euid),
            "mark 会话树 uid 必须放行（票 18）"
        );
        if crate::config::MARK_UID != euid {
            assert!(!peer_allowed_for(euid + 7, euid), "任意本地 uid 必须拒绝");
        }
    }

    #[test]
    fn prepare_socket_mode_follows_cross_uid_axis() {
        // netns（cross_uid=false）= 0600（附⑦ 语义不变）；mark = 0666 +
        // accept 期凭证白名单强制（peer_allowlist 测试为强制核）。
        let p = prof("egress='if:wg0'");
        {
            let dir = tempfile::tempdir().unwrap();
            let ch = prepare(dir.path(), &p, "t-mode-netns", false).expect("prepare netns");
            let md = std::fs::metadata(&ch.sock_path).unwrap();
            assert_eq!(md.permissions().mode() & 0o777, 0o600);
        }
        {
            let dir = tempfile::tempdir().unwrap();
            // 票 21：mark 分支经 mark::ensure_shim_binary 写 /var/tmp/iso-cc-mark/bin
            //（uid 4210 资产根）——nix 沙箱该路径不可写 → fail-loud 跳过本分支；
            // socket-mode 断言语义由上方 netns 分支同构覆盖，mark 分支宿主面照常执行。
            let mark_bin = crate::mark::state_root().join("bin");
            if std::fs::create_dir_all(&mark_bin).is_ok() {
                let ch = prepare(dir.path(), &p, "t-mode-mark", true).expect("prepare mark");
                let md = std::fs::metadata(&ch.sock_path).unwrap();
                assert_eq!(md.permissions().mode() & 0o777, 0o666);
            } else {
                eprintln!(
                    "[skip] 票 21: mark 根 {} 不可写（nix 沙箱）——跳过 mark 分支 socket-mode 断言",
                    mark_bin.display()
                );
            }
        }
    }
}
