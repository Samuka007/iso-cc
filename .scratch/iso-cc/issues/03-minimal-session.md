# 03 — 最小 rootless 会话（locale + env + exec）

**What to build:** `iso-cc run -- <cmd>`：userns+mountns 建立、locale bind 覆盖（`/etc/localtime` 符号链接解析后绑真实路径 + `/etc/timezone`）、TZ/LANG/LC_* 注入、exec 目标命令、信号收割。kill -9 会话根进程后宿主状态 diff 为空。

**Blocked by:** 02

**Status:** done（2026-09-26，本机验收）

- [x] 会话内 locale 视线一致（P6a glibc `%z` == tzdb 偏移；P6b Node Intl == 声明；P6c bind 因宿主 /etc 不可预创建转 SKIP——tzdb 内嵌 TZif 路线覆盖）
- [x] kill/正常退出后 netns/mountns 随进程树消亡；PDEATHSIG(SIGKILL) 兜底；挂载点缺失预创建 = N3 有界例外（doctor 输出登记）
- [ ] 挂载点缺失时预创建 + doctor 报告
