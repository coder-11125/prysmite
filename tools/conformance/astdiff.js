// usage: node astdiff.js files.txt [--show Type.field] [--limit N]
const J=(x)=>JSON.stringify(x,(k,v)=>typeof v==="bigint"?v.toString()+"n":v);
const fs=require('fs'),cp=require('child_process'),flow=require('flow-parser'),{flowOptions,skip,cliFlags,recordedTree}=require('./opts');
const BIN=process.env.PRYSMITE_BIN || require('path').resolve(__dirname, '../../target/release/prysmite');

function optsFor(f){ let ex={}; try{ex=JSON.parse(fs.readFileSync(f.replace(/\.js$/,'.options.json'),'utf8'))}catch(e){} return ex; }
const args=process.argv.slice(3); const show=args.includes('--show')?args[args.indexOf('--show')+1]:null;
const limit=args.includes('--limit')?+args[args.indexOf('--limit')+1]:2000000;
const SKIP=new Set([...(process.argv.includes('--trees')||process.argv.includes('--oracle')?['sourceType']:[]),'loc','range','start','end','comments','trailingComments','leadingComments','tokens','errors','docblock','interpreter']);
const checkRanges=args.includes('--ranges'); let MAP=null; const counts={}, examples={};
let compared=0, identical=0;
function rec(key,file,detail){counts[key]=(counts[key]||0)+1;(examples[key]=examples[key]||[]).length<3&&examples[key].push(file+' '+detail);}
function diff(a,b,file,parent,field){ // a=flow, b=mine
  if(a===b) return true;
  if(typeof a==='number'&&!isFinite(a)&&b===null) return true; // JSON can't carry NaN/Infinity
  const ta=a&&typeof a==='object', tb=b&&typeof b==='object';
  if(Array.isArray(a)||Array.isArray(b)){
    if(!Array.isArray(a)||!Array.isArray(b)){rec(`${parent}.${field}: array-vs-${Array.isArray(a)?typeof b:typeof a}`,file,'');return false;}
    if(a.length!==b.length){rec(`${parent}.${field}: array length`,file,`flow=${a.length} mine=${b.length}`);return false;}
    let ok=true; for(let i=0;i<a.length;i++) ok=diff(a[i],b[i],file,parent,field)&&ok; return ok;
  }
  if(ta&&tb){
    if(a.type!==b.type){rec(`${parent}.${field}: type ${a.type} vs ${b.type}`,file,'');return false;}
    let ok=true; const keys=new Set([...Object.keys(a),...Object.keys(b)]);
    if(checkRanges&&a.range&&b.start!==undefined){const mr=[MAP[b.start],MAP[b.end]];if(mr[0]!==a.range[0]||mr[1]!==a.range[1]){rec(`${a.type}: range`,file,`exp=${a.range} mine=${mr}`);ok=false;}}
    for(const k of keys){ if(SKIP.has(k)) continue;
      if(k==='value'&&((a.type==='Literal'&&(a.literalType==='regexp'||a.literalType==='bigint'))||a.type==='BigIntLiteralTypeAnnotation')) continue;
      if(!(k in b)){ if(a[k]!==null&&a[k]!==false&&!(Array.isArray(a[k])&&!a[k].length)){rec(`${a.type}.${k}: missing in mine`,file,J(a[k]).slice(0,60)); ok=false;} continue;}
      if(!(k in a)){ if(b[k]!==null&&b[k]!==false&&!(Array.isArray(b[k])&&!b[k].length)){rec(`${a.type}.${k}: extra in mine`,file,J(b[k]).slice(0,60)); ok=false;} continue;}
      ok=diff(a[k],b[k],file,a.type,k)&&ok; }
    return ok;
  }
  if(a===null||b===null||a===undefined||b===undefined){ if((a==null&&b===false)||(b==null&&a===false)) return true; rec(`${parent}.${field}: ${J(a)?.slice(0,30)} vs ${J(b)?.slice(0,30)}`,file,'');return false;}
  rec(`${parent}.${field}: value`,file,`flow=${J(a).slice(0,40)} mine=${J(b).slice(0,40)}`); return false;
}
const RECOVER=args.includes('--recover'); // compare the partial trees of files with errors
const ORACLE=args.includes('--oracle'); // reference = Flow's own parser, run live (see oracle/README.md)
function oracleTree(f){const o=f.replace(/\.js$/,'.options.json');const r=cp.spawnSync(process.env.ORACLE_BIN||'/tmp/prysmite-oracle/target/release/oracle',fs.existsSync(o)?[f,o]:[f],{encoding:'utf8',maxBuffer:1<<30});if(r.status!==0)return null;try{return JSON.parse(r.stdout)}catch(e){return null}}
const TREES=args.includes('--trees')||ORACLE; // reference = the tree recorded next to each fixture (Flow's own parser)
const files=fs.readFileSync(process.argv[2],'utf8').split('\n').filter(f=>f&&fs.statSync(f).isFile()).slice(0,limit);
if(TREES){
  // One process per file (flags differ per fixture); comment types are on in Flow's own parser.
  const mapFor=(f)=>{const buf=Buffer.from(fs.readFileSync(f).toString('utf8'));const M=new Int32Array(buf.length+1);let u=0,i2=0;while(i2<buf.length){const c=buf[i2];const len=c<0x80?1:c<0xe0?2:c<0xf0?3:4;for(let k=0;k<len&&i2+k<buf.length;k++)M[i2+k]=u;u+=len===4?2:1;i2+=len;}M[buf.length]=u;return M;};
  for(const f of files){
    const t=ORACLE?oracleTree(f):recordedTree(f); if(!t) continue; const hasErr=!!(t.errors&&t.errors.length); if(RECOVER?!hasErr:hasErr) continue;
    const r=cp.spawnSync(BIN,['-m','--flow-native','--comment-types','--es-extras',...(RECOVER?['--recover']:[]),...cliFlags(f),f],{encoding:'utf8',maxBuffer:1<<30});
    if(r.status!==0&&!RECOVER){compared++;rec('mine rejects a file Flow accepts',f,(r.stderr.split('\n')[0]||''));continue;}
    compared++; if(checkRanges) MAP=mapFor(f);
    if(diff(t,JSON.parse(r.stdout),f,'ROOT','program')) identical++;
  }
} else
for(let i=0;i<files.length;i+=200){
  const chunk=files.slice(i,i+200).filter(f=>{if(skip(f)) return false; try{return flow.parse(fs.readFileSync(f,'utf8'),flowOptions(f)).errors.length===0}catch(e){return false}});
  if(!chunk.length) continue;
  const r=cp.spawnSync(BIN,['-m',...chunk],{encoding:'utf8',maxBuffer:1<<30});
  const lines=r.stdout.split('\n'); const mine={};
  for(let j=0;j+1<lines.length;j++){ if(lines[j].startsWith('// ')){ mine[lines[j].slice(3)]=lines[j+1]; j++; } }
  for(const f of chunk){ if(!mine[f]) continue; compared++;
    let ref; try{ref=flow.parse(fs.readFileSync(f,'utf8'),flowOptions(f));}catch(e){continue}
    if(show){ const before=Object.assign({},counts); }
    if(checkRanges){const buf=Buffer.from(fs.readFileSync(f).toString('utf8'));MAP=new Int32Array(buf.length+1);let u=0,i2=0;while(i2<buf.length){const c=buf[i2];const len=c<0x80?1:c<0xe0?2:c<0xf0?3:4;for(let k=0;k<len&&i2+k<buf.length;k++)MAP[i2+k]=u;u+=len===4?2:1;i2+=len;}MAP[buf.length]=u;}
    const ok=diff(ref,JSON.parse(mine[f]),f,'ROOT','program'); if(ok) identical++; }
}
const rows=Object.entries(counts).sort((a,b)=>b[1]-a[1]);
console.log(`compared ${compared} files; structurally identical: ${identical}`);
if(!TREES&&identical!==compared) process.exitCode=1;
for(const [k,v] of rows.slice(0,+(process.env.TOP||45))) console.log(String(v).padStart(6),k,'\n         ',(examples[k][0]||'').slice(-140));
