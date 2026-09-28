//! 会话观测 + L3 对账 sweep 枚举器（票 13；设计稿 design-session-lanes §4-L3）。
//!
//! 枚举键（09 取证硬约束的落地）：
//! - **pasta 本体**：re-exec passt.avx2（file capabilities → dumpable=0）使
//!   `/proc/<pid>/environ` 与 `/proc/<pid>/ns/` 均 EACCES（票 13 实测复核）→
//!   唯一可读键 = argv 指纹（`--outbound-if4`）+ spawn 模式拼在 cmdline 尾部的
//!   bootstrap argv（`--session-id <id>`）。
//! - **slirp4netns 网关**：普通二进制，`Command::env` 注入的 ISO_CC_SESSION 可读。
//! - **cc 树**：bootstrap 自设 `env::set_var`，经 execve 保持，可靠。
//!
//! 清除动作（SIGKILL/删除）归 gc（票 14）；本模块只枚举 + 判定。

use serde::Serialize;
use std::collections::BTreeSet;
use std::io;

/// 环境标记键（list.rs 原 :11 注释错位修正：env 键只覆盖 cc 树与 slirp 网关；
/// pasta 本体 environ EACCES，不可用此键——见模块文档）。
const MARKER: &[u8] = b"ISO_CC_SESSION=";

#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub pid: u32,
    pub id: String,
    /// "gateway"（pasta argv 键 / slirp env 键）或 "cc"（bootstrap 自设标记）。
    pub kind: &'static str,
}

/// /proc 单进程观测事实（L3 sweep 枚举基础，设计稿 §4-L3）。
#[derive(Debug, Clone)]
pub struct ProcFacts {
    pub pid: u32,
    /// ISO_CC_SESSION 标记值（environ 可读时；pasta 本体 EACCES → None）。
    pub marker: Option<String>,
    pub cmdline: Vec<String>,
    /// /proc/<pid>/ns/net inode（pasta 本体 EACCES → None）。
    pub netns: Option<u64>,
}

/// L3 sweep 产出（doctor sweep 检查组输入，设计稿 §5「reverse 现实→清单」）。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct LifecycleResidue {
    pub gateways: Vec<GatewayResidue>,
    pub dirs: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GatewayResidue {
    pub pid: u32,
    /// "pasta" | "slirp4netns"
    pub kind: String,
    pub session_id: Option<String>,
    pub cmdline: String,
}

/// 网关 argv 指纹：pasta = cmdline 含 `--outbound-if4`（proven invocation 专属 flag），
/// 或 argv0 基名 ∈ {pasta, passt, passt.avx2}（re-exec 后 comm 变、cmdline 保留原 argv）；
/// slirp4netns = argv0 基名。
pub fn gateway_kind(cmdline: &[String]) -> Option<&'static str> {
    if cmdline.iter().any(|a| a == "--outbound-if4") {
        return Some("pasta");
    }
    let argv0 = cmdline.first()?;
    let base = argv0.rsplit('/').next().unwrap_or(argv0);
    match base {
        "pasta" | "passt" | "passt.avx2" => Some("pasta"),
        "slirp4netns" => Some("slirp4netns"),
        _ => None,
    }
}

/// pasta 本体的会话关联键：spawn 模式下 bootstrap argv 拼在其 cmdline 尾部
/// （`… -- <iso-cc> session-bootstrap --plan … --session-id <id> -- …`），
/// `--session-id` 的下一 token 即会话 id。
pub fn argv_session_id(cmdline: &[String]) -> Option<String> {
    cmdline
        .iter()
        .position(|a| a == "--session-id")
        .and_then(|i| cmdline.get(i + 1))
        .cloned()
}

/// 活跃会话视图（`iso-cc list`）：env 标记进程（cc 树 + slirp 网关）∪ pasta 网关
/// （argv 键）。会话死 = 标记/进程消失，list 自然收敛（状态零持久）。
pub fn scan() -> Vec<Session> {
    let mut out = Vec::new();
    for p in scan_procs() {
        let gw = gateway_kind(&p.cmdline);
        // mark 会话根（票 18）：uid 4210 树的 environ 对宿主 EACCES（setuid 非规则
        // dumpable），env 标记键不可读——argv `--session-id` 键兜底（cmdline 全局可读；
        // 常驻助手 cmdline 恒带该 flag，见 session::spawn_mark）。
        let argv_id = argv_session_id(&p.cmdline);
        if p.marker.is_none() && gw.is_none() && argv_id.is_none() {
            continue;
        }
        let id = p.marker.or(argv_id).unwrap_or_else(|| "?".into());
        out.push(Session {
            pid: p.pid,
            id,
            kind: if gw.is_some() { "gateway" } else { "cc" },
        });
    }
    out.sort_by_key(|s| s.pid);
    out
}

