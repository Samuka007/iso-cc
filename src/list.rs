use serde::Serialize;

const MARKER: &[u8] = b"ISO_CC_SESSION=";

#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub pid: u32,
    pub id: String,
}

/// 扫描 /proc 中带 ISO_CC_SESSION 标记的进程（网关进程）。
/// 状态零持久：会话死 = 标记消失 = list 自然收敛。
pub fn scan() -> Vec<Session> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return out;
    };
    for e in entries.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let env = std::fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
        if env.is_empty() {
            continue;
        }
        if let Some(id) = env
            .split(|b| *b == 0)
            .filter(|kv| !kv.is_empty())
            .find_map(|kv| kv.strip_prefix(MARKER))
            .map(|s| String::from_utf8_lossy(s).into_owned())
        {
            out.push(Session { pid, id });
        }
    }
    out.sort_by_key(|s| s.pid);
    out
}

pub fn render_human(sessions: &[Session]) -> String {
    if sessions.is_empty() {
        return "no live sessions\n".into();
    }
    let mut s = format!("{:>7}  {}\n", "PID", "SESSION");
    for x in sessions {
        s.push_str(&format!("{:>7}  {}\n", x.pid, x.id));
    }
    s
}
