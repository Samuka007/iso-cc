#!/bin/sh
# 工单 27：任意发行版一键安装/卸载（curl-bash 快速路径的落盘脚本）。
#
# bundle 布局（packaging/bundle.sh 产出）解包即用：
#   <PREFIX>/bin/iso-cc                 主二进制
#   <PREFIX>/lib/iso-cc/libexec/        pasta / slirp4netns / tun2proxy
# 该布局由 src/provider/mod.rs 的 exe-relative 解析层（第三层）零配置命中——
# 无 setup、无 PATH 依赖、无 ISO_CC_HELPER_DIR。
#
# 用法：
#   ./install.sh install [VERSION]      # 从 GitHub Release 拉 bundle（缺省 = 最新 release）
#   ./install.sh install --from PATH    # 离线：本地 bundle 目录或 .tar.gz
#   ./install.sh uninstall
#
# 环境：PREFIX（默认 /usr/local）——测试一律 PREFIX=/tmp/... 覆盖，勿真写系统前缀。
# 幂等：重复安装覆盖同路径。完整性：Release 模式经 sha256sums.txt 强校验（fail-loud）。

set -eu

REPO="${ISO_CC_REPO:-Samuka007/iso-cc}"
PREFIX="${PREFIX:-/usr/local}"
BIN_DIR="$PREFIX/bin"
LIBEXEC_DIR="$PREFIX/lib/iso-cc/libexec"

log() { printf '== install.sh: %s\n' "$*"; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

usage() {
	cat <<EOF
用法: $0 install [VERSION]      # 从 GitHub Release 安装（缺省 = 最新 release）
      $0 install --from PATH    # 离线安装：本地 bundle 目录（含 iso-cc + libexec/）或 .tar.gz
      $0 uninstall              # 卸载（提示用户态 gc 清理）

环境: PREFIX=/usr/local   二进制 -> \$PREFIX/bin，helper -> \$PREFIX/lib/iso-cc/libexec
EOF
}

fetch() { # <url> <out-file>
	if command -v curl >/dev/null 2>&1; then
		curl -fSL --retry 3 -sS -o "$2" "$1"
	elif command -v wget >/dev/null 2>&1; then
		wget -q -O "$2" "$1"
	else
		die "缺少 curl 或 wget——请先安装其一"
	fi
}

sha256_of() { # <file> -> stdout（hex）
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | cut -d' ' -f1
	elif command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | cut -d' ' -f1
	else
		die "缺少 sha256sum 或 shasum——完整性校验无法进行（fail-loud）"
	fi
}

detect_arch() {
	m=$(uname -m)
	case "$m" in
	x86_64) printf 'amd64' ;;
	aarch64 | arm64) printf 'arm64' ;;
	*) die "不支持的架构：$m（bundle 发布 amd64/arm64）" ;;
	esac
}

latest_version() { # -> stdout（tag 去掉 v 前缀）
	api="https://api.github.com/repos/$REPO/releases/latest"
	if command -v curl >/dev/null 2>&1; then
		rel=$(curl -fsSL "$api") || die "获取最新 release 失败：$api（网络/限流？可显式传 VERSION 或用 --from）"
	else
		rel=$(wget -qO- "$api") || die "获取最新 release 失败：$api"
	fi
	ver=$(printf '%s\n' "$rel" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
	[ -n "$ver" ] || die "解析 tag_name 失败：$api"
	case "$ver" in
	v*) ver=${ver#v} ;;
	esac
	printf '%s\n' "$ver"
}

install_payload() { # <stage-dir>：含 iso-cc + libexec/ 的 bundle 根
	[ -f "$1/iso-cc" ] || die "bundle 布局缺 iso-cc：$1"
	[ -d "$1/libexec" ] || die "bundle 布局缺 libexec/：$1"
	mkdir -p "$BIN_DIR" || die "无法创建 $BIN_DIR（权限不足？用 sudo，或 PREFIX=... 覆盖）"
	install -m 0755 "$1/iso-cc" "$BIN_DIR/iso-cc"
	# 整目录替换：重复安装幂等，旧 helper 集不残留
	rm -rf "$LIBEXEC_DIR"
	mkdir -p "$LIBEXEC_DIR"
	n=0
	for f in "$1"/libexec/*; do
		[ -f "$f" ] || continue
		install -m 0755 "$f" "$LIBEXEC_DIR/${f##*/}"
		n=$((n + 1))
	done
	[ "$n" -gt 0 ] || die "bundle libexec/ 为空：$1/libexec"
	log "已安装 $BIN_DIR/iso-cc + $LIBEXEC_DIR/（$n 个 helper）"
	log "自证："
	"$BIN_DIR/iso-cc" --version || die "自证失败：$BIN_DIR/iso-cc --version 异常"
}

