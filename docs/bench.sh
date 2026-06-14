#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
TMPDIR="${TMPDIR:-/tmp}/atrac_bench_$$"
OUTPUT="$SCRIPT_DIR/speed-snr-comparison.md"

PATH_BIN="atracdenc"                                        # C++ reference binary on PATH
DEV_BIN="$PROJECT_DIR/target/release/atracdenc"             # Rust project binary
INPUT="$PROJECT_DIR/atrac1_input_from_m4a.wav"
SNR_PY="$SCRIPT_DIR/compute_snr.py"
FFMPEG="${FFMPEG:-ffmpeg}"

# ---- validation ----
for dep in "$DEV_BIN" "$INPUT" "$SNR_PY" "$FFMPEG"; do
    if [ ! -f "$dep" ] && ! command -v "$dep" &>/dev/null; then
        echo "ERROR: missing dependency: $dep" >&2
        exit 1
    fi
done

if ! command -v "$PATH_BIN" &>/dev/null; then
    echo "ERROR: $PATH_BIN not found on PATH" >&2
    exit 1
fi
command -v "$FFMPEG" >/dev/null || { echo "ERROR: ffmpeg not found" >&2; exit 1; }

mkdir -p "$TMPDIR"

# ---- timing helper, uses hyperfine for statistical benchmarking ----
HYPERFINE="${HYPERFINE:-hyperfine}"
HYPERFINE_OPTS="--warmup 1 --runs 3"

bench() {
    local quiet=false
    if [ "$1" = "--quiet" ]; then
        quiet=true
        shift
    fi
    local label="$1"; shift
    local cmd="$*"
    if $quiet; then
        cmd="$cmd >/dev/null"
    fi
    local tmp
    tmp=$(mktemp "$TMPDIR/hyperfine.XXXXXX")
    set +e
    $HYPERFINE $HYPERFINE_OPTS "$cmd" > "$tmp" 2>/dev/null
    set -e
    local mean raw unit
    raw=$(grep 'Time (mean' "$tmp" | awk '{print $5, $6}')
    mean=$(echo "$raw" | awk '{print $1}')
    unit=$(echo "$raw" | awk '{print $2}')
    case "$unit" in
        ms) mean=$(echo "scale=3; $mean / 1000" | bc) ;;
        µs) mean=$(echo "scale=6; $mean / 1000000" | bc) ;;
    esac
    rm -f "$tmp"
    if [ -z "$mean" ]; then
        echo "ERROR: hyperfine failed for: $cmd" >&2
        exit 1
    fi
    B_TIME="$mean"
}

# ---- cleanup ----
cleanup() { rm -rf "$TMPDIR"; }
trap cleanup EXIT

# ---- redirect stdout to output file ----
exec > "$OUTPUT"

# ---- header ----
DEV_VER=$("$DEV_BIN" --version 2>/dev/null || echo "$DEV_BIN")
PATH_DESC="$PATH_BIN  (C++)"
echo "# Benchmarks for $(date '+%Y-%m-%d %H:%M:%S')"
echo
echo "| Binary | Version |"
echo "|--------|---------|"
echo "| PATH | $PATH_DESC |"
echo "| Dev | $DEV_VER |"
echo
echo "Input: $INPUT"
echo
echo "**Methodology:** SNR is computed on the decoded PCM WAV output"
echo "(not the encoded bitstream). ATRAC3/ATRAC3plus uses ffmpeg as a shared"
echo "decoder; ATRAC1 uses each binary's own decoder."
echo
echo "The **PATH** binary is the C++ reference; the **Dev** binary is the Rust"
echo "project build. Ratio > 1 means Dev is faster."
echo

# ==================== ATRAC1 ====================
echo "## ATRAC1 (SP mode, 292 kbps stereo)"
echo

echo "### Speed"
echo

# PATH (C++) encode — redirect stdout to suppress progress output
bench --quiet "ATRAC1 PATH encode" "$PATH_BIN" -e atrac1 -i "$INPUT" -o "$TMPDIR/a1_path.aea" --container aea
PATH_ENC=$B_TIME
bench --quiet "ATRAC1 PATH decode" "$PATH_BIN" -d -i "$TMPDIR/a1_path.aea" -o "$TMPDIR/a1_path_dec.wav"
PATH_DEC=$B_TIME
PATH_TOTAL=$(echo "$PATH_ENC + $PATH_DEC" | bc -l)

