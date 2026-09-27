//! setup.rs —— setup-manifested 资源收敛循环（票 14；设计稿 design-session-lanes §5，
//! spec 变更（一））。幂等：重跑 action diff = 0。
//!
//! ① provider 收敛：which + `--version` → 绝对路径+版本 upsert；缺失报包名，不代装。
//! ② 挂载点收敛：redirect 集内缺失者 mkdir/touch + 登记（只登记 setup 创建的——
//!    预存在路径非本工具所有，gc 不得回收）；`/etc/timezone` bind 不在收敛集
//!    （R2 视线 = TZ env + /etc/localtime + Intl，不依赖该文件）。
//! ③ manifest 重建 + 原子落盘（manifest::save）。
//!
//! run 期契约随本命令切换（cutover）：会话过程零持久物，挂载点缺失 = fail-loud。

use crate::config::Profile;
use crate::manifest::{self, Entry, EntryKind, Manifest};
use anyhow::{anyhow, Context};
use std::path::Path;

/// provider 条目登记理由（指名 R/ADR）。
const REASON_PROVIDER: &str = "R11/D7 provider 钉路径（票 14 清单态，替换 09 期 which 过渡）";
/// mountpoint 条目登记理由（N3 例外①移至 setup 期）。
const REASON_MOUNTPOINT: &str = "N3 例外① setup 期创建并登记（spec 变更（一）setup-manifested，票 14）";

/// 单条收敛动作（人类可读行 + `--json` 数组元素共用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Action {
    pub target: String,
    pub detail: String,
}

/// mountpoint key：`<profile>:<dst>`（stale 判定 + gc --profile 定界的定界符）。
pub fn mountpoint_key(profile_name: &str, dst: &str) -> String {
    format!("{profile_name}:{dst}")
}

/// 挂载点创建形态：bind src 是目录 → mkdir -p(dst)；否则 touch 空文件。
/// （bind(2) 要求 src/dst 同型；src 缺失按文件处理，bind 期错误由 run fail-loud。）
fn create_mountpoint(src: &Path, dst: &Path) -> anyhow::Result<&'static str> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)
            .with_context(|| format!("创建挂载点目录 {}", dst.display()))?;
        Ok("mkdir")
    } else {
        std::fs::write(dst, b"").with_context(|| format!("创建挂载点文件 {}", dst.display()))?;
        Ok("touch")
    }
}