install_release() { # <version>
	arch=$(detect_arch)
	base="https://github.com/$REPO/releases/download/v$1"
	# 资产名两代并存：bundle.sh 的 git describe 保留 tag 的 v 前缀（v0.1.0 实发形态），
	# 票面预期为无前缀——两者依次尝试，先命中先用。
	asset=""
	for name in \
		"iso-cc-bundle-$1-linux-$arch.tar.gz" \
		"iso-cc-bundle-v$1-linux-$arch.tar.gz"; do
		log "下载 $base/$name"
		if fetch "$base/$name" "$tmp/bundle.tar.gz" &&
			fetch "$base/sha256sums.txt" "$tmp/sha256sums.txt"; then
			asset=$name
			break
		fi
	done
	[ -n "$asset" ] || die "v$1 release 无可下载 bundle（网络/限流/资产缺失？）"
	want=$(awk -v a="$asset" '$2 == a || $2 == "*"a { print $1 }' "$tmp/sha256sums.txt")
	[ -n "$want" ] || die "sha256sums.txt 无 $asset 条目（release 产物不完整？）"
	got=$(sha256_of "$tmp/bundle.tar.gz")
	[ "$got" = "$want" ] || die "sha256 不符：$asset 期望 $want 实得 $got（下载损坏或上游漂移——fail-loud）"
	log "sha256 校验通过：$got"
	tar -xzf "$tmp/bundle.tar.gz" -C "$tmp"
	stage=$(echo "$tmp"/iso-cc-bundle-*)
	[ -d "$stage" ] || die "tar.gz 解包后无 iso-cc-bundle-*/ 根目录"
	install_payload "$stage"
}

install_from() { # <path>：bundle 目录或 tar.gz（离线模式，绕过网络）
	case "$1" in
	*.tar.gz | *.tgz)
		[ -f "$1" ] || die "tar.gz 不存在：$1"
		tar -xzf "$1" -C "$tmp"
		stage=$(echo "$tmp"/iso-cc-bundle-*)
		[ -d "$stage" ] || die "tar.gz 解包后无 iso-cc-bundle-*/ 根目录：$1"
		install_payload "$stage"
		;;
	*)
		[ -d "$1" ] || die "--from 需为 bundle 目录（含 iso-cc + libexec/）或 .tar.gz：$1"
		install_payload "$1"
		;;
	esac
}

cmd_uninstall() {
	removed=0
	if [ -e "$BIN_DIR/iso-cc" ]; then
		rm -f "$BIN_DIR/iso-cc"
		log "已删除 $BIN_DIR/iso-cc"
		removed=1
	fi
	if [ -d "$LIBEXEC_DIR" ]; then
		rm -rf "$LIBEXEC_DIR"
		log "已删除 $LIBEXEC_DIR"
		removed=1
	fi
	rmdir "$PREFIX/lib/iso-cc" 2>/dev/null || true
	[ "$removed" -eq 1 ] || log "未发现已安装文件（$BIN_DIR/iso-cc、$LIBEXEC_DIR）——幂等退出"
	if command -v iso-cc >/dev/null 2>&1; then
		log "用户态清理提示：iso-cc gc --all（回收 session/profile-state；有活跃会话会拒绝）"
	else
		log "用户态清理提示：会话/profile-state 残留在 ~/.local/state/iso-cc/（重装后可 iso-cc gc --all）"
	fi
}

main() {
	cmd=${1:-}
	[ -n "$cmd" ] || {
		usage
		exit 1
	}
	shift

	command -v tar >/dev/null 2>&1 || die "缺少 tar——请先安装"
	command -v install >/dev/null 2>&1 || die "缺少 install（coreutils/busybox）——请先安装"

	case "$cmd" in
	install)
		from=""
		ver=""
		while [ $# -gt 0 ]; do
			case "$1" in
			--from)
				[ $# -ge 2 ] || die "--from 需要路径参数"
				from=$2
				shift 2
				;;
			--from=*)
				from=${1#--from=}
				shift
				;;
			-h | --help)
				usage
				exit 0
				;;
			v*)
				ver=${1#v}
				shift
				;;
			-*)
				die "未知选项：$1（见 --help）"
				;;
			*)
				ver=$1
				shift
				;;
			esac
		done
		tmp=$(mktemp -d) || die "mktemp 失败"
		trap 'rm -rf "$tmp"' EXIT
		trap 'exit 130' INT
		trap 'exit 143' TERM
		if [ -n "$from" ]; then
			[ -z "$ver" ] || die "--from 与 VERSION 不能同用"
			install_from "$from"
		else
			[ -n "$ver" ] || ver=$(latest_version)
			log "目标版本：$ver（PREFIX=$PREFIX）"
			install_release "$ver"
		fi
		;;
	uninstall)
		cmd_uninstall
		;;
	-h | --help)
		usage
		;;
	*)
		usage
		die "未知命令：$cmd"
		;;
	esac
}

main "$@"
