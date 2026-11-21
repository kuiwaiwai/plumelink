// Release resource and real pairing smoke test. Does not send mouse/text/volume commands.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { basename, resolve } from 'node:path';
import { mkdir, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { chromium } from '@playwright/test';
function processRpc(exe, args, options = {}) {
  const child = spawn(exe, args, {
    windowsHide: true,
    stdio: ["pipe", "pipe", "pipe"],
    ...options,
  });
  const pending = [],
    buffer = [];
  let failure;
  child.stderr.on("data", (d) => process.stderr.write(d));
  createInterface({ input: child.stdout }).on("line", (line) => {
    try {
      const v = JSON.parse(line);
      const p = pending.shift();
      p ? p.resolve(v) : buffer.push(v);
    } catch {}
  });
  const fail = (e) => {
    failure = e;
    for (const p of pending.splice(0)) p.reject(e);
  };
  child.on("error", fail);
  child.on("exit", (code) => fail(new Error(`Process exited: ${code}`)));
  const next = () =>
    buffer.length
      ? Promise.resolve(buffer.shift())
      : failure
        ? Promise.reject(failure)
        : new Promise((resolve, reject) => {
            const timer = setTimeout(
              () => reject(new Error("Diagnostic timeout")),
              12000,
            );
            pending.push({
              resolve: (v) => {
                clearTimeout(timer);
                resolve(v);
              },
              reject: (e) => {
                clearTimeout(timer);
                reject(e);
              },
            });
          });
  return {
    child,
    next,
    request: async (value) => {
      const answer = next();
      child.stdin.write(
        (typeof value === "string" ? value : JSON.stringify(value)) + "\n",
      );
      const v = await answer;
      if (!v.ok) throw Error(v.error);
      return v.result;
    },
  };
}

const executable=resolve(process.env.PLUMELINK_EXECUTABLE || 'src-tauri/target/release/PlumeLink.exe');
await mkdir('artifacts/mobile-ui',{recursive:true});
const app=processRpc(executable,['--verify-server'],{cwd:process.env.TEMP,env:{...process.env,Path:process.env.SystemRoot+'\\System32;'+process.env.SystemRoot}});
try {
 const ready=await app.next();
 assert.ok(ready.ready);
 for (const path of ['/', '/index.html', '/assets/index.js']) {
   const response = await fetch(ready.origin + path);
   assert.equal(response.status, 404, `${path} must not expose the mobile UI`);
   assert.ok(!(await response.text()).includes('<html'));
 }
 const paired = await fetch(ready.origin + '/api/pair', {
   method: 'POST',
   headers: { 'Content-Type': 'application/json', Origin: ready.origin },
   body: JSON.stringify({ code: ready.code, deviceId: 'package-smoke-device', deviceName: '测试 Android 设备' }),
 });
 assert.equal(paired.status, 200);
 const { token } = await paired.json();
 assert.match(token, /^[a-f0-9]{64}$/);
 const browser = await chromium.launch({channel:'chrome',headless:true});
 try {
   const page = await browser.newPage();
   await page.context().grantPermissions(['local-network-access'], {origin:ready.origin});
   page.on('console', message => { if (message.type() === 'error') console.error(message.text()); });
   const probe = ready.origin + '/qfast-package-probe';
   // A test-only browser document supplies the correct Origin; the server still exposes no UI.
   await page.route(probe, route => route.fulfill({status:200,contentType:'text/html',body:'<!doctype html><title>Package protocol probe</title>'}));
   await page.goto(probe);
   const capability = await page.evaluate(({origin,token}) => new Promise((resolve,reject) => {
     const ws = new WebSocket(origin.replace(/^http/,'ws') + '/ws');
     const timer = setTimeout(()=>{ws.close();reject(new Error('QFast negotiation timeout'));},4000);
     ws.onopen = ()=>ws.send(JSON.stringify({kind:'auth',token}));
     ws.onerror = ()=>{clearTimeout(timer);reject(new Error('Package WebSocket failed'));};
     ws.onmessage = event=>{
       try {
         const frame=JSON.parse(event.data);
         if(frame.kind!=='ready') throw new Error('Expected ready');
         clearTimeout(timer); ws.close(); resolve({qfast:frame.qfast,qft:frame.qft});
       } catch(error) { clearTimeout(timer); ws.close(); reject(error); }
     };
   }),{origin:ready.origin,token});
   assert.equal(capability.qfast,undefined,'only QFT2 should be advertised');
   assert.match(capability.qft.session,/^[1-9][0-9]{0,19}$/);
   assert.deepEqual({...capability.qft,session:"session"},{version:2,frameBytes:64,session:"session",leaseMs:15000,
     motionTtlMs:400,actionTtlMs:800,maxCatchup:16384,adapter:"logical-pointer-v1"});
 } finally { await browser.close(); }
 const status = await fetch(ready.origin + '/api/status', { headers: { Authorization: `Bearer ${token}`, Origin: ready.origin } });
 assert.equal(status.status, 200);
 const headers={Authorization:`Bearer ${token}`,Origin:ready.origin};
 const files=await fetch(ready.origin+'/api/files',{headers});
 assert.equal((await files.json()).maxBytes,2*1024**3);
 const audio=await fetch(ready.origin+'/api/microphone/status',{headers});
 assert.equal(audio.status,200);
 const audioState=await audio.json();assert.equal(audioState.allowed,true);assert.equal(audioState.status.active,false);
 const uploadId='a123456789abcdef0123456789abcdef';
 const upload=await fetch(ready.origin+'/api/uploads',{method:'POST',headers:{...headers,'Content-Type':'application/json'},body:JSON.stringify({id:uploadId,name:'size-boundary.bin',size:2*1024**3})});
 assert.equal(upload.status,200);assert.equal((await upload.json()).size,2*1024**3);
 const cancel=await fetch(ready.origin+'/api/uploads/'+uploadId,{method:'DELETE',headers});assert.equal(cancel.status,200);
 await app.request({action:'revoke'});
 const rejected = await fetch(ready.origin + '/api/status', { headers: { Authorization: `Bearer ${token}`, Origin: ready.origin } });
 assert.equal(rejected.status, 401);
 await writeFile('artifacts/mobile-ui/package-results.json',JSON.stringify({executable:basename(executable),checks:['mobile page unavailable over HTTP','real HTTP pairing','QFT2 capability without legacy transport','authenticated status API','2GB limit and resumable session create/cancel','microphone status and permission','explicit revoke invalidates credentials'],systemInput:'No remote-control commands or recording were exercised in this package smoke test'},null,2));
 console.log('App-only page policy, pairing, QFT2 capability and revocation passed.');
} finally {await app.request({action:'quit'}).catch(()=>{});app.child.stdin.end();}
