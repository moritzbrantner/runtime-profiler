#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

rm -rf dist
cp -R site dist

npm exec --yes \
  --package=git+https://github.com/moritzbrantner/github-pages-template.git#d24d6fb63120a1087a6c7178e08a951d0c2ca060 \
  -- github-pages-template build \
  --config ./pages.config.json \
  --out ./dist \
  --augment

test -f dist/index.html
test -f dist/results/index.html
test -f dist/score/index.html
test -f dist/stats/index.html
test -f dist/evidence/index.html
test -f dist/project-pages.json
test -f dist/assets/site-runtime.js
test -f dist/assets/evidence-source.js

grep -Fq 'Inspect a public profiler bundle.' dist/index.html
grep -Fq 'See what the profiler measured.' dist/results/index.html
grep -Fq 'Evidence revision' dist/assets/site-runtime.js
grep -Fq 'repository-identity-mismatch' dist/assets/evidence-source.js

node -e "const fs=require('node:fs'); const m=JSON.parse(fs.readFileSync('dist/project-pages.json','utf8')); if (m.mode !== 'augment' || m.generatedFrom !== 'moritzbrantner/runtime-profiler' || m.managedPaths.includes('index.html')) process.exit(1);"
