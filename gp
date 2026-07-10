#!/usr/bin/env bash
set -euo pipefail

# gpush: add + commit + push อย่างฉลาด
# ใช้:  gpush -m "msg"         # commit ด้วยข้อความแล้ว push
#       gpush                   # ถ้าไม่ใส่ -m จะใช้ข้อความวันที่อัตโนมัติ
# ออปชัน:
#   -m "msg"       ข้อความ commit
#   -r origin      ระบุ remote (ดีฟอลต์: origin)
#   -b branch      ระบุสาขาที่จะ push (ดีฟอลต์: สาขาปัจจุบัน)
#   -n             --no-verify  (ข้าม hooks)
#   -a             --amend --no-edit (แก้ commit ล่าสุด)
#   -t vX.Y.Z      สร้าง tag แล้ว push tag ด้วย
#   -f             --force-with-lease
#   -h             แสดงวิธีใช้

usage() {
  grep '^# ' "$0" | sed 's/^# //'
  exit 0
}

[[ "${1:-}" == "-h" ]] && usage

MSG=""
REMOTE="origin"
BRANCH=""
NO_VERIFY=0
AMEND=0
TAG=""
FORCE=0

while getopts ":m:r:b:at:nfh" opt; do
  case "$opt" in
    m) MSG="$OPTARG" ;;
    r) REMOTE="$OPTARG" ;;
    b) BRANCH="$OPTARG" ;;
    a) AMEND=1 ;;
    t) TAG="$OPTARG" ;;
    n) NO_VERIFY=1 ;;
    f) FORCE=1 ;;
    h) usage ;;
    \?) echo "Unknown option: -$OPTARG" >&2; usage ;;
    :)  echo "Option -$OPTARG requires an argument." >&2; exit 1 ;;
  esac
done

# 1) อยู่ใน git repo ไหม
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "Error: ไม่ได้อยู่ใน Git repository" >&2
  exit 1
fi

# 2) ตรวจ config พื้นฐาน (แจ้งเตือนเฉยๆ)
if ! git config user.name >/dev/null; then
  echo "Note: ยังไม่ได้ตั้ง git user.name (git config --global user.name \"ชื่อคุณ\")"
fi
if ! git config user.email >/dev/null; then
  echo "Note: ยังไม่ได้ตั้ง git user.email (git config --global user.email you@example.com)"
fi

# 3) remote มีไหม (ถ้าไม่มีและมี env GIT_REMOTE_URL อยู่ จะเพิ่มให้อัตโนมัติ)
if ! git remote get-url "$REMOTE" >/dev/null 2>&1; then
  if [[ -n "${GIT_REMOTE_URL:-}" ]]; then
    echo "Adding remote '$REMOTE' -> $GIT_REMOTE_URL"
    git remote add "$REMOTE" "$GIT_REMOTE_URL"
  else
    echo "Warning: ไม่พบ remote '$REMOTE' (ตั้งด้วย: git remote add $REMOTE <url>)"
  fi
fi

# 4) สาขาปัจจุบัน
if [[ -z "$BRANCH" ]]; then
  BRANCH=$(git symbolic-ref --quiet --short HEAD 2>/dev/null || echo "")
  if [[ -z "$BRANCH" ]]; then
    echo "Error: ไม่ได้อยู่บนสาขา (detached HEAD). ระบุด้วย -b <branch>" >&2
    exit 1
  fi
fi

# 5) commit message
if [[ -z "$MSG" ]]; then
  MSG="chore: update $(date -Iseconds)"
fi

# 6) เตือนถ้ามีไฟล์ใหญ่กว่า 100MB (กันพุชขึ้น GitHub ไม่ได้)
LARGE=$(find . -path ./.git -prune -o -type f -size +100M -print | head -n 1 || true)
if [[ -n "$LARGE" ]]; then
  echo "Warning: พบไฟล์ใหญ่กว่า 100MB: $LARGE  (GitHub ปกติไม่อนุญาต)">&2
fi

# 7) add + commit (เฉพาะมีการเปลี่ยนแปลง)
git add -A

commit_args=(-m "$MSG")
[[ $NO_VERIFY -eq 1 ]] && commit_args+=(--no-verify)
[[ $AMEND -eq 1 ]] && commit_args+=(--amend --no-edit)

if git diff --cached --quiet; then
  echo "No changes to commit (ไม่มีอะไรใหม่ใน staging)"
else
  git commit "${commit_args[@]}"
fi

# 8) สร้าง tag (ถ้าระบุ -t)
if [[ -n "$TAG" ]]; then
  if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
    echo "Tag $TAG มีอยู่แล้ว (ข้ามสร้าง)"
  else
    git tag -a "$TAG" -m "$MSG"
  fi
fi

# 9) push
push_args=()
[[ $FORCE -eq 1 ]] && push_args+=(--force-with-lease)
[[ $NO_VERIFY -eq 1 ]] && push_args+=(--no-verify)

# upstream ตั้งแล้วหรือยัง
if git rev-parse --abbrev-ref --symbolic-full-name @{u} >/dev/null 2>&1; then
  # มี upstream แล้ว: push ไปยัง branch เดิม
  git push "$REMOTE" HEAD:"$BRANCH" "${push_args[@]}"
else
  # ยังไม่มี upstream: ตั้ง -u ให้เลย
  git push -u "$REMOTE" HEAD:"$BRANCH" "${push_args[@]}"
fi

# 10) push tag ถ้าระบุ
if [[ -n "$TAG" ]]; then
  git push "$REMOTE" "refs/tags/$TAG" "${push_args[@]}"
fi

echo "✅ Done → $REMOTE/$BRANCH"
