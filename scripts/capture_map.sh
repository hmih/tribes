#!/usr/bin/env bash
# Reproducible diagnostic captures of a loaded map.
#
# Runs the client once per named camera preset and writes `<out>/<preset>.png`.
# `TASCEND_CAM` is given as `x,y,z,tx,ty,tz` (position + look-at) so a shot can
# be retaken byte-for-reason after any asset change; see crates/client/src/shot.rs.
#
# Debug overlays (gameplay marker gizmos) are suppressed automatically whenever
# TASCEND_SHOT is set, so nothing debug-only ends up in a capture.
#
# usage: capture_map.sh [preset ...]
# env:   OUT=/tmp/shots  DELAY=25  MAP=<dir name>
#
# Coordinates are glTF/world space, i.e. (x_ue, z_ue, y_ue). The two bases sit at
# UE3 (-8448,-15555,5116) and (8449,15555,5116), which is where the presets below
# are aimed.
set -euo pipefail

cd "$(dirname "$0")/.."   # src/port

OUT="${OUT:-/tmp/shots}"
DELAY="${DELAY:-25}"
MAP="${MAP:-ArxNovena}"

# Rust dylibs are not on the default search path in this environment.
export DYLD_LIBRARY_PATH="${DYLD_LIBRARY_PATH:-/nix/store/ah2c1xy8qqg2209chn65yi8fd62zk3hi-rustc-1.95.0/lib/rustlib/aarch64-apple-darwin/lib}"
export TASCEND_MAP="$MAP"

BE_X=-8448; BE_Y=5116;  BE_Z=-15555
DS_X=8449;  DS_Y=5116;  DS_Z=15555

# name|camera
PRESETS=(
  "overhead|0,62000,0,0,0,0"
  "be_wide|-26000,16000,-36000,$BE_X,$BE_Y,$BE_Z"
  "be_close|-13500,8600,-21500,$BE_X,$BE_Y,$BE_Z"
  "be_gate|-8448,6200,-21000,$BE_X,$BE_Y,$BE_Z"
  "center|0,14000,0,$DS_X,$DS_Y,$DS_Z"
  "ds_wide|26000,16000,36000,$DS_X,$DS_Y,$DS_Z"
  "ds_close|13500,8600,21500,$DS_X,$DS_Y,$DS_Z"
  "aqueduct|0,9000,-20000,0,3000,20000"
  "under|0,5000,-20000,0,7000,5000"
)

want=("$@")
if [ ${#want[@]} -eq 0 ]; then
  want=()
  for p in "${PRESETS[@]}"; do want+=("${p%%|*}"); done
fi

mkdir -p "$OUT"
for entry in "${PRESETS[@]}"; do
  name="${entry%%|*}"
  cam="${entry#*|}"
  keep=0
  for w in "${want[@]}"; do [ "$w" = "$name" ] && keep=1; done
  [ "$keep" = 1 ] || continue

  echo "== $name  cam=$cam"
  rm -f "$OUT/$name.png"
  TASCEND_SHOT="$OUT/$name.png" \
  TASCEND_SHOT_DELAY="$DELAY" \
  TASCEND_CAM="$cam" \
  RUST_LOG=info \
    ./target/debug/client > "$OUT/$name.log" 2>&1 || true
  if [ -s "$OUT/$name.png" ]; then
    echo "   wrote $OUT/$name.png"
  else
    echo "   FAILED — see $OUT/$name.log"
  fi
done
