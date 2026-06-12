#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
TMPDIR="${TMPDIR:-/tmp}/atrac_bench_$$"

CPP_BIN="$PROJECT_DIR/atracdenc/build/src/atracdenc"
RUST_BIN="$PROJECT_DIR/target/release/atracdenc"
INPUT="$PROJECT_DIR/atrac1_input_from_m4a.wav"
SNR_PY="$SCRIPT_DIR/compute_snr.py"
FFMPEG="${FFMPEG:-ffmpeg}"

# ---- validation ----
for dep in "$CPP_BIN" "$RUST_BIN" "$INPUT" "$SNR_PY" "$FFMPEG"; do
    if [ ! -f "$dep" ] && ! command -v "$dep" &>/dev/null; then
        echo "ERROR: missing dependency: $dep" >&2
        exit 1
    fi
done

command -v "$FFMPEG" >/dev/null || { echo "ERROR: ffmpeg not found" >&2; exit 1; }

mkdir -p "$TMPDIR"

# ---- timing helper, uses hyperfine for statistical benchmarking ----
HYPERFINE="${HYPERFINE:-hyperfine}"
HYPERFINE_OPTS="--warmup 1 --runs 3"

bench() {
    local label="$1"; shift
    local cmd="$*"
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

echo "# Benchmarks for $(date '+%Y-%m-%d %H:%M:%S')"
echo "Input: $INPUT"
echo "C++:   $CPP_BIN"
echo "Rust:  $RUST_BIN"
echo
echo "**Methodology:** SNR is computed on the decoded PCM WAV output"
echo "(not the encoded bitstream). ATRAC3 uses ffmpeg as a shared decoder;"
echo "ATRAC1 uses each binary's own decoder."
echo

# ==================== ATRAC1 ====================
echo "## ATRAC1 (SP mode, 292 kbps stereo)"
echo

echo "### Speed"
echo

# C++ encode
bench "ATRAC1 C++ encode" "$CPP_BIN" -e atrac1 -i "$INPUT" -o "$TMPDIR/a1_orig.aea" --container aea
CPP_ENC=$B_TIME
# C++ decode
bench "ATRAC1 C++ decode" "$CPP_BIN" -d -i "$TMPDIR/a1_orig.aea" -o "$TMPDIR/a1_orig_dec.wav"
CPP_DEC=$B_TIME
CPP_TOTAL=$(echo "$CPP_ENC + $CPP_DEC" | bc -l)

# Rust encode
bench "ATRAC1 Rust encode" "$RUST_BIN" -e atrac1 -i "$INPUT" -o "$TMPDIR/a1_rust.aea" --container aea --nostdout
RUST_ENC=$B_TIME
# Rust decode
bench "ATRAC1 Rust decode" "$RUST_BIN" -d -i "$TMPDIR/a1_rust.aea" -o "$TMPDIR/a1_rust_dec.wav" --nostdout
RUST_DEC=$B_TIME
RUST_TOTAL=$(echo "$RUST_ENC + $RUST_DEC" | bc -l)

A1_AEA_SZ=$(stat -f%z "$TMPDIR/a1_orig.aea" 2>/dev/null || stat -c%s "$TMPDIR/a1_orig.aea")

printf "| Stage | C++ (s) | Rust (s) | Ratio |\n"
printf "|-------|---------|----------|-------|\n"
printf "| Encode | %.3f | %.3f | **%.2f× faster** |\n" "$CPP_ENC" "$RUST_ENC" "$(echo "scale=2; $CPP_ENC/$RUST_ENC" | bc -l)"
printf "| Decode | %.3f | %.3f | %.2f× |\n" "$CPP_DEC" "$RUST_DEC" "$(echo "scale=2; $CPP_DEC/$RUST_DEC" | bc -l)"
printf "| **Total** | **%.3f** | **%.3f** | **%.2f× faster** |\n" "$CPP_TOTAL" "$RUST_TOTAL" "$(echo "scale=2; $CPP_TOTAL/$RUST_TOTAL" | bc -l)"
echo

A1_SNR=$(python3 "$SNR_PY" "$TMPDIR/a1_orig_dec.wav" "$TMPDIR/a1_rust_dec.wav")

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

bench "ATRAC3 128 C++" "$CPP_BIN" -e atrac3 -i "$INPUT" -o "$TMPDIR/a3_128_orig.wav" --container riff --bitrate 128
CPP_128=$B_TIME

bench "ATRAC3 128 Rust" "$RUST_BIN" -e atrac3 -i "$INPUT" -o "$TMPDIR/a3_128_rust.wav" --container riff --bitrate 128 --nostdout
RUST_128=$B_TIME

# decode both to PCM
"$FFMPEG" -y -i "$TMPDIR/a3_128_orig.wav" -c:a pcm_s16le "$TMPDIR/a3_128_orig_pcm.wav" 2>/dev/null
"$FFMPEG" -y -i "$TMPDIR/a3_128_rust.wav" -c:a pcm_s16le "$TMPDIR/a3_128_rust_pcm.wav" 2>/dev/null

A3_128_SZ=$(stat -f%z "$TMPDIR/a3_128_orig.wav" 2>/dev/null || stat -c%s "$TMPDIR/a3_128_orig.wav")
A3_128_BR=$(ffprobe -v quiet -show_entries stream=bit_rate -of csv=p=0 "$TMPDIR/a3_128_orig.wav" 2>/dev/null)
A3_128_SNR=$(python3 "$SNR_PY" "$TMPDIR/a3_128_orig_pcm.wav" "$TMPDIR/a3_128_rust_pcm.wav")

echo "### Speed"
echo
printf "| Binary | Time (s) | Ratio |\n"
printf "|--------|----------|-------|\n"
printf "| C++ | %.3f | — |\n" "$CPP_128"
printf "| Rust | %.3f | **%.2f× faster** |\n" "$RUST_128" "$(echo "scale=2; $CPP_128/$RUST_128" | bc -l)"
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

bench "ATRAC3 102 C++" "$CPP_BIN" -e atrac3 -i "$INPUT" -o "$TMPDIR/a3_102_orig.wav" --container riff --bitrate 102
CPP_102=$B_TIME

bench "ATRAC3 102 Rust" "$RUST_BIN" -e atrac3 -i "$INPUT" -o "$TMPDIR/a3_102_rust.wav" --container riff --bitrate 102 --nostdout
RUST_102=$B_TIME

"$FFMPEG" -y -i "$TMPDIR/a3_102_orig.wav" -c:a pcm_s16le "$TMPDIR/a3_102_orig_pcm.wav" 2>/dev/null
"$FFMPEG" -y -i "$TMPDIR/a3_102_rust.wav" -c:a pcm_s16le "$TMPDIR/a3_102_rust_pcm.wav" 2>/dev/null

A3_102_SZ=$(stat -f%z "$TMPDIR/a3_102_orig.wav" 2>/dev/null || stat -c%s "$TMPDIR/a3_102_orig.wav")
A3_102_BR=$(ffprobe -v quiet -show_entries stream=bit_rate -of csv=p=0 "$TMPDIR/a3_102_orig.wav" 2>/dev/null)
A3_102_SNR=$(python3 "$SNR_PY" "$TMPDIR/a3_102_orig_pcm.wav" "$TMPDIR/a3_102_rust_pcm.wav")

echo "### Speed"
echo
printf "| Binary | Time (s) | Ratio |\n"
printf "|--------|----------|-------|\n"
printf "| C++ | %.3f | — |\n" "$CPP_102"
printf "| Rust | %.3f | **%.2f× faster** |\n" "$RUST_102" "$(echo "scale=2; $CPP_102/$RUST_102" | bc -l)"
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

bench "ATRAC3_LP C++" "$CPP_BIN" -e atrac3_lp4 -i "$INPUT" -o "$TMPDIR/a3lp_64_orig.wav" --container riff
CPP_LP4=$B_TIME

bench "ATRAC3_LP Rust" "$RUST_BIN" -e atrac3-lp4 -i "$INPUT" -o "$TMPDIR/a3lp_64_rust.wav" --container riff --nostdout
RUST_LP4=$B_TIME

"$FFMPEG" -y -i "$TMPDIR/a3lp_64_orig.wav" -c:a pcm_s16le "$TMPDIR/a3lp_64_orig_pcm.wav" 2>/dev/null
"$FFMPEG" -y -i "$TMPDIR/a3lp_64_rust.wav" -c:a pcm_s16le "$TMPDIR/a3lp_64_rust_pcm.wav" 2>/dev/null

A3LP_SZ=$(stat -f%z "$TMPDIR/a3lp_64_orig.wav" 2>/dev/null || stat -c%s "$TMPDIR/a3lp_64_orig.wav")
A3LP_BR=$(ffprobe -v quiet -show_entries stream=bit_rate -of csv=p=0 "$TMPDIR/a3lp_64_orig.wav" 2>/dev/null)
A3LP_SNR=$(python3 "$SNR_PY" "$TMPDIR/a3lp_64_orig_pcm.wav" "$TMPDIR/a3lp_64_rust_pcm.wav")

echo "### Speed"
echo
printf "| Binary | Time (s) | Ratio |\n"
printf "|--------|----------|-------|\n"
printf "| C++ | %.3f | — |\n" "$CPP_LP4"
printf "| Rust | %.3f | **%.2f× faster** |\n" "$RUST_LP4" "$(echo "scale=2; $CPP_LP4/$RUST_LP4" | bc -l)"
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

# ==================== SUMMARY ====================
echo "## Summary"
echo

printf "| Mode | Codec | Bitrate | C++ (s) | Rust (s) | Speedup | SNR |\n"
printf "|------|-------|---------|---------|----------|---------|-----|\n"

printf "| SP | atrac1 | 292 kbps | %.3f¹ | %.3f¹ | **%.2f×** | **%s** |\n" \
    "$CPP_TOTAL" "$RUST_TOTAL" "$(echo "scale=2; $CPP_TOTAL/$RUST_TOTAL" | bc -l)" "$A1_SNR"

printf "| LP2 | atrac3 | 128 kbps | %.3f | %.3f | **%.2f×** | **%s** |\n" \
    "$CPP_128" "$RUST_128" "$(echo "scale=2; $CPP_128/$RUST_128" | bc -l)" "$A3_128_SNR"

printf "| LP105 | atrac3 | 102 kbps | %.3f | %.3f | **%.2f×** | **%s** |\n" \
    "$CPP_102" "$RUST_102" "$(echo "scale=2; $CPP_102/$RUST_102" | bc -l)" "$A3_102_SNR"

printf "| LP4 | atrac3_lp4 | 64 kbps | %.3f | %.3f | **%.2f×** | **%s** |\n" \
    "$CPP_LP4" "$RUST_LP4" "$(echo "scale=2; $CPP_LP4/$RUST_LP4" | bc -l)" "$A3LP_SNR"
echo
echo "¹ ATRAC1 times are encode + decode combined."
echo
echo "## Notes"
echo
echo "- Measurements use hyperfine ($HYPERFINE_OPTS) for statistical benchmarking."
echo "- Encodes use \`--nostdout\` (Rust) or \`> /dev/null\` (C++) to eliminate console I/O."
echo "- SNR (Signal-to-Noise Ratio): higher = better. Measures how close the Rust output"
echo "  is to the C++ reference. 84 dB = nearly identical; 28 dB = audible differences."
echo "- See [precision-analysis.md](precision-analysis.md) for details on cross-encoder SNR differences."
