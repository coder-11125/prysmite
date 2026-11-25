// usage: node cmp.js files.txt  -> accept/reject comparison with flow-parser
const fs = require('fs'), cp = require('child_process'), flow = require('flow-parser'), { flowOptions, skip, cliFlags, recordedTree } = require('./opts');
const BIN = process.env.PRYSMITE_BIN || require('path').resolve(__dirname, '../../target/release/prysmite');
const files = fs.readFileSync(process.argv[2], 'utf8').split('\n').filter(f => f && fs.statSync(f).isFile());

let both = 0, onlyFlow = [], onlyMine = [], neither = 0;
for (const f of files) {
  const src = fs.readFileSync(f, 'utf8');
  let flowOk;
  if (skip(f)) { const t = recordedTree(f); if (!t) continue; flowOk = !(t.errors && t.errors.length); }
  else { try { flowOk = (flow.parse(src, flowOptions(f)).errors || []).length === 0; } catch (e) { flowOk = false; } }
  const r = cp.spawnSync(BIN, ['--check', ...cliFlags(f), f], { encoding: 'utf8' });
  const mineOk = r.status === 0;
  if (flowOk && mineOk) both++;
  else if (flowOk && !mineOk) {
    const m = /^error: (.*)\n\s*--> .*?:(\d+):(\d+)\n(?:.*\n)?(\d+) \| (.*)$/m.exec(r.stderr) || [];
    onlyFlow.push({ f, msg: m[1], line: +m[2], col: +m[3], text: m[5] });
  } else if (!flowOk && mineOk) onlyMine.push(f); else neither++;
}
fs.writeFileSync(process.argv[3] || 'cmp.json', JSON.stringify({ both, onlyFlow, onlyMine, neither }, null, 1));
process.exitCode = onlyFlow.length || onlyMine.length ? 1 : 0;
console.log(`both accept: ${both}, flow-only accepts (prysmite WRONGLY rejects): ${onlyFlow.length}, prysmite-only accepts: ${onlyMine.length}, both reject: ${neither}`);
