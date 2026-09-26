//! ns.rs —— 会话 namespace 装配原语（票 11；设计稿 design-session-lanes §2）。
//!
//! 全 crate 唯一 unsafe 面：每个 unsafe fn 的前置条件写成 SAFETY 注释并由入口保证
//! （§2.2）；错误一律 `last_os_error`，无文案（原语层，§2.1）。binds 的 CString 在
//! 父进程预转换（[`cstring_binds`]），pre_exec 闭包只持转换结果——本模块成功路径
//! 零堆分配（自映射行以栈上缓冲构造）。

use std::ffi::{CStr, CString};
use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;

use nix::mount::{mount, MsFlags};
use nix::sched::{unshare, CloneFlags};

/// 入口 A（primary，pasta spawn 之下，§1.3 上树）：仅 CLONE_NEWNS + rprivate + bind_ro +
/// PDEATHSIG 对账。CLONE_NEWUSER|NEWNET 建立与 uid_map 映射面不在本入口——pasta spawn
/// 自建 userns/netns 并内建映射（E2），该面移出入口 A（§2.3 收缩幅度）。
///
/// # SAFETY(入口)
///
/// 仅 fork 后 exec 前或 bootstrap 单线程早期上下文调用；`binds` 已预转换；调用者已
/// 处于 pasta 建立的 userns/netns（E2），故 CLONE_NEWNS 合法。
///
/// `expected_ppid` = PDEATHSIG 对账基准：非 0 → 精确对账（`getppid()` 必须等于该值）；
/// 0 → 结构对账（pasta spawn 路径父 pid 不可预知：`getppid() == 1` 即已 reparent 到
/// init = 原父已死）。不符一律 kill(self) 自尽（PR_SET_PDEATHSIG(2const) 竞态②）。
pub fn enter_mountns(binds: &[(CString, CString)], expected_ppid: u32) -> io::Result<()> {
    set_pdeathsig_verified(libc::SIGKILL, expected_ppid)?;
    // SAFETY: 单线程 pre_exec/bootstrap 早期；CLONE_NEWNS 仅需调用者位于目标 userns（入口保证）
    unsafe { unshare_mountns()? };
    // SAFETY: mount(2) 于本 ns；MS_REC|MS_PRIVATE 不产生宿主可见变化（mountns 私有）
    unsafe { make_root_private()? };
    for (src, dst) in binds {
        // SAFETY: 两次 mount(2)；dst 存在性由 run 期挂载点预创建断言保证
        unsafe { bind_ro(src, dst)? };
    }
    Ok(())
}

/// 入口 B（slirp4netns 回退专用，§1.3 下树）：三 ns 自建（CLONE_NEWUSER|NEWNS|NEWNET）+
/// 单条自映射 + rprivate + bind_ro。parent 永不写 /proc/<pid>/maps（Facts §3 残留源消灭），
/// 自映射在本入口内 pre_exec 期完成（竞态面消失，父死 = 无窗口）。
///
/// # SAFETY(入口)
///
/// 仅 pre_exec（fork 后 exec 前）单线程上下文调用（CLONE_NEWUSER 对多线程进程失败）；
/// `binds` 已预转换；`expected_ppid` = spawn 时刻父 pid（iso-cc 直接 spawn，精确已知）。
pub fn enter_selfmap_ns(binds: &[(CString, CString)], expected_ppid: u32) -> io::Result<()> {
    // 自映射行需要 unshare 时刻的真实（parent ns）uid/gid：unshare(CLONE_NEWUSER) 之后
    // 尚无 uid_map，getuid()/getgid() 呈未映射 overflow uid（65534），不可作映射源
    // SAFETY: getuid(2)/getgid(2) AS-safe；libc 声明为 unsafe extern（仅属性读取）
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    // SAFETY: 单线程 pre_exec；CLONE_NEWUSER 对多线程进程失败——入口保证单线程
    unsafe { unshare_user_net_mountns()? };
    // SAFETY: prctl(2)/getppid(2)/kill(2) 均 AS-safe；对账 = §4-L1（PR_SET_PDEATHSIG(2const) 竞态清单）
    set_pdeathsig_verified(libc::SIGKILL, expected_ppid)?;
    // SAFETY: user_namespaces(7) 单条自映射规则；先于挂载操作（caps 依赖 userns root）
    unsafe { write_self_ugid_map(uid, gid)? };
    // SAFETY: mount(2) 于本 ns；MS_REC|MS_PRIVATE 不产生宿主可见变化（mountns 私有）
    unsafe { make_root_private()? };
    for (src, dst) in binds {
        // SAFETY: 两次 mount(2)；dst 存在性由 run 期挂载点预创建断言保证
        unsafe { bind_ro(src, dst)? };
    }
    Ok(())
}

