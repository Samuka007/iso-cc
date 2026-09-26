//! gc.rs —— 清单回收（票 14；设计稿 design-session-lanes §5）。
//!
//! - 默认：sweep 报告（L3 枚举器复用，票 13），有残留 exit 1（fail-loud 约定）。
//! - `--prune`：清 stale 条目（仅登记簿；profile 已从 config 消失的条目）。
//! - `--all`：全量回收——有活跃会话拒绝；mountpoint size/mtime 变化 = 已用户数据化
//!   拒绝（除非 `--force`）；profile-state 仅随显式 `--profile` 回收；**provider
//!   二进制永不删除**（非本工具所有），只除名。终态 = 清单与现实一致（残留=0）。

use crate::config::Profile;
use crate::list;
use crate::manifest::{self, Entry, EntryKind, Manifest};
use anyhow::{bail, Context};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct GcOpts {
    pub prune: bool,
    pub all: bool,
    pub yes: bool,
    pub force: bool,
    pub profile: Option<String>,
}

/// entry 的 profile 定界（mountpoint/profile-state 的 key = `<profile>:<路径或名>`）。
fn entry_profile(e: &Entry) -> Option<&str> {
    match e.kind {
        EntryKind::Mountpoint | EntryKind::ProfileState => e.key.split_once(':').map(|(p, _)| p),
        EntryKind::Provider => None,
    }
}

/// 纯判定：stale = profile 已从 config 消失的条目（设计稿 §5 doctor reverse 同一判定）。
pub fn stale_entries(entries: &[Entry], profiles: &BTreeMap<String, Profile>) -> Vec<Entry> {
    entries
        .iter()
        .filter(|e| entry_profile(e).is_some_and(|p| !profiles.contains_key(p)))
        .cloned()
        .collect()
}

/// 纯判定：活跃会话 = cc 树（bootstrap 自设标记）∪ 归属明确的网关。
/// id="?" 的网关指纹进程 = sweep 孤儿（票 13 判定核），不是活跃会话——恰是 --all 的清理对象。
pub fn has_active_sessions(sessions: &[list::Session]) -> bool {
    sessions.iter().any(|s| s.kind == "cc" || (s.kind == "gateway" && s.id != "?"))
}

/// 纯判定：mountpoint 回收守卫（设计稿 §5 gc：size/mtime 变化 = 已用户数据化 → 拒绝
/// 除非 --force）。登记时刻由 setup 在创建后采样（registered_at ≥ mtime），故
/// mtime > registered_at 即创建后被触碰。Err = 拒绝文案。
pub fn reclaim_decision(
    is_dir: bool,
    size: u64,
    dir_nonempty: bool,
    mtime_millis: u64,
    registered_at: u64,
    force: bool,
) -> Result<(), String> {
    let changed = (is_dir && dir_nonempty) || (!is_dir && size != 0) || mtime_millis > registered_at;
    if changed && !force {
        return Err("自创建后 size/mtime 变化 = 已用户数据化（--force 可显式越过）".into());
    }
    Ok(())
}

/// 现实侧事实（mountpoint 守卫输入）。
fn path_facts(p: &std::path::Path) -> anyhow::Result<(bool, u64, bool, u64)> {
    let meta = std::fs::metadata(p)
        .with_context(|| format!("读取挂载点元数据 {}", p.display()))?;
    let is_dir = meta.is_dir();
    let size = if is_dir { 0 } else { meta.len() };
    let dir_nonempty = is_dir
        && std::fs::read_dir(p)
            .with_context(|| format!("枚举挂载点目录 {}", p.display()))?
            .next()
            .transpose()?
            .is_some();
    let mtime = meta
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok((is_dir, size, dir_nonempty, mtime))
}

fn remove_path(p: &std::path::Path, is_dir: bool, force: bool) -> anyhow::Result<()> {
    if is_dir {
        if force {
            std::fs::remove_dir_all(p)
        } else {
            std::fs::remove_dir(p)
        }
    } else {
        std::fs::remove_file(p)
    }
    .with_context(|| format!("回收 {}", p.display()))
}

fn confirm_or_abort(opts: &GcOpts, plan_lines: &[String]) -> anyhow::Result<()> {
    if opts.yes {
        return Ok(());
    }
    eprintln!("gc --all 计划：");
    for l in plan_lines {
        eprintln!("  - {l}");
    }
    eprint!("确认执行？输入 y 继续：");
    use std::io::Write as _;
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("读取确认输入")?;
    if line.trim() != "y" {
        bail!("gc --all 已取消（未执行任何动作）");
    }
    Ok(())
}

