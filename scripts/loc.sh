#!/usr/bin/env bash
# loc.sh — lines-of-code breakdown for the Casting repo, per language.
#
# Uses pygount when available (accurate code/comment split); otherwise falls
# back to a no-dependency line counter (total lines per language).
#
# Usage:  ./scripts/loc.sh [path]      (default path: repo root)
#         ./scripts/loc.sh frontend
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
TARGET="${1:-.}"

# Folders/dirs we never count (dependencies + build output).
SKIP=".git,node_modules,dist,build,target,.hermes,vendor,.casting,venv,.venv,__pycache__,.cache,.next,coverage,frontend/dist,frontend/node_modules"

# --- Locate pygount --------------------------------------------------------
PYGOUNT=""
for c in \
  "$(command -v pygount 2>/dev/null || true)" \
  "/tmp/pyg/bin/pygount" \
  "$HOME/.local/bin/pygount"; do
  if [ -n "$c" ] && [ -x "$c" ]; then PYGOUNT="$c"; break; fi
done

if [ -n "$PYGOUNT" ]; then
  echo "pygount at: $PYGOUNT"
  "$PYGOUNT" --format=summary --folders-to-skip="$SKIP" "$TARGET"
  exit 0
fi

echo "pygount not found — using basic line counter (total lines, no code/comment split)." >&2

# --- Fallback: count lines by extension ------------------------------------
python3 - "$TARGET" <<'PY'
import os, sys, collections
root = sys.argv[1]
SKIP = {'.git','node_modules','dist','build','target','.hermes','vendor','.casting',
        'venv','.venv','__pycache__','.cache','.next','coverage'}
# language by extension (order matters: pick first match)
LANGS = {
  '.rs': 'Rust', '.go': 'Go', '.py': 'Python', '.js': 'JavaScript',
  '.ts': 'TypeScript', '.tsx': 'TypeScript (React)', '.jsx': 'JavaScript (React)',
  '.css': 'CSS', '.scss':'SCSS', '.html': 'HTML', '.md': 'Markdown',
  '.json': 'JSON', '.yaml': 'YAML', '.yml': 'YAML', '.toml': 'TOML',
  '.sql': 'SQL', '.sh': 'Shell', '.dockerfile':'Dockerfile', 'Dockerfile':'Dockerfile',
  'Makefile':'Make', 'Cargo.toml':'TOML', 'package.json':'JSON',
}
counts = collections.defaultdict(lambda: [0,0])  # lang -> [files, lines]
for dirpath, dirnames, filenames in os.walk(root):
    dirnames[:] = [d for d in dirnames if d not in SKIP]
    for fn in filenames:
        if fn in ('package-lock.json',):  # generated lockfile — skip in fallback
            continue
        base, ext = fn.rsplit('.',1) if '.' in fn else (fn,'')
        lang = None
        low = fn.lower()
        if low in LANGS: lang = LANGS[low]
        elif '.'+ext.lower() in LANGS: lang = LANGS['.'+ext.lower()]
        elif low == 'makefile': lang = 'Make'
        else: continue
        try:
            with open(os.path.join(dirpath,fn),'r',encoding='utf-8',errors='ignore') as f:
                n = sum(1 for _ in f)
        except Exception:
            continue
        counts[lang][0]+=1; counts[lang][1]+=n

print(f"\n{'Language':<22}{'Files':>7}{'Lines':>10}")
print('-'*40)
total_f = total_l = 0
for lang,(f,l) in sorted(counts.items(), key=lambda kv:-kv[1][1]):
    print(f"{lang:<22}{f:>7}{l:>10,}")
    total_f+=f; total_l+=l
print('-'*40)
print(f"{'TOTAL':<22}{total_f:>7}{total_l:>10,}")
PY
