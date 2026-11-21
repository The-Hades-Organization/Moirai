#!/bin/sh
# Worktrees da fixture `hist`: um repo por unidade, com commits em datas fixas.
set -eu
mk() {
  dir="$HOME/wt/$1"
  mkdir -p "$dir"
  git -C "$dir" init -q -b "feat/$1"
}
commit() {
  dir="$HOME/wt/$1"
  date="$2"
  file="$3"
  lines="$4"
  seq 1 "$lines" > "$dir/$file"
  GIT_AUTHOR_DATE="$date" GIT_COMMITTER_DATE="$date" \
    GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t \
    sh -c "git -C '$dir' add . && git -C '$dir' commit -q -m '$file'"
}
mk u-um
commit u-um 2026-11-01T00:10:00Z a.txt 120
commit u-um 2026-11-01T00:20:00Z b.txt 30
commit u-um 2026-11-01T00:30:00Z a.txt 10
commit u-um 2026-11-05T00:30:00Z fora.txt 500
mk u-dois
commit u-dois 2026-11-02T01:00:00Z c.txt 700
mk u-skill
mk u-velha
