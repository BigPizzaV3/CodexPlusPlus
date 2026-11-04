'use strict';

// This fixture uses a publicly committed test-only key and binds only to loopback.
const fs = require('node:fs');
const http = require('node:http');
const https = require('node:https');
const net = require('node:net');
const path = require('node:path');
const readline = require('node:readline');

const scheme = process.argv[2];
if (scheme !== 'http' && scheme !== 'https') throw new Error('invalid redirect scheme');
const expectedAuth = 'Bearer skill-proxy-fictional-token';
const events = {
  starts: 0,
  initialToken: false,
  redirects: 0,
  secureFinals: 0,
  secureFinalToken: false,
  directHttp: 0,
  directHttpToken: false,
  proxiedHttp: 0,
  proxiedHttpToken: false,
  proxyTarget: '',
  proxyConnects: 0,
  targetConnections: 0,
  firstBytes: [],
  secureConnections: 0,
  tlsErrors: [],
};

function recordHttp(req, res, proxied) {
  if (proxied) {
    events.proxiedHttp += 1;
    events.proxiedHttpToken ||= req.headers.authorization === expectedAuth;
    events.proxyTarget = req.url;
  } else {
    events.directHttp += 1;
    events.directHttpToken ||= req.headers.authorization === expectedAuth;
  }
  res.writeHead(200, { 'content-type': 'text/plain', connection: 'close' });
  res.end('plain-final');
}

const plain = http.createServer((req, res) => recordHttp(req, res, false));
plain.on('clientError', (_error, socket) => socket.destroy());
const proxy = http.createServer((req, res) => recordHttp(req, res, true));
proxy.on('connect', (_req, socket) => {
  events.proxyConnects += 1;
  socket.end('HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n');
});
proxy.on('clientError', (_error, socket) => socket.destroy());

const secure = https.createServer({
  cert: fs.readFileSync(path.join(__dirname, 'test_cert.pem')),
  key: fs.readFileSync(path.join(__dirname, 'test_key.pem')),
}, (req, res) => {
  if (req.url === '/start') {
    events.starts += 1;
    events.initialToken ||= req.headers.authorization === expectedAuth;
    events.redirects += 1;
    const port = target.address().port;
    res.writeHead(302, {
      location: `${scheme}://api.github.com:${port}/final`,
      connection: 'close',
    });
    res.end();
  } else if (req.url === '/final') {
    events.secureFinals += 1;
    events.secureFinalToken ||= req.headers.authorization === expectedAuth;
    res.writeHead(200, { 'content-type': 'text/plain', connection: 'close' });
    res.end('secure-final');
  } else {
    res.writeHead(404, { connection: 'close' });
    res.end();
  }
});
secure.on('secureConnection', () => { events.secureConnections += 1; });
secure.on('tlsClientError', (error, socket) => {
  events.tlsErrors.push(error.message);
  socket.destroy();
});

// A single port accepts both protocols, so direct plaintext is observed too.
const target = net.createServer((socket) => {
  events.targetConnections += 1;
  socket.on('error', () => {});
  socket.setTimeout(5000, () => socket.destroy());
  socket.once('data', (data) => {
    events.firstBytes.push(data[0]);
    socket.pause();
    const upstream = net.connect({
      host: '127.0.0.1',
      port: (data[0] === 0x16 ? secure : plain).address().port,
    });
    upstream.on('error', () => socket.destroy());
    upstream.once('connect', () => {
      upstream.write(data);
      socket.pipe(upstream);
      upstream.pipe(socket);
      socket.resume();
    });
  });
});

proxy.listen(0, '127.0.0.1', () => {
  plain.listen(0, '127.0.0.1', () => {
    secure.listen(0, '127.0.0.1', () => {
      target.listen(0, '127.0.0.1', () => {
        console.log(`READY ${target.address().port} ${proxy.address().port}`);
      });
    });
  });
});

const input = readline.createInterface({ input: process.stdin });
input.on('line', (line) => {
  if (line === 'report') {
    process.stdout.write(`${JSON.stringify(events)}\n`, () => process.exit(0));
  }
});
input.on('close', () => process.exit(0));
