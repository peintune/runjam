#!/usr/bin/env bash
#
# release.sh — 一键发布：add → commit → push → (tag → push tags → 写入版本元数据)
#
# 默认: commit "fix" + patch 递增 (v1.0.23 → v1.0.24)，打 tag 并推送，
#       触发 GitHub Actions 构建（build.yml 只在 tag v* 时构建）。
#
# 用法:
#   ./release.sh                # fix + v1.0.24（完整发布）
#   ./release.sh -m "feat: xxx"  # 自定义 commit
#   ./release.sh -M              # minor 递增 (v1.0.23 → v1.1.0)
#   ./release.sh -j              # major 递增 (v1.0.23 → v2.0.0)
#   ./release.sh -t v2.0.0       # 指定 tag
#   ./release.sh -p              # 只 push（不 tag / 不触发构建 / 不写版本库）
#   ./release.sh -d '{"linux_x64":"https://..."}'  # 自定义 download_urls（默认自动生成各平台直链）
#   ./release.sh -n              # dry-run，只显示不执行
#   ./release.sh -h              # 帮助
#
# 环境变量（可选）:
#   RELEASES_API_URL     写入版本元数据的接口，默认 https://www.runjam.app/api/releases
#   RELEASES_ADMIN_TOKEN 接口鉴权 token（未设置则跳过写入并提示）
#
set -euo pipefail

# ── 默认配置 ──────────────────────────────────────
COMMIT_MSG="fix"
TAG_MODE="patch"      # patch | minor | major
CUSTOM_TAG=""
DRY_RUN=false
PUSH_ONLY=false
CUSTOM_DOWNLOAD_URLS=""

usage() {
  sed -n '3,19p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
}

while getopts ":m:t:d:Mjpnh" opt; do
  case "$opt" in
    m) COMMIT_MSG="$OPTARG" ;;
    t) CUSTOM_TAG="$OPTARG" ;;
    d) CUSTOM_DOWNLOAD_URLS="$OPTARG" ;;
    M) TAG_MODE="minor" ;;
    j) TAG_MODE="major" ;;
    p) PUSH_ONLY=true ;;
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

# 检查 tag 是否已存在（仅发布模式需要 tag）
if [[ "$PUSH_ONLY" != true ]] && git rev-parse "$NEW_TAG" >/dev/null 2>&1; then
  echo "❌ tag $NEW_TAG 已存在，请指定其他版本号 (-t)"
  exit 1
fi

# ── 解析 GitHub 仓库路径（用于构造下载直链）──────────
REMOTE_URL=$(git remote get-url origin 2>/dev/null || echo "https://github.com/peintue/runjam.git")
REPO_PATH=$(echo "$REMOTE_URL" | sed -E 's#(https?://[^/]+/|git@[^:]+:)([^/]+/[^/.]+)(\.git)?$#\2#')
REPO_PATH="${REPO_PATH:-peintune/runjam}"

# ── 构造 download_urls（GitHub Releases 直链，文件名可预测）──
build_download_urls() {
  local tag="$1"
  if [[ -n "$CUSTOM_DOWNLOAD_URLS" ]]; then
    echo "$CUSTOM_DOWNLOAD_URLS"
    return
  fi
  local ver="${tag#v}"
  local base="https://github.com/${REPO_PATH}/releases/download/${tag}"
  echo "{\"macos_aarch64\":\"${base}/RunJam_${ver}_aarch64.dmg\",\"macos_x86_64\":\"${base}/RunJam_${ver}_x64.dmg\",\"windows_x64\":\"${base}/RunJam_${ver}_x64-setup.exe\"}"
}

# ── 写入版本元数据到 Supabase（经 Vercel 接口）──────
# 简单 JSON 转义（处理 commit message 中的引号/反斜杠）
json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

publish_metadata() {
  local tag="$1"
  local api_url="${RELEASES_API_URL:-https://www.runjam.app/api/releases}"
  local token="${RELEASES_ADMIN_TOKEN:-}"

  if [[ -z "$token" ]]; then
    echo "⚠️  未设置 RELEASES_ADMIN_TOKEN，跳过写入版本元数据（在 Vercel 环境变量中配置后即可自动写入）"
    return 0
  fi

  local dl notes
  dl=$(build_download_urls "$tag")
  notes=$(json_escape "$COMMIT_MSG")
  local payload
  payload=$(printf '{"version":"%s","notes":"%s","github_url":"https://github.com/%s/releases/tag/%s","download_urls":%s}' \
    "$tag" "$notes" "$REPO_PATH" "$tag" "$dl")

  echo "📝 写入版本元数据 → $api_url"
  local http_code body body_file
  body_file=$(mktemp)
  # -L 跟随重定向；--post301/302/303 保证重定向后仍是 POST（curl 默认会把
  # 301/302/303 降级为 GET 导致请求体丢失）。
  http_code=$(curl -sS -L --post301 --post302 --post303 \
    -o "$body_file" -w "%{http_code}" \
    -X POST "$api_url" \
    -H "Content-Type: application/json" \
    -H "x-admin-token: $token" \
    -d "$payload") || { echo "❌ 请求失败（网络/超时）"; rm -f "$body_file"; return 1; }
  body=$(cat "$body_file")
  rm -f "$body_file"
  echo "   HTTP $http_code: $body"
  if [[ "$http_code" -lt 200 || "$http_code" -ge 300 ]] || ! echo "$body" | grep -q '"ok":true'; then
    echo "❌ 写入版本元数据失败"
    return 1
  fi
  echo "✅ 版本元数据已写入 Supabase releases 表"
}

# ── 执行函数 ──────────────────────────────────────
run() {
  if [[ "$DRY_RUN" == true ]]; then
    echo "  ▷ $*"
  else
    "$@"
  fi
}

# ── 信息确认 ──────────────────────────────────────
if [[ "$PUSH_ONLY" == true ]]; then
  echo "📦 模式: 只 push（不打 tag、不触发构建、不写版本库）"
else
  echo "📦 当前 tag: ${LATEST_TAG:-无}"
  echo "🏷️  新 tag:  $NEW_TAG"
fi
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

if [[ "$PUSH_ONLY" == true ]]; then
  if [[ "$DRY_RUN" != true ]]; then
    echo ""
    echo "✅ push 完成（未打 tag，未触发构建）"
  fi
  exit 0
fi

run git tag "$NEW_TAG"
run git push origin --tags

if [[ "$DRY_RUN" != true ]]; then
  publish_metadata "$NEW_TAG"
  echo ""
  echo "✅ 发布完成: $NEW_TAG"
  echo "   GitHub Actions 将自动触发构建: https://github.com/$REPO_PATH/actions"
fi
