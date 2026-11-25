// Compare Program.comments (type, value, range) between flow-parser and prysmite.
const fs = require('fs'), cp = require('child_process'), path = require('path');
const flow = require('flow-parser');
const { flowOptions, skip } = require('./opts');
const BIN = process.env.PRYSMITE_BIN || path.resolve(__dirname, '../../target/release/prysmite');
const files = fs.readFileSync(process.argv[2], 'utf8').split('\n').filter(f => f && fs.existsSync(f) && !skip(f));
let compared = 0, same = 0; const bad = [];
for (let i = 0; i < files.length; i += 200) {
  const chunk = files.slice(i, i + 200).filter(f => { try { return flow.parse(fs.readFileSync(f, 'utf8'), flowOptions(f)).errors.length === 0; } catch (e) { return false; } });
  if (!chunk.length) continue;
  const r = cp.spawnSync(BIN, ['-m', ...chunk], { encoding: 'utf8', maxBuffer: 1 << 30 });
  const lines = r.stdout.split('\n'), mine = {};
  for (let j = 0; j + 1 < lines.length; j++) if (lines[j].startsWith('// ')) { mine[lines[j].slice(3)] = lines[j + 1]; j++; }
  for (const f of chunk) {
    if (!mine[f]) continue;
    compared++;
    const src = fs.readFileSync(f).toString('utf8');
    const buf = Buffer.from(src);
    const MAP = new Int32Array(buf.length + 1); let u = 0, k = 0;
    while (k < buf.length) { const c = buf[k]; const len = c < 0x80 ? 1 : c < 0xe0 ? 2 : c < 0xf0 ? 3 : 4; for (let m = 0; m < len && k + m < buf.length; m++) MAP[k + m] = u; u += len === 4 ? 2 : 1; k += len; }
    MAP[buf.length] = u;
    const ref = flow.parse(src, flowOptions(f)).comments.map(c => [c.type, c.value, c.range[0], c.range[1]]);
    const got = (JSON.parse(mine[f]).comments || []).map(c => [c.type, c.value, MAP[c.start], MAP[c.end]]);
    if (JSON.stringify(ref) === JSON.stringify(got)) same++; else bad.push(f);
  }
}
console.log(`compared ${compared} files; comments identical: ${same}`);
if (same !== compared) process.exitCode = 1;
for (const f of bad.slice(0, 10)) console.log('  differs:', f);
