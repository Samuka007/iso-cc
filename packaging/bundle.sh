#!/usr/bin/env bash
# 工单 08 bundle 形态：静态 iso-cc + 上游静态 helper（pasta/slirp4netns/tun2proxy）
# 打成 tar.gz，helper 置 libexec/ 相对目录——解决 jammy 等无 passt 发行版的缺口
# （ADR 0007 预案；票 14 manifest 钉路径机制原样复用，ISO_CC_HELPER_DIR 补 env 层）。
#
# 用法：
#   nix develop -c packaging/bundle.sh            # CI（tag checkout：describe = 精确 tag）
#   ISO_CC_BUNDLE_OUT=/tmp/x packaging/bundle.sh  # 本地（需 direnv 环境的 cargo+musl target）
#
# 产出：$ISO_CC_BUNDLE_OUT/iso-cc-bundle-<version>-linux-amd64.tar.gz
#   tar 根 = iso-cc-bundle-<version>/{iso-cc, libexec/{pasta,slirp4netns,tun2proxy}}
#
# 供应链：所有 helper sha256 钉死在 HELPER_PINS，下载后强校验（fail-loud，
# 绝不静默接受上游漂移；上游更新 → 校验失败 → 人工复钉并附版本注记）。

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
OUT_DIR="${ISO_CC_BUNDLE_OUT:-packaging/dist}"
CACHE_DIR="$OUT_DIR/cache"
STAGE_ROOT="$OUT_DIR/stage"
ARCH="amd64"

# 版本与 build.rs 同源同语义：git describe --tags --dirty，无 git/无 tag 回落 Cargo.toml。
VERSION="$(git describe --tags --dirty 2>/dev/null \
  || sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)"
[ -n "$VERSION" ] || { echo "bundle: 版本解析失败（fail-loud）" >&2; exit 1; }

command -v cargo >/dev/null || { echo "bundle: cargo 不在 PATH——用 nix develop -c 运行" >&2; exit 1; }

# ---- 1) 静态单文件 iso-cc（musl；D5 跨发行版单文件） -------------------------
echo "== bundle: cargo build (x86_64-unknown-linux-musl) =="
cargo build --release --target x86_64-unknown-linux-musl
BIN="target/x86_64-unknown-linux-musl/release/iso-cc"

# 注入一致性守卫：二进制 --version 必须与脚本版本同源（build.rs 注入 vs 脚本 describe）。
BIN_VER="$("$BIN" --version)"
[ "$BIN_VER" = "iso-cc $VERSION" ] || {
  echo "bundle: 版本不一致（fail-loud）：二进制 '$BIN_VER' vs 脚本 'iso-cc $VERSION'" >&2
  exit 1
}

# ---- 2) 上游静态 helper：下载缓存 + sha256 钉定校验 --------------------------
# pasta：passt.top 官方静态单文件（rolling builds/latest；当前钉定 build =
# 2026_07_28.f8df3f1-18-gdf90211）。上游更新会使校验失败——属预期 fail-loud。
# slirp4netns：官方 release 静态 x86_64（与官方 SHA256SUMS 逐一核对过）。
# tun2proxy：官方 musl zip（issue 16 择型版本 0.8.3）。
HELPER_PINS=(
  "https://passt.top/builds/latest/x86_64/pasta|pasta-x86_64|73320bcc96eaad082660d57d25228e1230f252d9d1fcc7a55c52bedbd0c64385"
  "https://github.com/rootless-containers/slirp4netns/releases/download/v1.3.5/slirp4netns-x86_64|slirp4netns-x86_64|8e54132bc80fc60d53af4b544dae63a81151774b56f129e572f7f1a2e89a57cf"
  "https://github.com/tun2proxy/tun2proxy/releases/download/v0.8.3/tun2proxy-x86_64-unknown-linux-musl.zip|tun2proxy-x86_64-unknown-linux-musl.zip|17a784e88b7b533984d9f4d83a20f9a9311678f27548c6850b57bbc29bbbf604"
)

fetch_verify() { # <url> <cache-name> <sha256>
  local url="$1" name="$2" want="$3" file="$CACHE_DIR/$name"
  if [ ! -f "$file" ]; then
    mkdir -p "$CACHE_DIR"
    echo "== bundle: 下载 $name =="
    curl -fSL --retry 3 -o "$file.part" "$url"
    mv "$file.part" "$file"
  fi
  echo "$want  $file" | sha256sum -c - >/dev/null || {
    echo "bundle: sha256 校验失败（fail-loud，供应链钉定）：$name——上游已漂移则人工复钉 HELPER_PINS" >&2
    exit 1
  }
  echo "== bundle: sha256 OK $name"
}

STAGE="$STAGE_ROOT/iso-cc-bundle-$VERSION"
rm -rf "$STAGE"
mkdir -p "$STAGE/libexec"

for pin in "${HELPER_PINS[@]}"; do
  IFS='|' read -r url name sha <<<"$pin"
  fetch_verify "$url" "$name" "$sha"
done

# ---- 3) 装配 stage ------------------------------------------------------------
install -m 0755 "$BIN" "$STAGE/iso-cc"
install -m 0755 "$CACHE_DIR/pasta-x86_64" "$STAGE/libexec/pasta"
install -m 0755 "$CACHE_DIR/slirp4netns-x86_64" "$STAGE/libexec/slirp4netns"

# tun2proxy：zip 内二进制名 tun2proxy-bin（与 nixpkgs 同名）→ 装配为清单 key 名 tun2proxy
#（src/provider/socks.rs BIN_CANDIDATES 首选名；ISO_CC_HELPER_DIR/PATH 两层均按候选名命中）。
T2P_ZIP="$CACHE_DIR/tun2proxy-x86_64-unknown-linux-musl.zip"
T2P_DIR="$CACHE_DIR/tun2proxy-unpacked"
rm -rf "$T2P_DIR" && mkdir -p "$T2P_DIR"
if command -v unzip >/dev/null; then
  unzip -q "$T2P_ZIP" -d "$T2P_DIR"
else
  python3 -c 'import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' \
    "$T2P_ZIP" "$T2P_DIR"
fi
install -m 0755 "$T2P_DIR/tun2proxy-bin" "$STAGE/libexec/tun2proxy"

# ---- 4) tar.gz -----------------------------------------------------------------
TARBALL="$OUT_DIR/iso-cc-bundle-$VERSION-linux-$ARCH.tar.gz"
mkdir -p "$(dirname "$TARBALL")"
tar -C "$STAGE_ROOT" -czf "$TARBALL" "iso-cc-bundle-$VERSION"
echo "== bundle: $TARBALL"
sha256sum "$TARBALL"
