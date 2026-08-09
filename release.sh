#!/usr/bin/env bash
#
# release.sh — 一键发布：add → commit → push → tag → push tags
#
# 默认: commit "fix" + patch 递增 (v1.0.23 → v1.0.24)
#
# 用法:
#   ./release.sh                # fix + v1.0.24
#   ./release.sh -m "feat: xxx"  # 自定义 commit
#   ./release.sh -M              # minor 递增 (v1.0.23 → v1.1.0)
#   ./release.sh -j              # major 递增 (v1.0.23 → v2.0.0)
#   ./release.sh -t v2.0.0       # 指定 tag
#   ./release.sh -n              # dry-run，只显示不执行
#   ./release.sh -h              # 帮助
#
set -euo pipefail

# ── 默认配置 ──────────────────────────────────────
COMMIT_MSG="fix"
TAG_MODE="patch"      # patch | minor | major
CUSTOM_TAG=""
DRY_RUN=false

usage() {
  sed -n '3,15p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
}

while getopts ":m:t:Mjnh" opt; do
  case "$opt" in
    m) COMMIT_MSG="$OPTARG" ;;
    t) CUSTOM_TAG="$OPTARG" ;;
    M) TAG_MODE="minor" ;;
    j) TAG_MODE="major" ;;
    n) DRY_RUN=true ;;
    h) usage ;;
    *) echo "未知选项: -$OPTARG"; usage ;;
  esac
done

# ── 计算新 tag ────────────────────────────────────
LATEST_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "")

if [[ -n "$CUSTOM_TAG" ]]; then
  # 自动补 v 前缀
  if [[ "$CUSTOM_TAG" == v* ]]; then
    NEW_TAG="$CUSTOM_TAG"
  else
    NEW_TAG="v$CUSTOM_TAG"
  fi
elif [[ -z "$LATEST_TAG" ]]; then
  NEW_TAG="v0.1.0"
else
  BASE="${LATEST_TAG#v}"
  IFS='.' read -r MAJOR MINOR PATCH <<< "$BASE"
  MAJOR=${MAJOR:-0}; MINOR=${MINOR:-0}; PATCH=${PATCH:-0}
  case "$TAG_MODE" in
    patch) PATCH=$((PATCH + 1)) ;;
    minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
    major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
  esac
  NEW_TAG="v${MAJOR}.${MINOR}.${PATCH}"
fi

# 检查 tag 是否已存在
if git rev-parse "$NEW_TAG" >/dev/null 2>&1; then
  echo "❌ tag $NEW_TAG 已存在，请指定其他版本号 (-t)"
  exit 1
fi

# ── 执行函数 ──────────────────────────────────────
run() {
  if [[ "$DRY_RUN" == true ]]; then
    echo "  ▷ $*"
  else
    "$@"
  fi
}

# ── 信息确认 ──────────────────────────────────────
echo "📦 当前 tag: ${LATEST_TAG:-无}"
echo "🏷️  新 tag:  $NEW_TAG"
echo "💬 commit:  $COMMIT_MSG"
[[ "$DRY_RUN" == true ]] && echo "🔍 dry-run 模式（不实际执行）"
echo ""

# ── 执行发布流程 ──────────────────────────────────
if [[ -z "$(git status --porcelain)" ]]; then
  echo "⚠️  工作区无改动，跳过 add/commit"
else
  run git add .
  run git commit -m "$COMMIT_MSG"
fi

run git push
run git tag "$NEW_TAG"
run git push origin --tags

if [[ "$DRY_RUN" != true ]]; then
  echo ""
  echo "✅ 发布完成: $NEW_TAG"
  echo "   GitHub Actions 将自动触发构建: https://github.com/nicepkg/runjam/actions"
fi
