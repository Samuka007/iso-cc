//! manifest.rs —— setup-manifested 资源清单（票 14；设计稿 design-session-lanes §5，spec 变更（一））。
//!
//! 路径/属主：`~/.local/state/iso-cc/manifest.json`（user 属主；v1 setup 全程 rootless）。
//! nix profile 同构三机制（CompScan #8，只取模式不耦合 store）：
//! 1. 版本字段 + 不认识的版本即报错 → [`SCHEMA`] fail-loud（[`parse`]）；
//! 2. 整清单重建后原子落盘 → [`save_to`]（tempfile 同目录 persist，无就地编辑）；
//! 3. 代际 diff → doctor 双向校验的展示形态（doctor.rs）/ setup action diff（setup.rs）。
//!
//! entries 分类（audit-code-facts §10 汇总）：provider（钉路径，gc 只除名）、
//! mountpoint（setup 创建的挂载点，gc 按 size/mtime 守卫回收）、profile-state
//! （按 profile 的持久态，v1 无生产者；仅随显式 `--profile` 回收）。

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 清单 schema 版本（fail-loud：不认识的版本即报错，nix profile.cc:144-145 同构）。
pub const SCHEMA: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u64,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryKind {
    /// 网关 provider 钉路径（pasta / slirp4netns）。gc 永不删二进制，只除名。
    Provider,
    /// setup 创建的挂载点（redirect dst）。key = `<profile>:<dst>`。
    Mountpoint,
    /// 按 profile 的持久态（预留）。key = `<profile>:<名>`；仅随显式 --profile 回收。
    ProfileState,
    /// mark 引擎 file-cap 助手（票 18：持久 capability = setup-manifested）。
    /// key = `<profile>:uidrun`；path = 二进制绝对路径；gc 实删文件（本工具所有）。
    CapBin,
    /// mark 引擎策略路由面（票 18：uidrange rule + 表 5182 双路由 + unreachable
    /// 兜底 + v6 镜像）。key = `<profile>:route-rule`；path = None（宿主路由原语，
    /// 常量面见 config::MARK_*）；回收 = 除名 + 打印 rootful 回滚命令（flush 表 +
    /// del 规则，/tmp/iso-cc-exp18/rollback.sh 同源）。
    RouteRule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub kind: EntryKind,
    /// provider = bin 名；mountpoint/profile-state = `<profile>:<路径或名>`。
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// provider `--version` 输出（漂移 = doctor Warn）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// 登记时刻（unix 毫秒）。mountpoint 的 gc 用户数据化守卫基准。
    pub registered_at: u64,
    /// 指名 R/ADR 的登记理由（两级模型审计通道）。
    pub reason: String,
}

impl Manifest {
    pub fn empty() -> Self {
        Self {
            schema: SCHEMA,
            entries: Vec::new(),
        }
    }

    pub fn find(&self, kind: EntryKind, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.kind == kind && e.key == key)
    }

    /// 同 (kind, key) 替换（provider upsert：仅 path/version 变化时调用方才应调）。
    pub fn upsert(&mut self, entry: Entry) {
        match self
            .entries
            .iter()
            .position(|e| e.kind == entry.kind && e.key == entry.key)
        {
            Some(i) => self.entries[i] = entry,
            None => self.entries.push(entry),
        }
    }

    /// 同 (kind, key) 已存在则不动（mountpoint：保留原 registered_at，gc 守卫基准不可漂移）。
    pub fn insert_new_only(&mut self, entry: Entry) -> bool {
        if self.find(entry.kind, &entry.key).is_some() {
            return false;
        }
        self.entries.push(entry);
        true
    }

    pub fn retain(&mut self, f: impl FnMut(&Entry) -> bool) {
        self.entries.retain(f);
    }
}

/// 状态根：`~/.local/state/iso-cc/`（sessions/ 同根，session::sessions_root 派生一致）。
pub fn state_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    Path::new(&home).join(".local/state/iso-cc")
}

pub fn manifest_path() -> PathBuf {
    state_dir().join("manifest.json")
}

/// 当前时刻（unix 毫秒；registered_at 基准）。
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("时钟早于 epoch")
        .as_millis() as u64
}

/// 读取清单：None = 文件缺失（setup 未跑）；Err = 损坏/schema 未知（fail-loud）。
pub fn read() -> anyhow::Result<Option<Manifest>> {
    read_from(&manifest_path())
}

pub fn read_from(path: &Path) -> anyhow::Result<Option<Manifest>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context(format!("读取清单 {}", path.display())),
    };
    parse(&text, path).map(Some)
}

