#!/usr/bin/env bash
# sync-upstream.sh — 从上游镜像同步到魔改主分支
#
# 背景：
#   - upstream   = github.com/farion1231/cc-switch（原仓库）
#   - origin     = github.com/qianbkk/cc-switch（你 fork 的仓库）
#   - main       = 魔改区（曾经的 custom 分支），是 GitHub 默认分支
#   - upstream   = 上游镜像（曾经的 main 分支），跟 farion1231/main 同步
#   - 注意：本仓库 origin 与 remote upstream 都叫 upstream，分清语义:
#     路径写法 — 本地"上游镜像分支" = upstream;GH 上的远端仓库 = remote "upstream"
#
# 用法：
#   bash scripts/sync-upstream.sh
#
# 冲突处理：脚本默认 --ff-only 同步 upstream 分支（避免污染）
# 合并 main 分支时如果有冲突会停下来让你手动解决

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'

# ---- 0.5. 配置 origin fetch spec: 只跟踪 main + upstream 两个分支 ----
# 防止下次 sync fork(GitHub UI 或其他地方)把上游 100+ dev 分支带回 fork。
# 此操作幂等且只改本地 git config,不动远端。GitHub UI sync 行为不受 git config 控制,
# 因此脚本末尾另有一道"清 fork 上多余分支"的兜底逻辑。
EXPECTED_FETCH='+refs/heads/main:refs/remotes/origin/main
+refs/heads/upstream:refs/remotes/origin/upstream'
CURRENT_FETCH=$(git config --get-all remote.origin.fetch 2>/dev/null | tr '\n' ' ' | sed 's/ $//')
if [ "$CURRENT_FETCH" != "$(echo "$EXPECTED_FETCH" | tr '\n' ' ' | sed 's/ $//')" ]; then
  echo -e "${YELLOW}→ 配置 origin fetch: 只跟踪 main + upstream${NC}"
  git config remote.origin.url "$(git config remote.origin.url)"
  git config --unset-all remote.origin.fetch 2>/dev/null || true
  git config remote.origin.fetch '+refs/heads/main:refs/remotes/origin/main'
  git config --add remote.origin.fetch '+refs/heads/upstream:refs/remotes/origin/upstream'
fi

# ---- 0. 卫语句：working tree 必须 clean ----
if ! git diff --quiet HEAD; then
    echo -e "${RED}❌ working tree 有未提交修改${NC}"
    echo "   先 commit / stash / reset 后再跑本脚本"
    exit 1
fi

if ! git diff --cached --quiet HEAD; then
    echo -e "${RED}❌ 有已暂存但未提交的修改${NC}"
    echo "   先 commit / reset 后再跑本脚本"
    exit 1
fi

# ---- 1. 拉取 upstream 仓库与 origin fork ----
echo -e "${YELLOW}→ 拉取 upstream + origin${NC}"
git fetch upstream
git fetch origin

# ---- 2. 同步 upstream 分支(上游镜像)到 upstream/main(远端 farion1231)----
echo -e "${YELLOW}→ 切换到 upstream 分支(本地上游镜像)${NC}"
git checkout upstream

# 只在 fast-forward 时合并；否则说明 upstream 已经被改过，需手动处理
if ! git merge upstream/main --ff-only; then
    echo -e "${RED}❌ upstream 不是 upstream/main 的快进${NC}"
    echo "   说明 upstream 分支上有额外的 commit。需要先确定是合并冲突还是其他情况"
    echo "   运行 git log upstream --oneline -5 和 git log upstream/main --oneline -5 对比"
    exit 2
fi

echo -e "${YELLOW}→ 推送 upstream 到 origin${NC}"
git push origin upstream

# ---- 3. 把 upstream 合并到 main(魔改主分支)----
echo -e "${YELLOW}→ 切换到 main 分支(魔改工作区)${NC}"
git checkout main

# 不自动 commit，冲突时停下来
if ! git merge upstream --no-edit; then
    echo -e "${RED}❌ 合并冲突，需要手动解决${NC}"
    echo ""
    echo "接下来手动操作："
    echo "  1. 编辑冲突文件（git status 看列表）"
    echo "  2. git add <解决冲突的文件>"
    echo "  3. git commit （会自动完成 merge）"
    echo "  4. 跑这个脚本的 --push 阶段："
    echo "     bash scripts/sync-upstream.sh --push-only"
    echo ""
    echo "或者放弃这次合并："
    echo "  git merge --abort"
    exit 3
fi

# ---- 4. 推送到 origin ----
echo -e "${YELLOW}→ 推送 main 到 origin${NC}"
git push origin main

echo ""
echo -e "${GREEN}✅ 同步完成${NC}"
echo "   upstream: 现在 = farion1231/main(同步干净)"
echo "   main:     现在 = upstream + 你的 main 提交"
echo "   origin:   已同步推送"

# ---- 5. 提示：建议发 PR 给上游 ----
echo ""
echo "💡 提示：如果你想给上游发 PR（比如 bug 修复、功能）"
echo "   gh pr create --base farion1231:main --head qianbkk:main"
echo "   建议 PR 标题前缀用 [fix] / [feat] 等明确类型"

# ---- 6. 兜底:清掉 fork 上来自上游的非 main/upstream 分支 ----
# 防止任何路径(GitHub UI sync、其它 fork 工具)把那 100+ dev 分支带回来。
# 这一步只动你 fork qianbkk/cc-switch 上**不是** main 也不是 upstream 的分支。
# 安全确认: gh api 删除接口对 main 和 upstream 路径跳过了(见 grep -v),这两个分支不会被影响。
# GitHub 对每个 ref 单独 422 静默处理,失败的不打印噪声。
echo ""
echo "🧹 兜底:清理 fork 上来自上游的多余分支(保留 main + upstream)..."

if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
  # 拉 fork 上当前所有分支
  EXTRA_BRANCHES=$(gh api repos/qianbkk/cc-switch/branches --paginate 2>/dev/null \
    | python -c "import json,sys; data=json.load(sys.stdin); print('\n'.join(b['name'] for b in data if b['name'] not in ('main','upstream')))" 2>/dev/null || true)

  if [ -z "$EXTRA_BRANCHES" ]; then
    echo "   fork 上只有 main + upstream,无需清理"
  else
    COUNT=0
    FAILED=0
    while IFS= read -r branch; do
      [ -z "$branch" ] && continue
      # URL-encode 路径里的 /
      enc=$(printf '%s' "$branch" | sed 's|/|%2F|g')
      code=$(gh api -X DELETE --silent --include \
        "repos/qianbkk/cc-switch/git/refs/heads/$enc" 2>&1 \
        | head -1 | grep -oE 'HTTP/[0-9.]+ [0-9]+' | grep -oE '[0-9]+$' || echo "0")
      if [ "$code" = "204" ] || [ "$code" = "200" ]; then
        COUNT=$((COUNT+1))
      else
        FAILED=$((FAILED+1))
      fi
    done <<< "$EXTRA_BRANCHES"
    echo -e "   ${GREEN}✅ 已删除 $COUNT 个 fork 分支${NC}"
    [ "$FAILED" -gt 0 ] && echo "   ($FAILED 个因有 PR 关联跳过,可手动清理)"
  fi
else
  echo "   (跳过 — 需要 gh CLI 且已登录 qianbkk)"
fi
