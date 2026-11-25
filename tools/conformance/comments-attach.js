// usage: node comments-attach.js files.txt [--verbose] [--only substr]
// Compares leadingComments/trailingComments with Flow's recorded trees for fixtures that set
// `intern_comments`. Nodes are matched by (type, range); a node counts as matching when both its
// leading and trailing comment ranges are identical.
const fs = require('fs'), cp = require('child_process');
const { fixtureOptions, cliFlags, recordedTree } = require('./opts');
const BIN = process.env.PRYSMITE_BIN || require('path').resolve(__dirname, '../../target/release/prysmite');
const verbose = process.argv.includes('--verbose');
const only = process.argv.includes('--only') ? process.argv[process.argv.indexOf('--only') + 1] : null;
const LIVE = process.argv.includes('--oracle'); // run Flow's parser live with interning forced on
const ORACLE = process.env.ORACLE || '/tmp/prysmite-oracle/target/release/oracle';
const limit = process.argv.includes('--limit') ? +process.argv[process.argv.indexOf('--limit') + 1] : 1e9;
const files = fs.readFileSync(process.argv[2], 'utf8').split('\n').filter((f) => f && fs.existsSync(f) && (LIVE || fixtureOptions(f).intern_comments)).slice(0, limit);
function liveTree(f) {
  const o = f.replace(/\.js$/, '.options.json');
  const opts = o !== f && fs.existsSync(o) ? JSON.parse(fs.readFileSync(o, 'utf8')) : null;
  if (opts === null) { /* no options file: Flow's defaults, which the oracle only applies when given none */ }
  const tmp = '/tmp/prysmite-ic-opts.json';
  fs.writeFileSync(tmp, JSON.stringify({ ...(opts || { components: true, enums: true, pattern_matching: true, records: true, esproposal_decorators: true }), intern_comments: true }));
  const r = cp.spawnSync(ORACLE, [f, tmp], { encoding: 'utf8', maxBuffer: 1 << 30 });
  try { return JSON.parse(r.stdout); } catch (e) { return null; }
}
function collect(t, out) {
  const walk = (x) => {
    if (Array.isArray(x)) return x.forEach(walk);
    if (x && typeof x === 'object') {
      if (x.start !== undefined && !x.range) x = { ...x, range: [x.start, x.end] };
      const rg = (c) => c.range || [c.start, c.end];
      if (x.type && x.range && x.type !== 'Program') {
        const l = (x.leadingComments || []).map((c) => rg(c).join('-'));
        const tr = (x.trailingComments || []).map((c) => rg(c).join('-'));
        if (l.length || tr.length) {
          const k = x.type + '@' + x.range.join('-');
          (out[k] = out[k] || []).push(JSON.stringify({ l, t: tr }));
        }
      }
      for (const k in x) if (k !== 'comments' || x.type !== 'Program') walk(x[k]);
    }
  };
  walk(t);
}
let nfiles = 0, okfiles = 0, nodes = 0, okNodes = 0, rejected = 0;
const bad = [];
for (const f of files) {
  if (only && !f.includes(only)) continue;
  const t = LIVE ? liveTree(f) : recordedTree(f);
  if (!t || (t.errors && t.errors.length)) continue;
  nfiles++;
  const r = cp.spawnSync(BIN, ['-m', '--utf16', '--intern-comments', '--flow-native', '--comment-types', '--es-extras', ...cliFlags(f), f], { encoding: 'utf8', maxBuffer: 1 << 30 });
  if (r.status !== 0) { rejected++; bad.push([f, 'REJECTED ' + r.stderr.split('\n')[0]]); continue; }
  const a = {}, b = {};
  collect(t, a);
  collect(JSON.parse(r.stdout), b);
  let ok = true;
  const msgs = [];
  for (const k of new Set([...Object.keys(a), ...Object.keys(b)])) {
    nodes++;
    if (JSON.stringify(a[k]) === JSON.stringify(b[k])) okNodes++;
    else { ok = false; msgs.push(`  ${k}\n    flow: ${a[k]}\n    mine: ${b[k]}`); }
  }
  if (ok) okfiles++; else bad.push([f, msgs.join('\n')]);
}
for (const [f, m] of bad.slice(0, verbose ? 1000 : 0)) console.log(f.replace(/.*test\/flow\//, ''), '\n' + m);
if (!verbose) for (const [f, m] of bad) console.log(f.replace(/.*test\/flow\//, ''), m.split('\n').length / 3 | 0, 'node(s)');
console.log(`files: ${okfiles}/${nfiles} identical; nodes: ${okNodes}/${nodes}; rejected: ${rejected}`);
