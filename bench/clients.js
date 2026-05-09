// Open N WebSocket clients to the given wss URL and count received bytes.
// Used by bench/run.sh to load each server with a known number of subscribers.
const WebSocket = require('ws');

const url = process.argv[2];
const N = parseInt(process.argv[3] || '50', 10);
const auth = process.argv[4]; // optional `user:pass`

let totalBytes = 0;
let firstFrameAt = null;

const opts = { rejectUnauthorized: false };
if (auth) {
  opts.headers = { Authorization: 'Basic ' + Buffer.from(auth).toString('base64') };
}

const sockets = [];
for (let i = 0; i < N; i++) {
  const ws = new WebSocket(url, opts);
  ws.on('open', () => {});
  ws.on('error', (err) => {
    process.stderr.write(`ws err: ${err.message}\n`);
  });
  ws.on('message', (data) => {
    if (firstFrameAt === null) firstFrameAt = Date.now();
    totalBytes += data.length;
  });
  sockets.push(ws);
}

process.on('SIGTERM', () => {
  console.log(JSON.stringify({
    clients: N,
    totalBytes,
    firstFrameAt,
    finishedAt: Date.now(),
  }));
  for (const s of sockets) s.terminate();
  process.exit(0);
});

setInterval(() => {}, 60_000); // keep alive
