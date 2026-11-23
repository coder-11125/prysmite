// usage: node rng.js 'code'  -> prints "Type text" for every node in flow's tree and in mine, marking differences
const flow=require('flow-parser'),cp=require('child_process');
const BIN=process.env.PRYSMITE_BIN || require('path').resolve(__dirname, '../../target/release/prysmite');
const O={flow:'all'};
const src=process.argv[2];
function collect(n,pick,out=[]){ if(Array.isArray(n)) n.forEach(x=>collect(x,pick,out)); else if(n&&typeof n==='object'){ const r=pick(n); if(n.type&&r) out.push([n.type, src.slice(r[0],r[1])]); for(const k in n) if(!['loc','range','start','end'].includes(k)) collect(n[k],pick,out);} return out; }
const f=collect(flow.parse(src,O).body,n=>n.range);
const m=collect(JSON.parse(cp.spawnSync(BIN,['-m','-e',src],{encoding:'utf8'}).stdout).body,n=>n.start!==undefined?[n.start,n.end]:null);
const w=Math.max(f.length,m.length);
for(let i=0;i<w;i++){ const a=f[i],b=m[i]; const same=a&&b&&a[0]===b[0]&&a[1]===b[1]; console.log((same?'  ':'≠ ')+(a?a[0]+' '+JSON.stringify(a[1]):'-').padEnd(48)+(same?'':'| mine: '+(b?b[0]+' '+JSON.stringify(b[1]):'-'))); }
