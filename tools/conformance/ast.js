const flow=require('flow-parser');
const OPTS={flow:'all'};
function strip(n){ if(Array.isArray(n)) return n.map(strip); if(n&&typeof n==='object'){const o={};for(const k in n){if(k==='loc'||k==='range'||k==='comments'||k==='trailingComments'||k==='leadingComments')continue;o[k]=strip(n[k]);}return o;} return n;}
const src=process.argv[2]; const r=flow.parse(src,OPTS);
if(r.errors&&r.errors.length) console.log('ERRORS:',r.errors.map(e=>e.message));
const path=process.argv[3]; let node=strip(r.body); if(path) node=eval('node'+path);
console.log(JSON.stringify(node,(k,v)=>typeof v==="bigint"?v.toString()+"n":v));
