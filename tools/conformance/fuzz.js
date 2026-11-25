// Mutation differential: corrupt fixtures at the token level and compare accept/reject between
// flow-parser and prysmite. Disagreements are minimized by shrinking the mutated file.
//
//   node fuzz.js files.txt [count] [seed]
//
// Prints a summary and writes disagreements (with the first lines of context) to fuzz-report.json.
const fs = require('fs'), os = require('os'), path = require('path'), cp = require('child_process');
const flow = require('flow-parser');
const { flowOptions, skip } = require('./opts');
const BIN = process.env.PRYSMITE_BIN || path.resolve(__dirname, '../../target/release/prysmite');
const files = fs.readFileSync(process.argv[2], 'utf8').split('\n').filter(f => f && fs.existsSync(f) && !skip(f));
const COUNT = +(process.argv[3] || 5000);
let seed = +(process.argv[4] || 1);
const rnd = () => { seed = (seed * 1664525 + 1013904223) >>> 0; return seed / 4294967296; };
const pick = a => a[Math.floor(rnd() * a.length)];

const TOKEN = /\/\/[^\n]*|\/\*[\s\S]*?\*\/|'(?:[^'\\\n]|\\.)*'|"(?:[^"\\\n]|\\.)*"|`(?:[^`\\]|\\[\s\S])*`|[A-Za-z_$#@][\w$]*|\d[\w.]*|\.\.\.|=>|[=!<>]=?=?|&&|\|\||\?\?|\?\.|[^\s\w]|\s+/g;
const POOL = ['(', ')', '{', '}', '[', ']', ',', ';', ':', '?', '!', '=', '=>', '<', '>', '|', '&', '...', '.', '*', '+', '-', '@', '#',
  'type', 'declare', 'class', 'function', 'async', 'await', 'yield', 'static', 'get', 'set', 'extends', 'implements', 'interface',
  'opaque', 'enum', 'import', 'export', 'default', 'as', 'of', 'in', 'return', 'this', 'super', 'new', 'const', 'let', 'var',
  'component', 'hook', 'match', 'record', 'renders', 'keyof', 'readonly', 'infer', 'is', 'asserts', 'abstract', 'override', 'x', '1', "'s'", '`t`'];

function mutate(src) {
  const toks = src.match(TOKEN) || [];
  const idx = [];
  toks.forEach((t, i) => { if (!/^\s+$/.test(t) && !/^\/[/*]/.test(t)) idx.push(i); });
  if (!idx.length) return null;
  const out = toks.slice();
  const n = 1 + (rnd() < 0.2 ? 1 : 0);
  for (let k = 0; k < n; k++) {
    const i = pick(idx);
    const r = rnd();
    if (r < 0.3) out[i] = '';
    else if (r < 0.5) out[i] = out[i] + ' ' + out[i];
    else if (r < 0.7) out[i] = pick(POOL);
    else if (r < 0.85) out[i] = out[i] + ' ' + pick(POOL) + ' ';
    else { const j = pick(idx); const t = out[i]; out[i] = out[j]; out[j] = t; }
  }
  return out.join('');
}

function flowAccepts(src, f) {
  try { return (flow.parse(src, flowOptions(f)).errors || []).length === 0; } catch (e) { return false; }
}

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'prysmite-fuzz-'));
const mutants = [];
while (mutants.length < COUNT) {
  const f = pick(files);
  let src;
  try { src = fs.readFileSync(f, 'utf8'); } catch (e) { continue; }
  if (src.length > 20000) continue;
  const m = mutate(src);
  if (m === null || m === src) continue;
  mutants.push({ f, src: m });
}
mutants.forEach((m, i) => { m.path = path.join(dir, `m${i}.js`); fs.writeFileSync(m.path, m.src); });
// prysmite in batches (one process per batch); a failing file shows up in the diagnostics.
const rejected = new Set();
for (let i = 0; i < mutants.length; i += 400) {
  const batch = mutants.slice(i, i + 400);
  const r = cp.spawnSync(BIN, ['--check', ...batch.map(m => m.path)], { encoding: 'utf8', maxBuffer: 1 << 28 });
  for (const m of r.stderr.matchAll(/-->\s+(\S+?):\d+:\d+/g)) rejected.add(m[1]);
}
let agree = 0; const bad = [];
for (const m of mutants) {
  const fl = flowAccepts(m.src, m.f), mine = !rejected.has(m.path);
  if (fl === mine) agree++; else bad.push({ file: m.f, flow: fl ? 'accept' : 'reject', mine: mine ? 'accept' : 'reject', src: m.src });
}
// shrink each disagreement by deleting lines while it persists
function shrink(m) {
  let lines = m.src.split('\n');
  const still = (ls) => {
    const s = ls.join('\n'); const p = path.join(dir, 'shrink.js'); fs.writeFileSync(p, s);
    const fl = flowAccepts(s, m.file); const r = cp.spawnSync(BIN, ['--check', p], { encoding: 'utf8' });
    return (fl ? 'accept' : 'reject') === m.flow && ((r.status === 0) ? 'accept' : 'reject') === m.mine;
  };
  for (let pass = 0; pass < 2; pass++) {
    for (let i = lines.length - 1; i >= 0; i--) {
      const t = lines.slice(0, i).concat(lines.slice(i + 1));
      if (t.length && still(t)) lines = t;
    }
  }
  return lines.join('\n');
}
const report = bad.slice(0, +(process.env.SHRINK || 15)).map(m => ({ ...m, src: m.src.split('\n').length > 400 ? m.src.slice(0, 600) : shrink(m).slice(0, 600) }));
fs.writeFileSync(process.env.REPORT || 'fuzz-report.json', JSON.stringify(report, null, 1));
console.log(`mutants: ${mutants.length}  agree: ${agree}  disagree: ${bad.length}  (flow-accepts/mine-rejects: ${bad.filter(b => b.flow === 'accept').length}, flow-rejects/mine-accepts: ${bad.filter(b => b.flow === 'reject').length})`);
fs.rmSync(dir, { recursive: true, force: true });
