// Controlled CLI fixture: no credentials, provider requests, or real agent calls.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const { spawn } = require('node:child_process');
const args = process.argv.slice(2);
assert.deepEqual(args, ['--input-format', 'stream-json', '--output-format', 'stream-json',
  '--conversation', 'fixture-root', '--model', 'fixture/model with spaces']);
assert.equal(process.cwd(), fs.realpathSync(process.env.FIXTURE_CWD));
let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', chunk => { input += chunk; });
process.stdin.on('end', () => {
  const lines = input.trimEnd().split('\n');
  assert.equal(lines.length, 1);
  const message = JSON.parse(lines[0]);
  assert.deepEqual(message, { event: 'user', message: { content: process.env.FIXTURE_PROMPT } });
  fs.writeFileSync(process.env.FIXTURE_INPUT, input);
  const send = value => process.stdout.write(JSON.stringify(value) + '\r\n');
  send({ event: 'init', conversation_id: 'fixture-root' });
  const mode = process.env.FIXTURE_MODE;
  if (mode === 'no-result') return;
  if (mode === 'invalid-json') { process.stdout.write('not-json\n'); return; }
  if (mode === 'oversized') { process.stdout.write('x'.repeat(1024 * 1024 + 1)); return; }
  if (mode === 'wait') {
    spawn(process.execPath, ['-e',
      `setTimeout(() => require('node:fs').writeFileSync(process.argv[1], 'survived'), 1400)`,
      process.env.FIXTURE_SURVIVOR], { stdio: 'ignore' });
    send({ event: 'step_update', step_update: {
      conversation_id: 'fixture-root', step_type: 'agent_response', text_delta: 'ready'
    } });
    setInterval(() => {}, 1000);
    return;
  }
  const result = { conversation_id: 'fixture-root', status: 'SUCCESS', response: '完成 ✓' };
  if (mode === 'wrong-root') result.conversation_id = 'wrong-root';
  if (mode === 'missing-root') delete result.conversation_id;
  if (mode === 'missing-response') delete result.response;
  if (mode === 'empty-response') result.response = '   ';
  if (mode === 'denied') { result.status = 'ERROR'; result.error = 'Permission denied'; }
  const bytes = Buffer.from(JSON.stringify({ event: 'result', result }));
  // Split inside a UTF-8 character and omit the final newline.
  const split = bytes.indexOf(Buffer.from('完')) + 1;
  process.stdout.write(bytes.subarray(0, split));
  setTimeout(() => {
    process.stdout.write(bytes.subarray(split));
    if (mode === 'nonzero') process.exitCode = 2;
  }, 15);
});
