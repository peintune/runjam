#!/usr/bin/env bash
#
# make-dmg.sh — 把已签名的 .app 打包成带 /Applications 拖拽快捷方式的 DMG。
#
# 为什么要这个脚本：
#   Tauri 2 自带的 dmg 打包（等价于 `hdiutil create -srcfolder RunJam.app`）只会把
#   .app 拷进镜像，不会创建 Applications 软链接。用户挂载 DMG 后只能看到一个
#   RunJam.app，没有"拖到 Applications 安装"的目标图标。
#   做法：先把 .app 和指向 /Applications 的软链接放进一个 staging 目录，再整体打包。
#
# 用法:
#   bash scripts/make-dmg.sh <app路径> <输出dmg路径> [背景图png]
#
# 第 3 个参数可选：传入背景图且本机装有 create-dmg 时，会生成带背景图 + 图标定位
# 的美化 DMG；否则（默认）用纯 hdiutil 打包 —— 功能完整，只是没有背景图。
# 想启用美化版，在 CI 里准备一张 660x400 左右的 PNG 并传第三个参数即可。
#
set -euo pipefail

APP_SRC="${1:-}"
DMG_OUT="${2:-}"
BACKGROUND="${3:-}"
VOLNAME="${DMG_VOLNAME:-RunJam}"

if [[ -z "$APP_SRC" || -z "$DMG_OUT" ]]; then
  echo "用法: bash scripts/make-dmg.sh <app> <output.dmg> [background.png]" >&2
  exit 1
fi

if [[ ! -d "$APP_SRC" ]]; then
  echo "❌ 找不到 app: $APP_SRC" >&2
  exit 1
fi

mkdir -p "$(dirname "$DMG_OUT")"
rm -f "$DMG_OUT"

STAGE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/runjam-dmg.XXXXXX")"
trap 'rm -rf "$STAGE_DIR"' EXIT

# ── 1) 组装 staging：RunJam.app + Applications 软链接 ──────────────
cp -R "$APP_SRC" "$STAGE_DIR/"
ln -s /Applications "$STAGE_DIR/Applications"

build_with_hdiutil() {
  echo "📦 hdiutil create -srcfolder (RunJam.app + Applications)"
  hdiutil create \
    -volname "$VOLNAME" \
    -srcfolder "$STAGE_DIR" \
    -ov \
    -format UDZO \
    "$DMG_OUT"
}

# ── 2) 打包（可选美化路径，失败自动回退）────────────────────────────
if [[ -n "$BACKGROUND" && -f "$BACKGROUND" ]] && command -v create-dmg >/dev/null 2>&1; then
  echo "🎨 create-dmg（背景图: $BACKGROUND）"
  CREATE_OUT="$STAGE_DIR/create-dmg-out"
  mkdir -p "$CREATE_OUT"
  if create-dmg \
      --overwrite \
      --dmg-title "$VOLNAME" \
      --dmg-background "$BACKGROUND" \
      --no-code-sign \
      "$APP_SRC" "$CREATE_OUT"; then
    # create-dmg 的文件名由它自己决定，这里统一 rename 成我们要的名字
    BUILT_DMG="$(find "$CREATE_OUT" -maxdepth 1 -name '*.dmg' | head -n 1)"
    if [[ -n "$BUILT_DMG" ]]; then
      mv "$BUILT_DMG" "$DMG_OUT"
    else
      echo "⚠️ create-dmg 未产出 dmg，回退 hdiutil"
      build_with_hdiutil
    fi
  else
    echo "⚠️ create-dmg 执行失败，回退 hdiutil"
    build_with_hdiutil
  fi
else
  build_with_hdiutil
fi

# ── 3) 校验：挂载确认 Applications 链接确实在镜像里 ──────────────────
MOUNT_POINT="$STAGE_DIR/mount"
mkdir -p "$MOUNT_POINT"
if hdiutil attach -readonly -nobrowse -mountpoint "$MOUNT_POINT" "$DMG_OUT" >/dev/null 2>&1; then
  echo "🔍 DMG 内容:"
  ls -l "$MOUNT_POINT"
  if [[ -L "$MOUNT_POINT/Applications" ]]; then
    echo "✅ Applications 拖拽快捷方式已就位"
  else
    echo "⚠️ 警告: 镜像内未找到 Applications 链接"
  fi
  hdiutil detach "$MOUNT_POINT" >/dev/null 2>&1 || true
else
  echo "⚠️ 无法挂载镜像做校验（CI 环境可能受限），跳过内容检查"
fi

echo "✅ DMG 生成完成: $DMG_OUT"
ls -lh "$DMG_OUT"