/// 收敛循环主体（设计稿 §5 setup 三步）。返回 (动作清单, 条目数)。
pub fn converge(profile_name: &str, profile: &Profile) -> anyhow::Result<(Vec<Action>, usize)> {
    let mut manifest = manifest::read()?.unwrap_or_else(Manifest::empty);
    let mut actions: Vec<Action> = Vec::new();

    // ① provider 收敛：which + --version → 绝对路径+版本 upsert（仅事实变化时——
    //    registered_at 是 mountpoint 守卫基准，provider 无此依赖，但幂等要求 diff=0）。
    //    socks5 形态（工单 16）额外收敛 tun2proxy worker（清单 key 与 bin 名解耦）。
    let mut wanted: Vec<(&str, Vec<&str>)> = vec![(profile.gateway().bin_name(), vec![profile.gateway().bin_name()])];
    if matches!(profile.egress(), Ok(crate::config::Egress::Socks5 { .. })) {
        wanted.push((crate::provider::socks::WORKER_KEY, crate::provider::socks::BIN_CANDIDATES.to_vec()));
    }
    for (key, candidates) in wanted {
        let path = crate::provider::which_any(&candidates).ok_or_else(|| {
            anyhow!(
                "setup：provider {key} 不在 PATH（egress={}）；缺失报包名：passt / slirp4netns / nixpkgs#tun2proxy 或上游静态单文件——setup 不代装",
                profile.egress
            )
        })?;
        let version = crate::provider::version_output(&path)?;
        let changed = manifest
            .find(EntryKind::Provider, key)
            .is_none_or(|e| e.path.as_deref() != Some(path.as_path()) || e.version.as_deref() != Some(version.as_str()));
        if changed {
            manifest.upsert(Entry {
                kind: EntryKind::Provider,
                key: key.to_string(),
                path: Some(path.clone()),
                version: Some(version.clone()),
                registered_at: manifest::now_millis(),
                reason: REASON_PROVIDER.into(),
            });
            actions.push(Action {
                target: format!("provider {key}"),
                detail: format!("{}（{version}）", path.display()),
            });
        }
    }

    // ② 挂载点收敛：redirect 集内缺失 mkdir/touch + 登记（insert_new_only 保留原
    //    registered_at = gc 用户数据化守卫基准；已存在路径不纳管）。
    for r in &profile.redirect {
        let (src, dst) = r
            .split_once('=')
            .ok_or_else(|| anyhow!("redirect 必须是 `src=dst`：{r:?}"))?;
        let src = Path::new(src);
        let dst = Path::new(dst);
        if !dst.exists() {
            let how = create_mountpoint(src, dst)?;
            let key = mountpoint_key(profile_name, &dst.to_string_lossy());
            manifest.insert_new_only(Entry {
                kind: EntryKind::Mountpoint,
                key,
                path: Some(dst.to_path_buf()),
                version: None,
                registered_at: manifest::now_millis(),
                reason: REASON_MOUNTPOINT.into(),
            });
            actions.push(Action {
                target: format!("mountpoint {}", dst.display()),
                detail: format!("{how}（src={}）", src.display()),
            });
        }
    }

    // ③ manifest 重建 + 原子写。
    manifest::save(&manifest).context("manifest 原子落盘")?;
    Ok((actions, manifest.entries.len()))
}

/// `iso-cc setup` 入口：收敛 + 报告（action diff = 动作数；幂等重跑 = 0）。
pub fn run(profile_name: &str, profile: &Profile, json: bool) -> anyhow::Result<()> {
    let (actions, entries) = converge(profile_name, profile)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "profile": profile_name,
                "actions": actions,
                "action_count": actions.len(),
                "entries": entries,
            }))?
        );
    } else {
        for a in &actions {
            println!("setup[{profile_name}]: {} → {}", a.target, a.detail);
        }
        println!(
            "setup[{profile_name}]: manifest 原子重建（{entries} entries）；action diff = {}",
            actions.len()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mountpoint_key_scopes_profile() {
        assert_eq!(mountpoint_key("web", "/tmp/x"), "web:/tmp/x");
        let key = mountpoint_key("web", "/tmp/x");
        let (profile, dst) = key.split_once(':').unwrap();
        assert_eq!((profile, dst), ("web", "/tmp/x"));
    }

    #[test]
    fn converge_is_idempotent_on_manifest_fact() {
        // 纯事实核：同 path/version 的 provider 条目不产生动作（幂等的最小证明单元；
        // 全循环的 fs 面由冒烟覆盖）。
        let mut m = Manifest::empty();
        let e = Entry {
            kind: EntryKind::Provider,
            key: "pasta".into(),
            path: Some("/x/pasta".into()),
            version: Some("v1".into()),
            registered_at: 1,
            reason: REASON_PROVIDER.into(),
        };
        m.upsert(e.clone());
        let changed = m
            .find(EntryKind::Provider, "pasta")
            .is_none_or(|e| e.path.as_deref() != Some(Path::new("/x/pasta")) || e.version.as_deref() != Some("v1"));
        assert!(!changed, "事实未变不得产生动作（diff=0 前提）");
        m.upsert(Entry {
            version: Some("v2".into()),
            registered_at: 2,
            ..e
        });
        assert_eq!(m.find(EntryKind::Provider, "pasta").unwrap().version.as_deref(), Some("v2"));
    }
}
