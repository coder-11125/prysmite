// usage: node both.js 'code' [path]  -> shows flow vs mine, highlighting first differing subtrees
const flow=require('flow-parser'),cp=require('child_process'),fs=require('fs');
const BIN=process.env.PRYSMITE_BIN || require('path').resolve(__dirname, '../../target/release/prysmite');
const OPTS={flow:'all'};
const SKIP=new Set(['loc','range','start','end','comments','interpreter']);
const J=(x)=>JSON.stringify(x,(k,v)=>SKIP.has(k)?undefined:typeof v==='bigint'?v.toString()+'n':v);
const src=process.argv[2];
const f=flow.parse(src,OPTS); if(f.errors.length) console.log('FLOW ERRORS:',f.errors.map(e=>e.message));
const r=cp.spawnSync(BIN,['-m','-e',src],{encoding:'utf8'}); if(r.status) console.log('MINE ERROR:',r.stderr.split('\n')[0]);
const fj=JSON.parse(J(f)), mj=r.stdout?JSON.parse(r.stdout):null;
function walk(a,b,p){ if(J(a)===J(b)) return; if(a&&b&&typeof a==='object'&&typeof b==='object'&&!Array.isArray(a)&&a.type===b.type){ let any=false; for(const k of new Set([...Object.keys(a),...Object.keys(b)])){ if(SKIP.has(k)) continue; if(J(a[k])!==J(b[k])){ const d=walk(a[k],b[k],p+'.'+k); any=true; } } return; }
  if(Array.isArray(a)&&Array.isArray(b)&&a.length===b.length){ a.forEach((x,i)=>walk(x,b[i],p+'['+i+']')); return; }
  console.log('DIFF at',p,'\n  flow:',J(a)?.slice(0,260),'\n  mine:',J(b)?.slice(0,260)); }
if(mj) walk(fj,mj,'');