/// 真实 sweep（doctor 调用）：/proc 全量事实 + sessions/* 目录名 → 纯判定核心。
pub fn sweep_residue() -> LifecycleResidue {
    sweep_from(&scan_procs(), &session_dir_names())
}

/// 纯判定核心（伪造事实驱动，可单测）：
/// - cc 集 = 带标记且非网关指纹的进程（bootstrap 自设标记，09 取证可靠键）；
///   其 netns inode 集 = 活跃 netns 集。
/// - 网关归属双键（设计稿 §4-L3）：会话 id 命中 cc 集，或 netns inode 命中活跃集。
///   pasta 本体两键皆 EACCES 不可得 → 恒走 argv id 键；伪造孤儿两键皆不中 → residue。
/// - 无主目录 = sessions/<id> 的 id 不在 cc 集（活跃会话目录不报）。
pub fn sweep_from(procs: &[ProcFacts], dirs: &[String]) -> LifecycleResidue {
    let mut cc_ids: BTreeSet<&str> = BTreeSet::new();
    let mut live_netns: BTreeSet<u64> = BTreeSet::new();
    for p in procs {
        let Some(marker) = p.marker.as_deref() else {
            continue;
        };
        if gateway_kind(&p.cmdline).is_some() {
            continue;
        }
        cc_ids.insert(marker);
        if let Some(ns) = p.netns {
            live_netns.insert(ns);
        }
    }
    // mark 会话根（票 18）：environ EACCES → marker 键不可得，argv id 兜底
    //（gateway 指纹进程仍走 gateway 归属键，不入 cc 集）。
    let argv_ids: Vec<String> = procs
        .iter()
        .filter(|p| p.marker.is_none() && gateway_kind(&p.cmdline).is_none())
        .filter_map(|p| argv_session_id(&p.cmdline))
        .collect();
    for id in &argv_ids {
        cc_ids.insert(id.as_str());
    }
    let mut gateways = Vec::new();
    for p in procs {
        let Some(kind) = gateway_kind(&p.cmdline) else {
            continue;
        };
        let sid = p.marker.clone().or_else(|| argv_session_id(&p.cmdline));
        let owned = sid.as_deref().is_some_and(|s| cc_ids.contains(s))
            || p.netns.is_some_and(|ns| live_netns.contains(&ns));
        if !owned {
            gateways.push(GatewayResidue {
                pid: p.pid,
                kind: kind.to_string(),
                session_id: sid,
                cmdline: p.cmdline.join(" "),
            });
        }
    }
    let orphan_dirs = dirs
        .iter()
        .filter(|d| !cc_ids.contains(d.as_str()))
        .cloned()
        .collect();
    LifecycleResidue {
        gateways,
        dirs: orphan_dirs,
    }
}

/// L2 收编循环观测原语：ppid == 指定父的进程集（subreaper 语义下的收养子女）。
pub fn children_of(ppid: u32) -> Vec<u32> {
    proc_pids()
        .into_iter()
        .filter(|pid| read_ppid(*pid) == Some(ppid))
        .collect()
}

/// 单进程标记读取（L2 收编的双键之「标记」键）。三分语义（票 25 G3）：
/// `Ok(Some)` = 标记在册；`Ok(None)` = environ 可读且确无标记；`Err` = 读失败
///（ESRCH/ENOENT——进程退出/僵尸窗口，票 24 E1' 实锤的「无标记」误记来源）。
/// 调用方必须分记 `Err` 与 `Ok(None)`，不得把 race 折算成「无标记」。
pub fn proc_marker(pid: u32) -> io::Result<Option<String>> {
    let env = std::fs::read(format!("/proc/{pid}/environ"))?;
    Ok(marker_from_environ(&env))
}

fn scan_procs() -> Vec<ProcFacts> {
    proc_pids()
        .into_iter()
        .map(|pid| ProcFacts {
            pid,
            marker: read_environ_marker(pid),
            cmdline: read_cmdline(pid),
            netns: netns_inode(pid),
        })
        .collect()
}

fn proc_pids() -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut v: Vec<u32> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|s| s.parse().ok()))
        .collect();
    v.sort_unstable();
    v
}