/// 解析 + schema fail-loud（#1 机制：不认识的版本即报错）。
pub fn parse(text: &str, path: &Path) -> anyhow::Result<Manifest> {
    let m: Manifest = serde_json::from_str(text).with_context(|| {
        format!(
            "解析清单 {}（fail-loud：结构未知，deny_unknown_fields）",
            path.display()
        )
    })?;
    if m.schema != SCHEMA {
        bail!(
            "清单 schema = {} 不支持（本工具支持 {SCHEMA}）：{}（fail-loud）",
            m.schema,
            path.display()
        );
    }
    Ok(m)
}

/// 整清单重建 + 原子落盘（#2 机制：tempfile 同目录 persist，无就地编辑）。
pub fn save(m: &Manifest) -> anyhow::Result<()> {
    save_to(m, &manifest_path())
}

pub fn save_to(m: &Manifest, path: &Path) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("清单路径无父目录：{}", path.display()))?;
    std::fs::create_dir_all(dir).with_context(|| format!("创建清单目录 {}", dir.display()))?;
    let mut f = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("tempfile 创建于 {}", dir.display()))?;
    serde_json::to_writer_pretty(&mut f, m).context("序列化清单")?;
    use std::io::Write as _;
    f.flush().context("flush 清单")?;
    let _ = f.as_file().sync_all();
    f.persist(path)
        .map_err(|e| {
            anyhow::anyhow!(
                "清单原子落盘失败（persist {}）：{}",
                path.display(),
                e.error
            )
        })
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: EntryKind, key: &str, path: Option<&str>, version: Option<&str>) -> Entry {
        Entry {
            kind,
            key: key.into(),
            path: path.map(PathBuf::from),
            version: version.map(Into::into),
            registered_at: 1_760_000_000_000,
            reason: "R11/D7 provider 钉路径（票 14）".into(),
        }
    }

    #[test]
    fn roundtrip_and_kind_naming() {
        let mut m = Manifest::empty();
        m.upsert(entry(
            EntryKind::Provider,
            "pasta",
            Some("/x/pasta"),
            Some("v1"),
        ));
        m.insert_new_only(entry(
            EntryKind::Mountpoint,
            "p:/tmp/a",
            Some("/tmp/a"),
            None,
        ));
        m.insert_new_only(entry(
            EntryKind::ProfileState,
            "p:state",
            Some("/tmp/s"),
            None,
        ));
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"profile-state\""), "{json}");
        assert!(json.contains("\"schema\":1"), "{json}");
        let back: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn unknown_schema_fails_loud() {
        let err = parse(r#"{"schema":99,"entries":[]}"#, Path::new("/x")).unwrap_err();
        assert!(err.to_string().contains("schema = 99"), "{err}");
    }

    #[test]
    fn unknown_field_rejected() {
        let err = parse(r#"{"schema":1,"entries":[],"extra":1}"#, Path::new("/x")).unwrap_err();
        // anyhow 顶层 Display = context；deny_unknown 的事实在错误链里（{:#} 全链）
        assert!(format!("{err:#}").contains("unknown field"), "{err:#}");
    }

    #[test]
    fn missing_file_is_none_not_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_from(&dir.path().join("absent.json"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn atomic_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("manifest.json");
        let mut m = Manifest::empty();
        m.upsert(entry(
            EntryKind::Provider,
            "slirp4netns",
            Some("/x/s"),
            Some("1.3.5"),
        ));
        save_to(&m, &p).unwrap();
        let back = read_from(&p).unwrap().unwrap();
        assert_eq!(back, m);
        // 原子写不遗留 tempfile
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "仅 manifest.json 自身");
    }

    #[test]
    fn upsert_replaces_and_insert_new_only_preserves() {
        let mut m = Manifest::empty();
        assert!(m.insert_new_only(entry(EntryKind::Mountpoint, "p:/a", Some("/a"), None)));
        assert!(!m.insert_new_only(entry(EntryKind::Mountpoint, "p:/a", Some("/a"), None)));
        m.upsert(entry(
            EntryKind::Provider,
            "pasta",
            Some("/new"),
            Some("v2"),
        ));
        assert_eq!(m.entries.len(), 2);
        assert_eq!(
            m.find(EntryKind::Provider, "pasta")
                .unwrap()
                .version
                .as_deref(),
            Some("v2")
        );
    }

    #[test]
    fn key_format_profile_prefixed() {
        // mountpoint/profile-state 的 key 必须带 profile 前缀（stale 判定 + gc --profile 定界）
        let e = entry(EntryKind::Mountpoint, "web:/tmp/x", Some("/tmp/x"), None);
        let (profile, _) = e.key.split_once(':').unwrap();
        assert_eq!(profile, "web");
    }
}
