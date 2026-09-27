# 18 — mark 引擎实验：uid 策略路由 + mihomo TUN（spec 变更（五））

**What to build:** 引擎第二候选全链实证：setup 一次性装路由面（rootful，清单化）→ 会话零 netns、localhost 双向零摩擦、出口按 uid 走 mihomo TUN → fail-closed → gc 回收干净。

**Blocked by:** 无（sudo 可用性是硬前置——不可用则 blocked 交 operator）　**Owner:** 待派

## 实验设计（阶段 A：/tmp 一次性脚本 + sudo）

1. **TUN 化**：mihomo 开 `tun.enable, auto-route: false, auto-detect-interface: true`（转由我方路由）——重启 mihomo-sg-tmp 加 tun 段（root：/dev/net/tun）
2. **路由面**（rootful setup 形态，逐条留档 + 可回滚脚本）：
   - `ip rule add uidrange <U>-<U> lookup <N> pref <m>`（N=专用表，如 5182）
   - `ip route add default dev mihomo-tun table <N>`
   - 表内只有 tun 路由 → tun down = unreachable = **结构性 fail-closed**（无需黑洞规则，取证验证）
3. **uid 机制择型**（票面核心设计点，二选一取证）：
   - a) setup 建 subuid（usermod）+ newuidmap：会话 userns 映射 subuid→ns-root，全部进程宿主侧呈 subuid → 被 uidrange 命中；代价 = 依赖 setuid newuidmap（R5 语义放宽，登记）
   - b) file-cap 助手（setcap cap_setuid+ep on 小助手二进制）：setuid 到专用 uid 免 sudo；代价 = 二进制持久 capability（清单+doctor+gc 管理）
4. **验收面**：以该 uid 跑替身会话（无 netns！）：①`curl ifconfig.me` == SG IP；②`python3 -m http.server` 宿主 localhost 直达（零转发机制）；③tun down → 全断（fail-closed 取证）；④宿主其他用户/进程流量零影响；⑤回滚脚本 → 宿主 diff 为零（N3）

## 阶段 B（A 成立后）

config `engine = "netns"（默认）| "mark"`；setup 按 engine 分支装路由面并入清单；doctor 断言规则存在/表可达（规则丢失=Fail，原"静默 fail-open"缓解）；gc 回收；plan 展开；verify 套件引擎无关全绿

## Acceptance

- [ ] 阶段 A 五验收面全绿，全程留档 /tmp/iso-cc-exp18/（含回滚前后宿主 diff）
- [ ] uid 机制择型结论（a/b）+ 理由
- [ ] 阶段 B：`engine=mark` 全链 run+verify 绿（P13=SG、P8 后补）；engine=netns 不回归
- [ ] clippy -D warnings 绿；nextest 全绿

## 边界

- rootful 只出现在 setup 形态步骤（逐条可回滚脚本），不进会话路径；订阅/凭据不入仓库；`.scratch/**`、`docs/**` 只读；不 git；**不动 05 lane 文件集（config.rs/session.rs/setup.rs/doctor.rs/probe.rs/plan.rs/manifest.rs）——本票先纯实验（阶段 A 零仓库改动），阶段 B 待 05 合入后另行放行**
