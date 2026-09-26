# 06 — 泄漏与状态不变式验收矩阵 + iso-cc verify

**What to build:** P1–P15+P3b 探针（纯净度）产品化为 `iso-cc verify`：会话内一键红绿 + JSON；退出码 0/1/2（SKIP 不记 PASS）；裁决语义「环境与声明一致」+ 固定不覆盖面声明；P5 拔线 opt-in；P9 跑真 `claude doctor`。工具面子矩阵 P16–P27 按登记基线追加。

**Blocked by:** 03, 04, 05

**Status:** ready-for-agent

- [ ] 4090 + 本机全绿，可 CI 重放
- [ ] 退出码语义经故障注入验证
