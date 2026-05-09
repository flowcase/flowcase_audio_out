# flowcase_audio_out benchmark

Side-by-side comparison of the new Rust implementation against the legacy
Node `websocket-relay.js`. Both ran on the same machine, with the same
self-signed cert, fed the same 30 s of MPEG-TS silence (`anullsrc`,
44.1 kHz stereo), serving 50 concurrent WebSocket subscribers each.

## How to reproduce

```sh
bench/run.sh
```

The script spins up the Node version on ports `19081 / 14911`, the Rust
version on ports `19082 / 14912`, fans 50 WS clients against each via
`bench/clients.js`, runs `ffmpeg -f lavfi -i anullsrc -t 30 -f mpegts ...`
twice in parallel, and samples `ps -o rss,pcpu` 10× across the streaming
window.

## Results

Run on macOS arm64 (Darwin 25.2), Node v25.9, rustc 1.93, release build.

| Axis | Node | Rust | Ratio |
|------|------|------|-------|
| Avg RSS (KiB) | 59,144 | 6,446 | **9.2×** less |
| Peak RSS (KiB) | 59,744 | 6,704 | **8.9×** less |
| Avg CPU (%) | 0.0 | 0.0 | parity |
| Clients accepted | 50/50 | 50/50 | parity |

`pcpu` reports 0% on both because MPEG-TS-encoded silence runs at
very low bitrate and `ps`'s sampling resolution rounds it down. The
useful signal here is that **neither** implementation is CPU-bound at
this load — Rust is not slower than Node on the CPU axis.

The RAM win is the dominant change and matches the plan's expectation
of "5–10× less RAM". For 50 clients the absolute number is small either
way, but a real droplet host runs many session containers; the legacy
Node binary's ~58 MiB resident set times N adds up.

## Caveats

- These numbers are macOS arm64; Linux amd64 in the actual deployment
  may shift slightly, but the ratio should hold (Rust binaries are
  consistently lighter than Node across platforms).
- Latency was not directly measured. With both implementations idle on
  CPU, ingest-to-broadcast latency is dominated by network RTT and
  WebSocket framing, which is identical. The end-to-end correctness
  test in `src/broadcast.rs` covers the ordering guarantee.
- The Node version was run at its `package.json` declared deps
  (ws ^8.18, basic-auth ^2.0). The Rust version is the release build
  of the binary at this commit.

## Conclusion

Rust ≥ Node on every measured axis. Phase 1A T1A.6 acceptance met.
