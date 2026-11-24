// usage: ORACLE=/path/to/oracle node oracle-cmp.js files.txt [--show N] [--recover]
// Runs Flow's own Rust parser (built from facebook/flow's rust_port, see oracle/README.md) with each
// fixture's real options, and compares: accept/reject, first error message, first error position,
// and (with --recover) the whole error list.
const fs = require('fs'), cp = require('child_process'), path = require('path');
const { fixtureOptions, cliFlags } = require('./opts');
const BIN = process.env.PRYSMITE_BIN || path.resolve(__dirname, '../../target/release/prysmite');
const ORACLE = process.env.ORACLE || '/tmp/oracle/target/release/oracle';
const args = process.argv.slice(3);
const show = args.includes('--show') ? +args[args.indexOf('--show') + 1] : 5;
const recover = args.includes('--recover');
const files = fs.readFileSync(process.argv[2], 'utf8').split('\n').filter((f) => f && fs.existsSync(f));
const c = { total: 0, bothAccept: 0, bothReject: 0, flowOnly: 0, mineOnly: 0, msgEq: 0, posEq: 0, bothEq: 0, anyMsg: 0, listEq: 0 };
const ex = { flowOnly: [], mineOnly: [], msg: [], pos: [], list: [] };
function parseMine(stderr) {
  const out = [];
  const re = /error: ([^\n]*)\n\s*--> [^\n]*?:(\d+):(\d+)/g; let m;
  while ((m = re.exec(stderr))) out.push({ msg: m[1], line: +m[2], col: +m[3] - 1 });
  return out;
}
for (const f of files) {
  const optFile = f.replace(/\.js$/, '.options.json');
  const o = fs.existsSync(optFile) ? optFile : '';
  const fo = fixtureOptions(f);
  const r = cp.spawnSync(ORACLE, o ? [f, o] : [f], { encoding: 'utf8', maxBuffer: 1 << 30 });
  if (r.status !== 0) continue; // oracle crashed (e.g. unsupported options)
  let tree; try { tree = JSON.parse(r.stdout); } catch (e) { continue; }
  const errs = (tree.errors || []).map((e) => ({ msg: e.message, line: e.loc.start.line, col: e.loc.start.column }));
  const flags = [...cliFlags(f), '--comment-types', '--es-extras'];
  if (fo.pattern_matching === false || fo.records === false) continue;
  const m = cp.spawnSync(BIN, ['-c', ...(recover ? ['--recover'] : []), ...flags, f], { encoding: 'utf8', maxBuffer: 1 << 30 });
  c.total++;
  const mine = parseMine(m.stderr);
  const mineRej = m.status === 1;
  if (!errs.length && !mineRej) { c.bothAccept++; continue; }
  if (errs.length && !mineRej) { c.flowOnly++; ex.flowOnly.push(f + ' :: ' + errs[0].msg); continue; }
  if (!errs.length && mineRej) { c.mineOnly++; ex.mineOnly.push(f + ' :: ' + (mine[0] || {}).msg); continue; }
  c.bothReject++;
  const a = errs[0], b = mine[0] || {};
  const msgEq = a.msg === b.msg, posEq = a.line === b.line && a.col === b.col;
  if (msgEq) c.msgEq++; if (posEq) c.posEq++; if (msgEq && posEq) c.bothEq++;
  if (errs.some((e) => e.msg === b.msg)) c.anyMsg++;
  if (!msgEq) ex.msg.push(`${f.replace(/.*test\//, '')} flow=${a.line}:${a.col} ${a.msg} | mine=${b.line}:${b.col} ${b.msg}`);
  else if (!posEq) ex.pos.push(`${f.replace(/.*test\//, '')} flow=${a.line}:${a.col} mine=${b.line}:${b.col} ${a.msg}`);
  if (recover) {
    const same = errs.length === mine.length && errs.every((e, i) => e.msg === mine[i].msg && e.line === mine[i].line && e.col === mine[i].col);
    if (same) c.listEq++; else ex.list.push(`${f.replace(/.*test\//, '')} flow=${errs.length} mine=${mine.length}`);
  }
}
console.log(JSON.stringify(c));
for (const k of Object.keys(ex)) { if (ex[k].length) { console.log(`-- ${k}: ${ex[k].length}`); ex[k].slice(0, show).forEach((x) => console.log('   ' + x)); } }