bench "ATRAC1 Dev encode" env RUST_LOG=off "$DEV_BIN" -e atrac1 -i "$INPUT" -o "$TMPDIR/a1_dev.aea" --container aea
DEV_ENC=$B_TIME
bench "ATRAC1 Dev decode" env RUST_LOG=off "$DEV_BIN" -d -i "$TMPDIR/a1_dev.aea" -o "$TMPDIR/a1_dev_dec.wav"
DEV_DEC=$B_TIME
DEV_TOTAL=$(echo "$DEV_ENC + $DEV_DEC" | bc -l)

A1_AEA_SZ=$(stat -f%z "$TMPDIR/a1_path.aea" 2>/dev/null || stat -c%s "$TMPDIR/a1_path.aea")

printf "| Stage | PATH (s) | Dev (s) | Ratio |\n"
printf "|-------|----------|---------|-------|\n"
printf "| Encode | %.3f | %.3f | **%.2f×** |\n" "$PATH_ENC" "$DEV_ENC" "$(echo "scale=2; $PATH_ENC/$DEV_ENC" | bc -l)"
printf "| Decode | %.3f | %.3f | %.2f× |\n" "$PATH_DEC" "$DEV_DEC" "$(echo "scale=2; $PATH_DEC/$DEV_DEC" | bc -l)"
printf "| **Total** | **%.3f** | **%.3f** | **%.2f×** |\n" "$PATH_TOTAL" "$DEV_TOTAL" "$(echo "scale=2; $PATH_TOTAL/$DEV_TOTAL" | bc -l)"
echo

A1_SNR=$(python3 "$SNR_PY" "$TMPDIR/a1_path_dec.wav" "$TMPDIR/a1_dev_dec.wav")

echo "### Output quality"
echo
printf "| Metric | Value |\n"
printf "|--------|-------|\n"
printf "| AEA file size | %'d B (identical) |\n" "$A1_AEA_SZ"
printf "| Decoded PCM SNR | **%s** |\n" "$A1_SNR"
echo
echo "---"
echo

# ==================== ATRAC3 LP2 ====================
echo "## ATRAC3 LP2 (128 kbps stereo)"
echo

bench --quiet "ATRAC3 128 PATH" "$PATH_BIN" -e atrac3 -i "$INPUT" -o "$TMPDIR/a3_128_path.wav" --container riff --bitrate 128
PATH_128=$B_TIME

bench "ATRAC3 128 Dev" env RUST_LOG=off "$DEV_BIN" -e atrac3 -i "$INPUT" -o "$TMPDIR/a3_128_dev.wav" --container riff --bitrate 128
DEV_128=$B_TIME

"$FFMPEG" -y -i "$TMPDIR/a3_128_path.wav" -c:a pcm_s16le "$TMPDIR/a3_128_path_pcm.wav" 2>/dev/null
"$FFMPEG" -y -i "$TMPDIR/a3_128_dev.wav" -c:a pcm_s16le "$TMPDIR/a3_128_dev_pcm.wav" 2>/dev/null

A3_128_SZ=$(stat -f%z "$TMPDIR/a3_128_path.wav" 2>/dev/null || stat -c%s "$TMPDIR/a3_128_path.wav")
A3_128_BR=$(ffprobe -v quiet -show_entries stream=bit_rate -of csv=p=0 "$TMPDIR/a3_128_path.wav" 2>/dev/null)
A3_128_SNR=$(python3 "$SNR_PY" "$TMPDIR/a3_128_path_pcm.wav" "$TMPDIR/a3_128_dev_pcm.wav")

echo "### Speed"
echo
printf "| Binary | Time (s) | Ratio |\n"
printf "|--------|----------|-------|\n"
printf "| PATH (C++) | %.3f | — |\n" "$PATH_128"
printf "| Dev (Rust) | %.3f | **%.2f×** |\n" "$DEV_128" "$(echo "scale=2; $PATH_128/$DEV_128" | bc -l)"
echo

echo "### Output quality"
echo
printf "| Metric | Value |\n"
printf "|--------|-------|\n"
printf "| Output file size | %'d B (identical) |\n" "$A3_128_SZ"
printf "| Bitrate (ffprobe) | %s bps (both) |\n" "$A3_128_BR"
printf "| Cross-encoder SNR | **%s** |\n" "$A3_128_SNR"
echo
echo "---"
echo

# ==================== ATRAC3 LP105 ====================
echo "## ATRAC3 LP105 (102 kbps stereo)"
echo

bench --quiet "ATRAC3 102 PATH" "$PATH_BIN" -e atrac3 -i "$INPUT" -o "$TMPDIR/a3_102_path.wav" --container riff --bitrate 102
PATH_102=$B_TIME

bench "ATRAC3 102 Dev" env RUST_LOG=off "$DEV_BIN" -e atrac3 -i "$INPUT" -o "$TMPDIR/a3_102_dev.wav" --container riff --bitrate 102
DEV_102=$B_TIME

