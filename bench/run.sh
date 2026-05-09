#!/usr/bin/env bash
# Benchmark Rust flowcase_audio_out vs the legacy Node websocket-relay.
# Both run with the same self-signed cert, ingest 30 s of MPEG-TS silence,
# and serve 50 WS clients. We sample RSS + CPU% across the stream window
# and report peaks.
#
# Usage: bench/run.sh
# Outputs a JSON-ish block to stdout; bench/run.sh > bench/results.txt.

set -euo pipefail
cd "$(dirname "$0")/.."

CERT=${CERT:-/tmp/audio-out-smoke/cert.pem}
KEY=${KEY:-/tmp/audio-out-smoke/key.pem}
SECRET=bench
SAMPLES=${SAMPLES:-10}
DURATION=${DURATION:-30}
CLIENTS=${CLIENTS:-50}

if [[ ! -f "$CERT" || ! -f "$KEY" ]]; then
    mkdir -p "$(dirname "$CERT")"
    openssl req -x509 -newkey rsa:2048 -nodes -keyout "$KEY" -out "$CERT" \
        -days 1 -subj '/CN=localhost' >/dev/null 2>&1
fi

# Build the Rust binary in release mode.
cargo build --release --quiet

# Boot the Node version on ports 19081 / 14911.
NODE_LOG=$(mktemp)
node legacy-node/websocket-relay.js "$SECRET" 19081 14911 "$CERT" "$KEY" \
    > "$NODE_LOG" 2>&1 &
NODE_PID=$!

# Boot the Rust version on ports 19082 / 14912.
RUST_LOG=$(mktemp)
target/release/flowcase_audio_out "$SECRET" 19082 14912 "$CERT" "$KEY" \
    > "$RUST_LOG" 2>&1 &
RUST_PID=$!

cleanup() {
    kill "$NODE_PID" "$RUST_PID" "$NODE_CLIENTS_PID" "$RUST_CLIENTS_PID" 2>/dev/null || true
    wait 2>/dev/null || true
}
trap cleanup EXIT

sleep 1
if ! kill -0 "$NODE_PID" 2>/dev/null; then
    echo "node server failed to start:"; cat "$NODE_LOG"; exit 1
fi
if ! kill -0 "$RUST_PID" 2>/dev/null; then
    echo "rust server failed to start:"; cat "$RUST_LOG"; exit 1
fi

# Spawn 50 WS subscribers against each.
node bench/clients.js "wss://localhost:14911/" "$CLIENTS" > /dev/null 2>&1 &
NODE_CLIENTS_PID=$!
node bench/clients.js "wss://localhost:14912/" "$CLIENTS" > /dev/null 2>&1 &
RUST_CLIENTS_PID=$!

# Let them connect / subscribe.
sleep 2

# Pipe 30 s of MPEG-TS silence to each ingest port. We run two ffmpeg
# processes side-by-side so each server sees its own identical stream.
ffmpeg -hide_banner -loglevel error -f lavfi -i "anullsrc=r=44100:cl=stereo" \
    -t "$DURATION" -f mpegts "http://127.0.0.1:19081/$SECRET" &
FF_NODE=$!
ffmpeg -hide_banner -loglevel error -f lavfi -i "anullsrc=r=44100:cl=stereo" \
    -t "$DURATION" -f mpegts "http://127.0.0.1:19082/$SECRET" &
FF_RUST=$!

# Sample RSS (KiB) and CPU% every 1.5 s while ffmpeg is streaming.
SAMPLE_NODE=$(mktemp)
SAMPLE_RUST=$(mktemp)
for ((i=0;i<SAMPLES;i++)); do
    ps -p "$NODE_PID" -o rss=,pcpu= >> "$SAMPLE_NODE" || true
    ps -p "$RUST_PID" -o rss=,pcpu= >> "$SAMPLE_RUST" || true
    sleep 1.5
done

wait "$FF_NODE" "$FF_RUST" 2>/dev/null || true

summarize() {
    awk '
        { rss[NR]=$1; cpu[NR]=$2; if($1>peak_rss) peak_rss=$1; sum_rss+=$1; sum_cpu+=$2 }
        END {
            n=NR;
            if (n==0) { print "no samples"; exit 1 }
            printf "samples=%d  rss_avg_kib=%.0f  rss_peak_kib=%.0f  cpu_avg_pct=%.1f\n",
                n, sum_rss/n, peak_rss, sum_cpu/n
        }
    ' "$1"
}

echo
echo "==== node ===="
summarize "$SAMPLE_NODE"
echo
echo "==== rust ===="
summarize "$SAMPLE_RUST"
echo

rm -f "$SAMPLE_NODE" "$SAMPLE_RUST" "$NODE_LOG" "$RUST_LOG"
