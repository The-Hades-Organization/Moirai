#!/bin/sh
# Plano da fixture `hist` e, só no lado Rust, um `hades` falso que aponta o `claude` dos stubs (mesmo estimador do lado TS).
set -eu
sh "$(dirname "$0")/setup-plano.sh"
if [ "${SIDE:-}" = rust ]; then
  mkdir -p "$HOME/bin"
  cat > "$HOME/bin/hades" <<'SH'
#!/bin/sh
printf '%s\n' '{"bin":"claude","args":["-p","estima"],"cwd":"/","timeout_s":30}'
SH
  chmod +x "$HOME/bin/hades"
fi
