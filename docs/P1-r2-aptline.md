# P1 — R2 APT registry（原主 tracker 21-ppa-publishing.md；编号已迁 P 前缀）：aptline 发布接口（重写：原 PPA binary-repack 路线作废存档）

**What to build:** 自建 APT registry（Cloudflare R2 托管）+ 可复用发布接口 `aptline`。用户裁决 2026-09-27：放弃 PPA（源码包/binary-repack 灰色路线），改自建正规二进制 deb 仓库——**nfpm 产出的 deb 直接入池，无需源码包**。

**Blocked by:** 无　**Owner:** w12:p1（packaging 面）

> 工作位置：worktree packaging/ppa（本仓）。依赖降级注：peer 传称用户裁决 Depends 仅 passt / slirp4netns→Suggests / tun2proxy→Recommends——与我方票 08 亲验记录（passt+slirp4netns>=1.2.0）冲突，**待操作者确认前保留 peer 版本不覆盖**。

## 用户已确认的三点

1. 凭据集：R2 API Token（三元组）+ GPG 签名 key（专用，我方生成、用户分发公钥）+（可选）自定义域名；GitHub secrets 承载 CI 发布
2. 单 registry 多包：pool/ + dists/，component = 项目命名空间，多版本 pool 共存
3. 沉淀接口 = 独立 `aptline` 仓库（工具 + composite action），iso-cc 为首个消费者，其他项目一行接入

## Specification

1. **`aptline` 独立仓库**（新，本票创建于 workspace 兄弟目录，后续推用户 GitHub）：
   - `aptline publish --suite stable --component <proj> --debs <glob> --r2 <profile> --key <fingerprint>`
   - 内部：dpkg-deb 元数据抽取 → Packages 索引 → InRelease 签名（GPG）→ 幂等上传（rclone/R2 S3 API；同版本覆盖语义 = 覆盖+重签）
   - pool 布局 `pool/main/<首字母>/<pkg>/`；dists/stable/<component>/；**多版本共存**
   - 本地与 CI 同一工具；无凭据时 `--dry-run` 全链可审（本票交付形态）
2. **GitHub composite action**（`action/action.yml`）：inputs = component/debs-glob/suite + secrets（R2 三元组、GPG 私钥 armored、passphrase）；内部调 aptline
3. **iso-cc 接线**：release.yml 加 publish job（消费 composite action；凭据缺失时 job skip + Warn——凭据到位即活，零改码）
4. **客户端文档**：CLIENT.md 一页（curl 公钥 + `deb https://<domain> stable <component>` + apt install）
5. **本票交付边界**：aptline 工具 + action + iso-cc 接线全部按 **dry-run/本地文件系统模拟 R2** 完成并验收；真实 R2 冒烟等用户凭据（wizard 项，登记）

## Acceptance

- [ ] aptline 本地文件系统后端：两个合成 deb 入池 → Packages/InRelease 生成且 gpg 验签通过（本地测试 key）→ 第二个包入池后索引含两包（多包共存证明）
- [ ] 幂等：同版本重复 publish → 覆盖 + 重签，索引无重复条目
- [ ] composite action 语法/输入契约正确（act 或结构断言）
- [ ] iso-cc release.yml publish job 就位（无凭据 skip + Warn 路径实测）
- [ ] clippy（若 rust）/shell 严格模式绿；既有 141 测试不回归

## 轮子盘点

索引生成自研（dpkg-deb + POSIX，~200 行）vs aptly（重、服务化）vs arx（单二进制 aptify，19/研究会话提及，nixpkgs 可用性待查）——lane 先查 arx/`nixpkgs` 再定，自研兜底。rclone（nixpkgs）为 R2 上传层。GPG=gnupg。

## 边界

- 订阅/私有信息不入仓库；真凭据只在用户 secrets；`.scratch/**`、`docs/**` 只读；不 git（aptline 仓库的 git init 允许——它是新独立仓库）

## 实测补记（2026-09-27）：S3 凭据无法从 CF API token 派生

- `POST /accounts/{account}/tokens` 可程序化创建 R2 权限的 CF API token（Workers R2 Storage Bucket Item Write，scope=bucket），但 **(token id, token value) ≠ S3 SigV4 凭据**：rclone SigV4 实测 `SignatureDoesNotMatch`；GET token 对象无 accessKey 字段
- temp-access-credentials API 依赖**已存在的 R2 parent token**（parent_access_key_id + secret），cfat_ CF token 不能当 parent → 鸡生蛋
- **结论**：R2 S3 凭据（Access Key ID/Secret）只能 Dashboard 一次性创建（R2 → Manage API Tokens → Create API Token，Object Read & Write，scope bucket apt）。此后可用该 parent 派生 temp creds 供 CI 每次发版铸造短期凭据（密钥不常驻 CI）