fn sweep_report(residue: &list::LifecycleResidue) -> Vec<String> {
    let mut lines = Vec::new();
    if residue.gateways.is_empty() {
        lines.push("sweep：无孤儿网关".into());
    } else {
        for g in &residue.gateways {
            lines.push(format!(
                "sweep：孤儿网关 pid={} kind={} session={} cmd={:?}",
                g.pid,
                g.kind,
                g.session_id.as_deref().unwrap_or("?"),
                g.cmdline
            ));
        }
    }
    if residue.dirs.is_empty() {
        lines.push("sweep：无无主会话目录".into());
    } else {
        lines.push(format!("sweep：无主会话目录 ×{}（{}）", residue.dirs.len(), residue.dirs.join(", ")));
    }
    lines
}

/// `iso-cc gc` 入口。
pub fn run(opts: &GcOpts, profiles: &BTreeMap<String, Profile>) -> anyhow::Result<()> {
    let mut manifest = manifest::read()?.unwrap_or_else(Manifest::empty);

    // 默认：sweep 报告（无动作；残留 = exit 1）。
    if !opts.prune && !opts.all {
        for l in sweep_report(&list::sweep_residue()) {
            println!("gc: {l}");
        }
        println!(
            "gc: manifest {}（{} entries）",
            manifest::manifest_path().display(),
            manifest.entries.len()
        );
        let residue = list::sweep_residue();
        if !residue.gateways.is_empty() || !residue.dirs.is_empty() {
            std::process::exit(1);
        }
        return Ok(());
    }

    // --prune：仅登记簿。
    if opts.prune {
        let stale = stale_entries(&manifest.entries, profiles);
        for e in &stale {
            println!("gc --prune: 移除 stale 条目 {}（{}）", e.key, e.reason);
        }
        if stale.is_empty() {
            println!("gc --prune: 无 stale 条目");
        }
        let stale_keys: BTreeSet<&str> = stale.iter().map(|e| e.key.as_str()).collect();
        manifest.retain(|e| !stale_keys.contains(e.key.as_str()));
        manifest::save(&manifest)?;
        println!("gc --prune: manifest 原子重建（{} entries）", manifest.entries.len());
        return Ok(());
    }

    // --all：全量回收。
    let scan = list::scan();
    if has_active_sessions(&scan) {
        let ids: Vec<&str> = scan.iter().map(|s| s.id.as_str()).collect();
        bail!(
            "gc --all 拒绝：存在活跃会话（{}）——先正常退出会话（终态一致性前提）",
            ids.join(", ")
        );
    }
    let residue = list::sweep_residue();

    // 计划 + 守卫判定（全部通过或整体拒绝，不半执行）。
    let mut plan: Vec<String> = Vec::new();
    let mut refusals: Vec<String> = Vec::new();
    let mut mountpoint_reclaim: Vec<Entry> = Vec::new();
    for e in manifest.entries.iter().filter(|e| e.kind == EntryKind::Mountpoint) {
        let Some(path) = e.path.as_ref() else {
            refusals.push(format!("mountpoint {} 缺 path（清单损坏；--force 跳过守卫）", e.key));
            continue;
        };
        if !path.exists() {
            mountpoint_reclaim.push(e.clone());
            plan.push(format!("回收挂载点 {}（已在现实中消失，仅除名）", e.key));
            continue;
        }
        match path_facts(path) {
            Ok((is_dir, size, dir_nonempty, mtime)) => {
                if let Err(why) = reclaim_decision(is_dir, size, dir_nonempty, mtime, e.registered_at, opts.force) {
                    refusals.push(format!("mountpoint {} ：{why}", e.key));
                } else {
                    mountpoint_reclaim.push(e.clone());
                    plan.push(format!("回收挂载点 {}（{}）", e.key, path.display()));
                }
            }
            Err(e2) => refusals.push(format!("mountpoint {}：{e2}", e.key)),
        }
    }
    for g in &residue.gateways {
        plan.push(format!("SIGKILL 孤儿网关 pid={} kind={}", g.pid, g.kind));
    }
    for d in &residue.dirs {
        plan.push(format!("移除无主会话目录 sessions/{d}"));
    }
    let provider_dereg: Vec<Entry> = manifest
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::Provider)
        .cloned()
        .collect();
    for e in &provider_dereg {
        plan.push(format!(
            "provider 条目除名 {}（二进制永不删除：{}）",
            e.key,
            e.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default()
        ));
    }
    let profile_state_reclaim: Vec<Entry> = match opts.profile.as_deref() {
        Some(p) => manifest
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::ProfileState && entry_profile(e) == Some(p))
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    for e in &profile_state_reclaim {
        plan.push(format!("回收 profile-state {}（显式 --profile）", e.key));
    }
    if opts.profile.is_none() {
        let kept = manifest.entries.iter().filter(|e| e.kind == EntryKind::ProfileState).count();
        if kept > 0 {
            plan.push(format!("profile-state 条目 ×{kept} 保留（仅随显式 --profile 回收）"));
        }
    }

    if !refusals.is_empty() && !opts.force {
        for r in &refusals {
            eprintln!("gc --all 拒绝：{r}");
        }
        bail!("gc --all 未执行任何动作（{} 项拒绝；--force 可显式越过用户数据化守卫）", refusals.len());
    }

    if plan.is_empty() && refusals.is_empty() {
        println!("gc --all: 无可回收项（清单与现实已一致）");
        return Ok(());
    }
    confirm_or_abort(opts, &plan)?;

    // 执行：sweep 清理。
    for g in &residue.gateways {
        if !crate::ns::kill_pid(g.pid, libc::SIGKILL) {
            eprintln!("gc --all: 孤儿网关 pid={} 已不可杀（ESRCH/EPERM），跳过", g.pid);
        }
    }
    let sess_root = crate::session::sessions_root()?;
    for d in &residue.dirs {
        std::fs::remove_dir_all(sess_root.join(d))
            .with_context(|| format!("移除无主会话目录 sessions/{d}"))?;
    }
    // 执行：挂载点回收 + profile-state 回收（路径侧）。
    for e in &mountpoint_reclaim {
        if let Some(path) = e.path.as_ref() {
            if path.exists() {
                let is_dir = path.is_dir();
                remove_path(path, is_dir, opts.force)?;
            }
        }
    }
    for e in &profile_state_reclaim {
        if let Some(path) = e.path.as_ref() {
            if path.exists() {
                let is_dir = path.is_dir();
                remove_path(path, is_dir, opts.force)?;
            }
        }
    }
    // 执行：清单重建（mountpoint 回收 + provider 除名 + profile-state 条件回收）。
    let reclaimed_keys: BTreeSet<&str> = mountpoint_reclaim
        .iter()
        .chain(profile_state_reclaim.iter())
        .map(|e| e.key.as_str())
        .collect();
    manifest.retain(|e| match e.kind {
        EntryKind::Provider => false, // 只除名，二进制永不删除
        EntryKind::Mountpoint => !reclaimed_keys.contains(e.key.as_str()),
        EntryKind::ProfileState => !reclaimed_keys.contains(e.key.as_str()),
    });
    manifest::save(&manifest)?;

    // 终态断言：清单与现实一致（残留=0）。
    let after = list::sweep_residue();
    let residue_left = !after.gateways.is_empty() || !after.dirs.is_empty();
    println!("gc --all: manifest 原子重建（{} entries）", manifest.entries.len());
    for l in sweep_report(&after) {
        println!("gc --all 终态: {l}");
    }
    if residue_left {
        bail!("gc --all 终态不一致：sweep 残留未清零（见上方报告）");
    }
    println!("gc --all: 终态清单与现实一致（残留=0）");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(kind: EntryKind, key: &str) -> Entry {
        Entry {
            kind,
            key: key.into(),
            path: None,
            version: None,
            registered_at: 1_000,
            reason: "r".into(),
        }
    }

    fn profiles(names: &[&str]) -> BTreeMap<String, Profile> {
        names
            .iter()
            .map(|n| (n.to_string(), toml::from_str(&format!("egress = 'if:x{n}'")).unwrap()))
            .collect()
    }

    #[test]
    fn stale_detection_scopes_profile_kinds_only() {
        let entries = vec![
            e(EntryKind::Provider, "pasta"),
            e(EntryKind::Mountpoint, "gone:/tmp/a"),
            e(EntryKind::Mountpoint, "web:/tmp/b"),
            e(EntryKind::ProfileState, "gone:state"),
        ];
        let stale = stale_entries(&entries, &profiles(&["web"]));
        let keys: Vec<&str> = stale.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["gone:/tmp/a", "gone:state"], "provider 永不按 profile 判 stale");
    }

    #[test]
    fn active_session_refusal_ignores_forged_orphans() {
        let mk = |pid: u32, id: &str, kind: &'static str| list::Session {
            pid,
            id: id.into(),
            kind,
        };
        assert!(has_active_sessions(&[mk(1, "pasta-1", "cc")]));
        assert!(has_active_sessions(&[mk(2, "pasta-1", "gateway")]));
        assert!(!has_active_sessions(&[mk(3, "?", "gateway")]), "id=? 的网关指纹 = 孤儿，非活跃会话");
        assert!(!has_active_sessions(&[]));
    }

    #[test]
    fn reclaim_guard_user_data_refuses_without_force() {
        // 空文件 + mtime 未越过登记时刻 → 可回收
        assert!(reclaim_decision(false, 0, false, 900, 1_000, false).is_ok());
        // size ≠ 0 → 用户数据
        assert!(reclaim_decision(false, 7, false, 900, 1_000, false).is_err());
        // 目录非空 → 用户数据
        assert!(reclaim_decision(true, 0, true, 900, 1_000, false).is_err());
        // mtime > registered_at → 创建后被触碰
        assert!(reclaim_decision(false, 0, false, 1_001, 1_000, false).is_err());
        // mtime == registered_at（同毫秒创建）→ 未变
        assert!(reclaim_decision(false, 0, false, 1_000, 1_000, false).is_ok());
        // --force 越过
        assert!(reclaim_decision(false, 7, false, 1_001, 1_000, true).is_ok());
    }
}