/// pre_exec 安装面：`CommandExt::pre_exec` 本体为 unsafe fn（其 unsafety = 闭包须在
/// fork 后 exec 前、单线程上下文执行且 AS-safe），安装收敛到本模块——session.rs
/// 保持零 unsafe 词法。`hook` 只能由本模块入口 A/B（或 set_pdeathsig_verified）构成，
/// 逐条满足 AS-safe 纪律（§2.1/§2.2）。
pub fn install_pre_exec<F>(cmd: &mut Command, hook: F)
where
    F: FnMut() -> io::Result<()> + Send + Sync + 'static,
{
    // SAFETY: pre_exec 前置条件（AS-safe、fork-exec 窗口、单线程）由 hook 的构成方保证
    unsafe { cmd.pre_exec(hook) };
}

/// PDEATHSIG 挂载 + 竞态对账（设计稿 §4-L1 / man PR_SET_PDEATHSIG(2const) 竞态清单）：
/// prctl 后 getppid 对账 `expected_ppid`，不符自尽（kill self）。竞态②「prctl 时父已死
/// 不发信号」由对账覆盖；prctl 之后父死亡由内核信号覆盖。
///
/// # SAFETY
///
/// prctl(2)/getppid(2)/kill(2) 均 AS-safe；对账逻辑见 §4-L1。`expected_ppid == 0` =
/// 结构对账（pasta spawn 路径父 pid 不可预知，reparent 到 pid 1 即原父已死）；
/// 非 0 = 精确对账（父 pid 由调用方 spawn 时刻已知，slirp 父侧 pre_exec）。
pub fn set_pdeathsig_verified(sig: libc::c_int, expected_ppid: u32) -> io::Result<()> {
    // SAFETY: prctl(2) AS-safe；PR_SET_PDEATHSIG 仅写进程属性，无误用面
    if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, sig) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: getppid(2) AS-safe；libc 声明为 unsafe extern（仅属性读取，无 UB 面）
    let ppid = unsafe { libc::getppid() } as u32;
    let orphaned = if expected_ppid == 0 {
        ppid == 1
    } else {
        ppid != expected_ppid
    };
    if orphaned {
        // SAFETY: kill(2) AS-safe；自尽按竞态清单②（prctl 时父已死则不发信号）
        unsafe { libc::kill(libc::getpid(), sig) };
        return Err(io::Error::other("pdeathsig 对账失败（父进程已死）"));
    }
    Ok(())
}

/// SAFETY: 单线程 pre_exec；CLONE_NEWNS 仅需调用者位于目标 userns（入口 A 保证）。
unsafe fn unshare_mountns() -> io::Result<()> {
    unshare(CloneFlags::CLONE_NEWNS).map_err(|_| io::Error::last_os_error())
}

/// SAFETY: 单线程 pre_exec；CLONE_NEWUSER 对多线程进程失败——入口 B 保证单线程。
unsafe fn unshare_user_net_mountns() -> io::Result<()> {
    unshare(CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNS | CloneFlags::CLONE_NEWNET)
        .map_err(|_| io::Error::last_os_error())
}

/// SAFETY: mount(2) 于本 ns；MS_REC|MS_PRIVATE 不产生宿主可见变化（mountns 私有）。
unsafe fn make_root_private() -> io::Result<()> {
    mount(
        None::<&CStr>,
        c"/",
        None::<&CStr>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&CStr>,
    )
    .map_err(|_| io::Error::last_os_error())
}

