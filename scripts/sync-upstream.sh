#!/usr/bin/env bash
# sync-upstream.sh — 从 upstream/main 拉取更新并合并到 custom 分支
#
# 背景：
#   - upstream = github.com/farion1231/cc-switch （原仓库）
#   - origin   = github.com/qianbkk/cc-switch    （你 fork 的仓库）
#   - main     = 干净跟踪 upstream/main
#   - custom   = 所有魔改在此分支
#
# 用法：
#   bash scripts/sync-upstream.sh
#
# 冲突处理：
#   脚本默认 --ff-only 同步 main（避免污染 main）
#   合并 custom 时如果有冲突会停下来让你手动解决（提供明确指引）

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'

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

# ---- 1. 拉取 upstream 和 origin ----
echo -e "${YELLOW}→ 拉取 upstream + origin${NC}"
git fetch upstream
git fetch origin

# ---- 2. 同步 main 到 upstream/main ----
echo -e "${YELLOW}→ 切换到 main 分支${NC}"
git checkout main

# 只在 fast-forward 时合并；否则说明 main 已经被改过，需手动处理
if ! git merge upstream/main --ff-only; then
    echo -e "${RED}❌ main 不是 upstream/main 的快进${NC}"
    echo "   说明 main 分支上有额外的 commit。需要先确定是合并冲突还是其他情况"
    echo "   运行 git log main --oneline -5 和 git log upstream/main --oneline -5 对比"
    exit 2
fi

echo -e "${YELLOW}→ 推送 main 到 origin${NC}"
git push origin main

# ---- 3. 把 main 合并到 custom ----
echo -e "${YELLOW}→ 切换到 custom 分支${NC}"
git checkout custom

# 不自动 commit，冲突时停下来
if ! git merge main --no-edit; then
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
echo -e "${YELLOW}→ 推送 custom 到 origin${NC}"
git push origin custom

echo ""
echo -e "${GREEN}✅ 同步完成${NC}"
echo "   main:   现在 = upstream/main + 你的 main 提交"
echo "   custom: 现在 = main + 之前 custom 的所有提交"
echo "   origin: 已同步推送"

# ---- 5. 提示：建议发 PR 给上游 ----
echo ""
echo "💡 提示：如果你想给上游发 PR（比如 bug 修复、功能）"
echo "   gh pr create --base farion1231:main --head qianbkk:custom"
echo "   建议 PR 标题前缀用 [fix] / [feat] 等明确类型"