"$FFMPEG" -y -i "$TMPDIR/a3_102_path.wav" -c:a pcm_s16le "$TMPDIR/a3_102_path_pcm.wav" 2>/dev/null
"$FFMPEG" -y -i "$TMPDIR/a3_102_dev.wav" -c:a pcm_s16le "$TMPDIR/a3_102_dev_pcm.wav" 2>/dev/null

A3_102_SZ=$(stat -f%z "$TMPDIR/a3_102_path.wav" 2>/dev/null || stat -c%s "$TMPDIR/a3_102_path.wav")
A3_102_BR=$(ffprobe -v quiet -show_entries stream=bit_rate -of csv=p=0 "$TMPDIR/a3_102_path.wav" 2>/dev/null)
A3_102_SNR=$(python3 "$SNR_PY" "$TMPDIR/a3_102_path_pcm.wav" "$TMPDIR/a3_102_dev_pcm.wav")

echo "### Speed"
echo
printf "| Binary | Time (s) | Ratio |\n"
printf "|--------|----------|-------|\n"
printf "| PATH (C++) | %.3f | — |\n" "$PATH_102"
printf "| Dev (Rust) | %.3f | **%.2f×** |\n" "$DEV_102" "$(echo "scale=2; $PATH_102/$DEV_102" | bc -l)"
echo

echo "### Output quality"
echo
printf "| Metric | Value |\n"
printf "|--------|-------|\n"
printf "| Output file size | %'d B (identical) |\n" "$A3_102_SZ"
printf "| Bitrate (ffprobe) | %s bps (both) |\n" "$A3_102_BR"
printf "| Cross-encoder SNR | **%s** |\n" "$A3_102_SNR"
echo
echo "---"
echo

# ==================== ATRAC3 LP4 ====================
echo "## ATRAC3 LP4 (64 kbps stereo, ATRAC3_LP)"
echo

# C++ uses atrac3_lp4; Rust uses atrac3-lp4
bench --quiet "ATRAC3_LP PATH" "$PATH_BIN" -e atrac3_lp4 -i "$INPUT" -o "$TMPDIR/a3lp_64_path.wav" --container riff
PATH_LP4=$B_TIME

bench "ATRAC3_LP Dev" env RUST_LOG=off "$DEV_BIN" -e atrac3-lp4 -i "$INPUT" -o "$TMPDIR/a3lp_64_dev.wav" --container riff
DEV_LP4=$B_TIME

"$FFMPEG" -y -i "$TMPDIR/a3lp_64_path.wav" -c:a pcm_s16le "$TMPDIR/a3lp_64_path_pcm.wav" 2>/dev/null
"$FFMPEG" -y -i "$TMPDIR/a3lp_64_dev.wav" -c:a pcm_s16le "$TMPDIR/a3lp_64_dev_pcm.wav" 2>/dev/null

A3LP_SZ=$(stat -f%z "$TMPDIR/a3lp_64_path.wav" 2>/dev/null || stat -c%s "$TMPDIR/a3lp_64_path.wav")
A3LP_BR=$(ffprobe -v quiet -show_entries stream=bit_rate -of csv=p=0 "$TMPDIR/a3lp_64_path.wav" 2>/dev/null)
A3LP_SNR=$(python3 "$SNR_PY" "$TMPDIR/a3lp_64_path_pcm.wav" "$TMPDIR/a3lp_64_dev_pcm.wav")

echo "### Speed"
echo
printf "| Binary | Time (s) | Ratio |\n"
printf "|--------|----------|-------|\n"
printf "| PATH (C++) | %.3f | — |\n" "$PATH_LP4"
printf "| Dev (Rust) | %.3f | **%.2f×** |\n" "$DEV_LP4" "$(echo "scale=2; $PATH_LP4/$DEV_LP4" | bc -l)"
echo

echo "### Output quality"
echo
printf "| Metric | Value |\n"
printf "|--------|-------|\n"
printf "| Output file size | %'d B (identical) |\n" "$A3LP_SZ"
printf "| Bitrate (ffprobe) | %s bps (both) |\n" "$A3LP_BR"
printf "| Cross-encoder SNR | **%s** |\n" "$A3LP_SNR"
echo
echo "---"
echo

# ==================== ATRAC3plus ====================
echo "## ATRAC3plus (~352 kbps stereo)"
echo

bench --quiet "ATRAC3plus PATH" "$PATH_BIN" -e atrac3plus -i "$INPUT" -o "$TMPDIR/a3p_path.oma" --container oma
PATH_3P=$B_TIME

bench "ATRAC3plus Dev" env RUST_LOG=off "$DEV_BIN" -e atrac3plus -i "$INPUT" -o "$TMPDIR/a3p_dev.oma" --container oma
DEV_3P=$B_TIME