/// SAFETY: 两次 mount(2)（BIND → BIND|REMOUNT|RDONLY）；dst 存在性由 run 期挂载点
/// 预创建断言保证。
unsafe fn bind_ro(src: &CString, dst: &CString) -> io::Result<()> {
    mount(
        Some(src.as_c_str()),
        dst.as_c_str(),
        None::<&CStr>,
        MsFlags::MS_BIND,
        None::<&CStr>,
    )
    .map_err(|_| io::Error::last_os_error())?;
    mount(
        None::<&CStr>,
        dst.as_c_str(),
        None::<&CStr>,
        MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY,
        None::<&CStr>,
    )
    .map_err(|_| io::Error::last_os_error())
}

/// `uid`/`gid` 必须是 unshare(CLONE_NEWUSER) 之前捕获的 parent ns 真实 uid/gid
/// （user_namespaces(7) 单条自映射规则："0 <own> 1" 的 <own> = unshare 时刻的
/// 有效 uid/gid；unshare 之后 getuid() 呈未映射 overflow uid，不可用）。
///
/// SAFETY: 先 setgroups=deny，再 uid_map/gid_map 各一行；open(2)/write(2)/close(2)
/// 均 AS-safe；在挂载操作之前执行（caps 依赖 userns root）。行内容栈上构造：
/// pre_exec 上下文零堆分配（§2.1）。
unsafe fn write_self_ugid_map(uid: libc::uid_t, gid: libc::gid_t) -> io::Result<()> {
    write_self_map(c"/proc/self/setgroups", b"deny")?;
    let (ubuf, ulen) = map_line(uid);
    write_self_map(c"/proc/self/uid_map", &ubuf[..ulen])?;
    let (gbuf, glen) = map_line(gid);
    write_self_map(c"/proc/self/gid_map", &gbuf[..glen])
}

/// `/proc/self/<file>` 直写：open(2)/write(2)/close(2) 均 AS-safe（man 约定）；
/// 循环写完全部字节（proc 写允许部分写）。成功路径零堆分配（pre_exec 纪律，§2.1）。
fn write_self_map(path: &CStr, data: &[u8]) -> io::Result<()> {
    // SAFETY: open(2)；path 为调用方持有的有效 CStr，O_WRONLY 只写打开
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut off = 0usize;
    while off < data.len() {
        // SAFETY: write(2)；fd 有效，指针/长度与 data[off..] 匹配
        let n = unsafe {
            libc::write(
                fd,
                data.as_ptr().add(off) as *const libc::c_void,
                data.len() - off,
            )
        };
        if n < 0 {
            // SAFETY: close(2)；fd 为 open(2) 返回的有效描述符
            unsafe { libc::close(fd) };
            return Err(io::Error::last_os_error());
        }
        off += n as usize;
    }
    // SAFETY: close(2)；fd 为 open(2) 返回的有效描述符
    unsafe { libc::close(fd) };
    Ok(())
}

/// 自映射行 `"0 <id> 1\n"` 栈上构造（零堆分配；返回 (缓冲, 有效长度)）。
fn map_line(id: u32) -> ([u8; 16], usize) {
    let mut buf = [0u8; 16]; // "0 " + ≤10 位数字 + " 1\n" ≤ 15
    buf[0] = b'0';
    buf[1] = b' ';
    let mut digits = [0u8; 10];
    let mut d = 0;
    let mut v = id;
    loop {
        digits[d] = b'0' + (v % 10) as u8;
        d += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let mut n = 2;
    while d > 0 {
        d -= 1;
        buf[n] = digits[d];
        n += 1;
    }
    buf[n] = b' ';
    buf[n + 1] = b'1';
    buf[n + 2] = b'\n';
    (buf, n + 3)
}

/// binds 的 CString 父进程预转换（§2.1：pre_exec 闭包只持转换结果）。
pub fn cstring_binds(binds: &[(String, String)]) -> io::Result<Vec<(CString, CString)>> {
    binds
        .iter()
        .map(|(s, d)| -> io::Result<(CString, CString)> {
            Ok((
                CString::new(s.as_str())
                    .map_err(|_| io::Error::other(format!("bind src 含 NUL：{s:?}")))?,
                CString::new(d.as_str())
                    .map_err(|_| io::Error::other(format!("bind dst 含 NUL：{d:?}")))?,
            ))
        })
        .collect()
}
