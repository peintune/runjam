#!/usr/bin/env bash
#
# release.sh — 一键发布：add → commit → push → (tag → push tags → 写入版本元数据)
#
# commit message 必填，且须为 Conventional Commits 格式：它会写入版本元数据并
# 作为更新说明展示给用户，因此不接受 "fix" / "update" 这类空泛内容。未提供 -m
# 时在终端交互询问；非交互（CI）环境必须显式传 -m。
#
# 用法:
#   ./release.sh                # 交互输入 commit + patch 递增 (v1.0.23 → v1.0.24)
#   ./release.sh -m "feat(update): add mirror download links"  # 指定 commit
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
# commit message 不设默认值：它会被写入版本元数据（publish_metadata）并作为更新
# 说明展示在用户的更新弹窗里，"fix" 这类内容对用户毫无信息量。
COMMIT_MSG=""
# dry-run 且未传 -m 时使用占位 message，此时跳过校验。
MSG_PLACEHOLDER=false
TAG_MODE="patch"      # patch | minor | major
CUSTOM_TAG=""
DRY_RUN=false
PUSH_ONLY=false
CUSTOM_DOWNLOAD_URLS=""

usage() {
  # 动态提取文件头的注释块（去掉 "# " 前缀），增删注释行无需维护行号。
  awk 'NR>1 && /^#/ { print substr($0,3); next } NR>1 { exit }' "$0"
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

# ── 同步版本号到 Cargo.toml / tauri.conf.json / package.json ──
# Tauri 2 编译产物的权威版本来自 src-tauri/Cargo.toml 的 [package] version
# （决定 dmg/exe 文件名、应用内 getVersion()），tauri.conf.json 的 version
# 必须与之完全一致（不一致会编译报错）；package.json 的 version 用于 CI 产物
# 命名（RunJam_${VERSION}_*.dmg）与 tauri-action。三者必须与 tag 对齐。
bump_version() {
  local ver="${1#v}"
  node -e '
    const fs = require("fs");
    const ver = process.argv[1];

    // 1) Cargo.toml：只改 [package] 段的 version，不影响依赖的 version 字段
    let cargo = fs.readFileSync("src-tauri/Cargo.toml", "utf8");
    const lines = cargo.split("\n");
    let inPkg = false;
    for (let i = 0; i < lines.length; i++) {
      const t = lines[i].trim();
      if (t.startsWith("[")) inPkg = t.startsWith("[package]") || t.startsWith("[package.");
      else if (inPkg && /^version\s*=/.test(t)) {
        lines[i] = t.replace(/"[^"]*"/, "\"" + ver + "\"");
        break;
      }
    }
    fs.writeFileSync("src-tauri/Cargo.toml", lines.join("\n"));

    // 2) tauri.conf.json（与 Cargo.toml 保持一致，Tauri 2 校验二者相同）
    // 3) package.json（CI 产物命名）
    for (const f of ["src-tauri/tauri.conf.json", "package.json"]) {
      const j = JSON.parse(fs.readFileSync(f, "utf8"));
      j.version = ver;
      fs.writeFileSync(f, JSON.stringify(j, null, 2) + "\n");
    }
    console.log("  version -> " + ver + " (Cargo.toml / tauri.conf.json / package.json)");
  ' "$ver"
}

# ── commit message 校验 ───────────────────────────
# commit message 会被写入版本元数据的 notes 字段，并作为更新说明展示在用户的
# 更新弹窗里，因此必须描述具体改动，不能是 "fix" / "update" 这类空泛内容。
CONVENTIONAL_TYPES="feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert"
VAGUE_SUBJECTS="fix|fixes|update|updates|updated|change|changes|changed|test|tests|wip|tmp|temp|minor|patch|stuff|things|misc|code|cleanup|a|asdf"

validate_commit_msg() {
  local msg="$1"
  local subject="${msg%%$'\n'*}"
  if [[ ! "$subject" =~ ^(${CONVENTIONAL_TYPES})(\([a-zA-Z0-9_./-]+\))?!?:[[:space:]].+$ ]]; then
    echo "❌ commit message 不符合 Conventional Commits 格式：" >&2
    echo "   <type>[(scope)][!]: <subject>" >&2
    echo "   type 取值: ${CONVENTIONAL_TYPES//|/, }" >&2
    echo "   示例: fix(session): discard stale retry timers after a turn finishes" >&2
    echo "   你的输入: $subject" >&2
    return 1
  fi
  local body="${subject#*: }"
  if (( ${#body} < 8 )); then
    echo "❌ subject 只有 ${#body} 个字符，请写清改了什么（至少 8 个字符）" >&2
    return 1
  fi
  local lower
  lower="$(printf '%s' "$body" | tr '[:upper:]' '[:lower:]')"
  if [[ "|$VAGUE_SUBJECTS|" == *"|$lower|"* ]]; then
    echo "❌ subject \"$body\" 过于空泛：它会作为更新说明展示给用户，请描述具体改动" >&2
    return 1
  fi
  return 0
}

# 未提供 -m 时在终端询问，并列出当前改动，方便写出准确描述。
prompt_commit_msg() {
  echo "💬 请输入 commit message（Conventional Commits 格式）："
  echo "   <type>[(scope)]: <subject>   例: feat(update): add mirror download links"
  echo "   本次改动："
  git status --porcelain | head -12 | sed 's/^/     /'
  IFS= read -r -p "> " COMMIT_MSG
}

# 决定最终的 commit message：显式 -m > 交互输入 > 报错退出（非交互环境必填）。
resolve_commit_msg() {
  [[ -n "$COMMIT_MSG" ]] && return 0
  # dry-run 不产生 commit，不打扰输入，用占位符走完流程。
  if [[ "$DRY_RUN" == true ]]; then
    COMMIT_MSG="<commit message required (-m)>"
    MSG_PLACEHOLDER=true
    return 0
  fi
  if [[ -t 0 ]]; then
    prompt_commit_msg
  fi
  if [[ -z "$COMMIT_MSG" ]]; then
    echo "❌ 缺少 commit message：用 -m \"feat(scope): what changed\" 指定，或在交互模式下输入。" >&2
    echo "   （commit message 会作为更新说明展示给用户，不接受 'fix' 这类空泛内容）" >&2
    exit 1
  fi
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
  # 只取首行（subject）作为更新说明：多行 message 的正文不适合塞进弹窗。
  notes=$(json_escape "${COMMIT_MSG%%$'\n'*}")
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
resolve_commit_msg
# dry-run 也校验显式传入的 message，方便在真正发布前就发现格式问题。
if [[ "$MSG_PLACEHOLDER" != true ]]; then
  validate_commit_msg "$COMMIT_MSG" || exit 1
fi

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
if [[ "$PUSH_ONLY" != true ]]; then
  # 发布前同步版本号，保证产物命名 / 应用内版本 / 下载直链一致
  if [[ "$DRY_RUN" == true ]]; then
    echo "  ▷ 更新 package.json / src-tauri/tauri.conf.json version -> ${NEW_TAG#v}"
  else
    echo "🔢 同步版本号 -> ${NEW_TAG#v}"
    bump_version "$NEW_TAG"
  fi
fi

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
