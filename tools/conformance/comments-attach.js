// usage: node comments-attach.js files.txt [--verbose] [--only substr]
// Compares leadingComments/trailingComments with Flow's recorded trees for fixtures that set
// `intern_comments`. Nodes are matched by (type, range); a node counts as matching when both its
// leading and trailing comment ranges are identical.
const fs = require('fs'), cp = require('child_process');
const { fixtureOptions, cliFlags, recordedTree } = require('./opts');
const BIN = process.env.PRYSMITE_BIN || require('path').resolve(__dirname, '../../target/release/prysmite');
const verbose = process.argv.includes('--verbose');
const only = process.argv.includes('--only') ? process.argv[process.argv.indexOf('--only') + 1] : null;
const files = fs.readFileSync(process.argv[2], 'utf8').split('\n').filter((f) => f && fs.existsSync(f) && fixtureOptions(f).intern_comments);
function collect(t, out) {
  const walk = (x) => {
    if (Array.isArray(x)) return x.forEach(walk);
    if (x && typeof x === 'object') {
      if (x.type && x.range && x.type !== 'Program') {
        const l = (x.leadingComments || []).map((c) => c.range.join('-'));
        const tr = (x.trailingComments || []).map((c) => c.range.join('-'));
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
  const t = recordedTree(f);
  if (!t || (t.errors && t.errors.length)) continue;
  nfiles++;
  const r = cp.spawnSync(BIN, ['-m', '--utf16', '--intern-comments', '--comment-types', '--es-extras', ...cliFlags(f), f], { encoding: 'utf8', maxBuffer: 1 << 30 });
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
