# 03 — 最小 rootless 会话（locale + env + exec）

**What to build:** `iso-cc run -- <cmd>`：userns+mountns 建立、locale bind 覆盖（`/etc/localtime` 符号链接解析后绑真实路径 + `/etc/timezone`）、TZ/LANG/LC_* 注入、exec 目标命令、信号收割。kill -9 会话根进程后宿主状态 diff 为空。

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] 会话内 `date +%Z`、`locale`、`readlink /etc/localtime`（真实路径）、Node `Intl` 四视线一致
- [ ] kill -9 后 `/proc/mountinfo`、`lsns`、`~/.claude*` diff 为空
- [ ] 挂载点缺失时预创建 + doctor 报告