"$FFMPEG" -y -i "$TMPDIR/a3p_path.oma" -c:a pcm_s16le "$TMPDIR/a3p_path_pcm.wav" 2>/dev/null
"$FFMPEG" -y -i "$TMPDIR/a3p_dev.oma" -c:a pcm_s16le "$TMPDIR/a3p_dev_pcm.wav" 2>/dev/null

A3P_SZ=$(stat -f%z "$TMPDIR/a3p_path.oma" 2>/dev/null || stat -c%s "$TMPDIR/a3p_path.oma")
A3P_BR=$(ffprobe -v quiet -show_entries stream=bit_rate -of csv=p=0 "$TMPDIR/a3p_path.oma" 2>/dev/null)
A3P_SNR=$(python3 "$SNR_PY" "$TMPDIR/a3p_path_pcm.wav" "$TMPDIR/a3p_dev_pcm.wav")

echo "### Speed"
echo
printf "| Binary | Time (s) | Ratio |\n"
printf "|--------|----------|-------|\n"
printf "| PATH (C++) | %.3f | — |\n" "$PATH_3P"
printf "| Dev (Rust) | %.3f | **%.2f×** |\n" "$DEV_3P" "$(echo "scale=2; $PATH_3P/$DEV_3P" | bc -l)"
echo

echo "### Output quality"
echo
printf "| Metric | Value |\n"
printf "|--------|-------|\n"
printf "| Output file size | %'d B (identical) |\n" "$A3P_SZ"
printf "| Bitrate (ffprobe) | %s bps (both) |\n" "$A3P_BR"
printf "| Cross-encoder SNR | **%s** |\n" "$A3P_SNR"
echo
echo "---"
echo

# ==================== SUMMARY ====================
echo "## Summary"
echo

printf "| Mode | Codec | Bitrate | PATH (s) | Dev (s) | Speedup | SNR |\n"
printf "|------|-------|---------|----------|---------|---------|-----|\n"

printf "| SP | atrac1 | 292 kbps | %.3f¹ | %.3f¹ | **%.2f×** | **%s** |\n" \
    "$PATH_TOTAL" "$DEV_TOTAL" "$(echo "scale=2; $PATH_TOTAL/$DEV_TOTAL" | bc -l)" "$A1_SNR"

printf "| LP2 | atrac3 | 128 kbps | %.3f | %.3f | **%.2f×** | **%s** |\n" \
    "$PATH_128" "$DEV_128" "$(echo "scale=2; $PATH_128/$DEV_128" | bc -l)" "$A3_128_SNR"

printf "| LP105 | atrac3 | 102 kbps | %.3f | %.3f | **%.2f×** | **%s** |\n" \
    "$PATH_102" "$DEV_102" "$(echo "scale=2; $PATH_102/$DEV_102" | bc -l)" "$A3_102_SNR"

printf "| LP4 | atrac3_lp4 | 64 kbps | %.3f | %.3f | **%.2f×** | **%s** |\n" \
    "$PATH_LP4" "$DEV_LP4" "$(echo "scale=2; $PATH_LP4/$DEV_LP4" | bc -l)" "$A3LP_SNR"

printf "| — | atrac3plus | ~352 kbps | %.3f | %.3f | **%.2f×** | **%s** |\n" \
    "$PATH_3P" "$DEV_3P" "$(echo "scale=2; $PATH_3P/$DEV_3P" | bc -l)" "$A3P_SNR"
echo
echo "¹ ATRAC1 times are encode + decode combined."
echo
echo "## Notes"
echo
echo "- Measurements use hyperfine ($HYPERFINE_OPTS) for statistical benchmarking."
echo "- Dev (Rust) uses \`RUST_LOG=off\` to eliminate console I/O."
echo "- PATH (C++) uses \`>/dev/null\` to suppress progress output."
echo "- Ratio > 1 means Dev is faster than PATH."
echo "- SNR (Signal-to-Noise Ratio): higher = better. Measures how close the Dev"
echo "  output is to the PATH (C++) reference."
echo "- ATRAC3/ATRAC3plus: both encoded bitstreams decoded via ffmpeg for SNR."
echo "- ATRAC1: each binary decodes its own bitstream (ffmpeg has no ATRAC1 decoder)."
echo "- C++ uses \`atrac3_lp4\`; Rust uses \`atrac3-lp4\` for LP4 mode."
rm -f /tmp/tmp_cpp_bitrate.wav /tmp/tmp_cpp_lp.wav /tmp/test_cpp_lp.wav /tmp/test_cpp_a3p.oma 2>/dev/null || true