fn read_environ_marker(pid: u32) -> Option<String> {
    marker_from_environ(&std::fs::read(format!("/proc/{pid}/environ")).ok()?)
}

fn marker_from_environ(env: &[u8]) -> Option<String> {
    env.split(|b| *b == 0)
        .filter(|kv| !kv.is_empty())
        .find_map(|kv| kv.strip_prefix(MARKER))
        .map(|s| String::from_utf8_lossy(s).into_owned())
}

fn read_cmdline(pid: u32) -> Vec<String> {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|b| {
            b.split(|c| *c == 0)
                .filter(|s| !s.is_empty())
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// `net:[4026531956]` → 4026531956。pasta 本体 EACCES → None（owner 语义：None = 键不可得）。
fn netns_inode(pid: u32) -> Option<u64> {
    let target = std::fs::read_link(format!("/proc/{pid}/ns/net")).ok()?;
    let s = target.to_string_lossy();
    s.strip_prefix("net:[")?.strip_suffix(']')?.parse().ok()
}

/// /proc/<pid>/stat 第 4 字段（ppid）；comm 可含空格/括号 → 从最后一个 `)` 后解析。
fn read_ppid(pid: u32) -> Option<u32> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = s.rfind(')')?;
    let mut it = s[close + 1..].split_whitespace();
    it.next()?; // state
    it.next()?.parse().ok()
}

/// sessions/ 下的目录名（= 会话 id 集合；枚举基础，spawn() 09 期建立）。
/// 票 18：mark 引擎会话资产在 /var/tmp/iso-cc-mark/sessions（跨 uid 可达根），
/// 与 netns 的 state 根一并枚举（同一 id 命名空间，sweep/gc 对账共用）。
fn session_dir_names() -> Vec<String> {
    let mut names = BTreeSet::new();
    for root in [
        crate::session::sessions_root().ok(),
        Some(crate::mark::sessions_root()),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(rd) = std::fs::read_dir(&root) {
            names.extend(
                rd.flatten()
                    .filter_map(|e| e.file_name().into_string().ok()),
            );
        }
    }
    names.into_iter().collect()
}

pub fn render_human(sessions: &[Session]) -> String {
    if sessions.is_empty() {
        return "no live sessions\n".into();
    }
    let mut s = format!("{:>7}  {:<7}  {}\n", "PID", "KIND", "SESSION");
    for x in sessions {
        s.push_str(&format!("{:>7}  {:<7}  {}\n", x.pid, x.kind, x.id));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmdline(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    fn facts(pid: u32, marker: Option<&str>, args: &[&str], netns: Option<u64>) -> ProcFacts {
        ProcFacts {
            pid,
            marker: marker.map(Into::into),
            cmdline: cmdline(args),
            netns,
        }
    }

    #[test]
    fn proc_marker_tri_state_splits_race_from_true_absence() {
        // 票 25 G3：E1' 实锤 race 伪象（标记在册，扫描落退出/僵尸窗口 → environ
        // 读失败被记成「无标记」）。三分契约：活进程 = Ok（带不带标记随环境）；
        // 摘除标记的活子进程 = Ok(None)（真缺失）；已亡 pid = Err（race 分支）。
        assert!(
            proc_marker(std::process::id()).is_ok(),
            "活进程自身 environ 必可读"
        );
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .env_remove("ISO_CC_SESSION")
            .spawn()
            .expect("spawn sleep");
        let absent = proc_marker(child.id());
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(absent.unwrap(), None, "可读但无标记 = Ok(None)，非 Err");
        let gone = proc_marker(u32::MAX - 1).unwrap_err();
        assert_eq!(
            gone.kind(),
            io::ErrorKind::NotFound,
            "读失败 = Err（race 分支），不得折成 None"
        );
    }

    #[test]
    fn pasta_argv_fingerprint_and_session_id() {
        let args = cmdline(&[
            "pasta",
            "-f",
            "--outbound-if4",
            "eth0",
            "-I",
            "tap0",
            "--",
            "iso-cc",
            "session-bootstrap",
            "--session-id",
            "pasta-42",
            "--",
            "true",
        ]);
        assert_eq!(gateway_kind(&args), Some("pasta"));
        assert_eq!(argv_session_id(&args).as_deref(), Some("pasta-42"));
    }

    #[test]
    fn passt_reexec_comm_shape_still_fingerprints() {
        // pasta re-exec 后 argv0 保留原样；argv0 基名兜底覆盖手工改名场景
        assert_eq!(
            gateway_kind(&cmdline(&["/usr/bin/passt.avx2", "-f"])),
            Some("pasta")
        );
        assert_eq!(
            gateway_kind(&cmdline(&["slirp4netns", "1", "tap0", "-c"])),
            Some("slirp4netns")
        );
        assert_eq!(gateway_kind(&cmdline(&["bash", "-c", "true"])), None);
    }

    #[test]
    fn forged_pasta_without_session_id_is_residue() {
        // 票 13 验收 #3 形态：伪造孤儿 pasta（--outbound-if4 指纹、无标记、无 --session-id、
        // 宿主 netns）→ residue
        let procs = vec![facts(
            100,
            None,
            &["bash", "-c", "sleep 299", "x", "--outbound-if4", "eth0"],
            None,
        )];
        let r = sweep_from(&procs, &[]);
        assert_eq!(r.gateways.len(), 1, "{r:?}");
        assert_eq!(r.gateways[0].pid, 100);
        assert_eq!(r.gateways[0].kind, "pasta");
        assert!(r.gateways[0].session_id.is_none());
        assert!(r.dirs.is_empty());
    }

    #[test]
    fn live_session_gateways_and_dirs_are_owned() {
        let procs = vec![
            // 两会话 cc 树：标记 + netns inode 可读
            facts(199, Some("s-1"), &["bash", "-c", "sleep"], Some(4026538888)),
            facts(
                200,
                Some("pasta-7"),
                &["bash", "-c", "sleep"],
                Some(4026539999),
            ),
            // pasta 本体：netns EACCES（None），argv id 键命中 cc 集
            facts(
                201,
                None,
                &[
                    "pasta",
                    "-f",
                    "--outbound-if4",
                    "eth0",
                    "--",
                    "iso-cc",
                    "session-bootstrap",
                    "--session-id",
                    "pasta-7",
                    "--",
                    "sleep",
                ],
                None,
            ),
            // slirp 网关：env 标记键命中
            facts(
                202,
                Some("s-1"),
                &["slirp4netns", "199", "tap0", "-c"],
                None,
            ),
        ];
        let r = sweep_from(&procs, &["pasta-7".to_string(), "crash-9".to_string()]);
        assert!(r.gateways.is_empty(), "{r:?}");
        assert_eq!(r.dirs, vec!["crash-9".to_string()]);
    }

    #[test]
    fn orphan_gateway_after_cc_death_is_residue() {
        // 网关中途/事后存活而 cc 树已死：标记 id 不再命中 cc 集 → residue（有界滞后收敛面）
        let procs = vec![facts(
            300,
            Some("s-1"),
            &["slirp4netns", "999", "tap0", "-c"],
            None,
        )];
        let r = sweep_from(&procs, &[]);
        assert_eq!(r.gateways.len(), 1, "{r:?}");
        assert_eq!(r.gateways[0].kind, "slirp4netns");
        assert_eq!(r.gateways[0].session_id.as_deref(), Some("s-1"));
    }

    #[test]
    fn unmarked_non_gateway_processes_are_not_residue() {
        let procs = vec![
            facts(400, None, &["systemd", "--user"], None),
            facts(401, None, &["vim", "Cargo.toml"], None),
        ];
        let r = sweep_from(&procs, &[]);
        assert!(r.gateways.is_empty(), "{r:?}");
    }

    #[test]
    fn netns_hit_owns_even_without_matching_id() {
        // netns 键独立成立：标记不可读但持有活跃会话 netns 的网关形态不算 residue
        let procs = vec![
            facts(500, Some("pasta-9"), &["bash"], Some(4026537777)),
            facts(
                501,
                None,
                &["slirp4netns", "500", "tap0", "-c"],
                Some(4026537777),
            ),
        ];
        let r = sweep_from(&procs, &[]);
        assert!(r.gateways.is_empty(), "{r:?}");
    }

    #[test]
    fn argv_session_id_covers_mark_session_root() {
        // 票 18：mark 会话根 uid 4210 → environ EACCES（marker None）；argv
        // `--session-id` 键兜底 → 会话目录在会话存活期不算 orphan，list 可见。
        let helper = crate::mark::helper_path();
        let procs = vec![facts(
            700,
            None,
            &[
                helper.to_str().unwrap(),
                "--session-id",
                "marksg-1",
                "--",
                "claude",
            ],
            None,
        )];
        let r = sweep_from(&procs, &["marksg-1".to_string()]);
        assert!(r.gateways.is_empty(), "{r:?}");
        assert!(r.dirs.is_empty(), "活跃 mark 会话目录不得报 orphan：{r:?}");
    }
}
