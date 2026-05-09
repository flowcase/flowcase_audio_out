# flowcase_audio_out

Tiny TLS-enabled WebSocket relay that fans an MPEG-TS stream out to browser
clients. Used inside Flowcase droplet images to ship desktop audio to the
session viewer.

## Build

```sh
cargo build --release
```

The binary lands at `target/release/flowcase_audio_out`.

## CLI

```
flowcase_audio_out <SECRET> <STREAM_PORT> <WS_PORT> <SSL_CERT> <SSL_KEY> [AUTH_TOKEN]
```

| Arg | Meaning |
|-----|---------|
| `SECRET` | URL-path secret. Only `POST /<SECRET>` on the ingest port is accepted. |
| `STREAM_PORT` | Plain HTTP port that ffmpeg POSTs the MPEG-TS stream to. |
| `WS_PORT` | TLS-only WebSocket port that the browser connects to (`wss://`). |
| `SSL_CERT` | PEM-encoded certificate. |
| `SSL_KEY` | PEM-encoded private key. |
| `AUTH_TOKEN` | Optional `user:pass`. When set, every WS client must send `Authorization: Basic base64(user:pass)`. |

The exact invocation used inside droplet images is in
[core-droplet-images/src/common/startup_scripts/vnc_startup.sh](../core-droplet-images/src/common/startup_scripts/vnc_startup.sh):

```sh
flowcase_audio_out flowcaseaudio 8081 4901 self.pem self.pem flowcase_user:$VNC_PW
```

## Manual end-to-end test

```sh
# 1. Generate a self-signed cert
mkdir -p /tmp/audio-out
openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout /tmp/audio-out/key.pem -out /tmp/audio-out/cert.pem \
    -days 1 -subj '/CN=localhost'

# 2. Start the relay
./target/release/flowcase_audio_out smoke 18181 14901 \
    /tmp/audio-out/cert.pem /tmp/audio-out/key.pem &
RELAY_PID=$!

# 3. Pipe audio into the ingest port (silence works as well as anything)
ffmpeg -f lavfi -i anullsrc=r=44100:cl=stereo -t 5 \
    -f mpegts http://localhost:18181/smoke

# 4. From another shell, listen on the WSS port
wscat --no-check -c wss://localhost:14901/

# 5. Tear down
kill $RELAY_PID
```

Expected: every chunk written by ffmpeg shows up as a binary frame on the
WS connection. Logs report `ingest connection accepted`,
`wss broadcast server listening`, `New WebSocket Connection`.

## Tests

```sh
cargo test
```

The suite exercises CLI parsing, the rustls config loader, the ingest
handler, and the WebSocket broadcast handler (including the 4001 close
on bad auth). The end-to-end POST → broadcast test runs both servers
on plain HTTP via `axum::serve` for speed; TLS plumbing is checked
manually (above) because exercising it in a unit test requires shipping
ffmpeg + a wss client.

## License

See [LICENSE](LICENSE).
