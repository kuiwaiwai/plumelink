// Desktop preview plus mobile captures from the QFT2 fixture; no real system input.
import { createServer } from 'node:http';
import { spawnSync } from 'node:child_process';
import { readFile, mkdir, copyFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';
const root=resolve('tauri-dist'), output=resolve('artifacts/readme-previews');
const {version}=JSON.parse(await readFile('package.json','utf8'));
await mkdir(output,{recursive:true});
const server=createServer(async(req,res)=>{
 const path=resolve(root,'.'+new URL(req.url,'http://localhost').pathname);
 if(!path.startsWith(root+sep)){res.writeHead(403).end();return;}
 try{res.setHeader('Content-Type',({'.html':'text/html; charset=utf-8','.js':'text/javascript','.css':'text/css','.png':'image/png'})[extname(path)]||'application/octet-stream');res.end(await readFile(path));}catch{res.writeHead(404).end();}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const origin=`http://127.0.0.1:${server.address().port}`;
const browser=await chromium.launch({channel:'chrome',headless:true});
try{
 const desktop=await browser.newPage({viewport:{width:760,height:680},deviceScaleFactor:2});
 await desktop.addInitScript(version=>{window.__TAURI_INTERNALS__={invoke:async()=>({halo:{enabled:true,size:"medium"},discovery:{active:true,error:''},admin:{active:false},recentController:null,running:true,connected:false,connectedCount:0,trusted:false,pairedCount:0,devices:[],controlMode:"free",primaryDevice:null,controlRequest:null,autostart:false,code:'123456',codeRemaining:600,addresses:[{name:'演示网络',address:'192.0.2.10',virtual:false}],selected:'192.0.2.10',port:17840,error:'',receivePath:'Downloads\\羽连',sharedFiles:[]})};},version);
 await desktop.goto(origin+'/desktop-ui/index.html');await desktop.locator('.qr-card img').waitFor();await desktop.evaluate(()=>document.fonts.ready);
 // The preview does not implement native window controls; let its transient notice expire.
 await desktop.waitForTimeout(5500);
 await desktop.screenshot({path:resolve(output,'desktop-pairing.png'),fullPage:true});
}finally{await browser.close();await new Promise(r=>server.close(r));}

const verified=spawnSync(process.execPath,['scripts/verify-mobile-ui.mjs'],{stdio:'inherit',windowsHide:true});
if(verified.error || verified.status!==0) throw verified.error || new Error('Mobile QFT2 screenshot verification failed');
for(const [source,target] of [['remote-light','mobile-remote'],['actions','mobile-more'],['pair','mobile-pairing']])
 await copyFile(resolve('artifacts/android-ui',source+'.png'),resolve(output,target+'.png'));
console.log('Captured desktop preview and verified QFT2 mobile UI; no real system input.');
