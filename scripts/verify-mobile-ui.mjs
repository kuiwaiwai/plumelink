// Native-style mobile UI regression on production assets and the real QFT/WSS fixture.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";
export async function verifyMobileUI({ page, context, commands, frames }) {
  const existingToken = await page.evaluate(() => localStorage.getItem('plumelink-token'));
  const out = resolve("artifacts/android-ui");await mkdir(out,{recursive:true});
  const checks=[];
  const snap=async name=>{
    await page.evaluate(async()=>{await Promise.all(document.getAnimations().filter(a=>a.effect?.getComputedTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{})));});
    await page.screenshot({path:resolve(out,`${name}.png`)});
  };
  const nav=async name=>{await page.getByRole("navigation",{name:"主导航"}).getByRole("button",{name,exact:true}).click();};
  const back=async()=>{assert.equal(await page.evaluate(()=>window.plumeLinkBack()),true);};
  for(const [width,height] of [[320,550],[360,640],[390,844],[430,932],[768,1024],[844,390]]) {
    await page.setViewportSize({width,height});
    const bounds=await page.evaluate(()=>({width:document.documentElement.scrollWidth,height:document.documentElement.scrollHeight,
      pad:document.querySelector('.touchpad').getBoundingClientRect().height,
      clipped:[...document.querySelectorAll('.remote-view button,.app-navigation button,.app-bar button')].filter(e=>{const r=e.getBoundingClientRect();return r.top<0||r.bottom>innerHeight+1||r.right>innerWidth+1||r.left<0;}).map(e=>e.textContent)}));
    assert.ok(bounds.width<=width&&bounds.height<=height&&!bounds.clipped.length&&bounds.pad>=100,JSON.stringify({width,height,...bounds}));
    checks.push({width,height,padHeight:bounds.pad,contained:true});
    if(width===320||width===844)await snap(`remote-${width}`);
  }
  await page.setViewportSize({width:320,height:350});
  await page.getByRole('button',{name:'更多',exact:true}).scrollIntoViewIfNeeded();
  const compact=await page.locator('.remote-tools').evaluate(e=>{const r=e.getBoundingClientRect();const page=e.closest('.app-content').getBoundingClientRect();return r.top>=page.top&&r.bottom<=page.bottom+1;});
  assert.ok(compact,'short screens keep tools reachable by scrolling');await snap('remote-compact');
  await page.setViewportSize({width:390,height:844});await page.locator('.remote-view').evaluate(e=>e.scrollTop=0);await snap('remote-light');
  const padHeight=await page.locator('.touchpad').evaluate(e=>e.clientHeight);
  const focusToggle=page.locator('.focus-toggle');
  assert.equal(await focusToggle.locator('svg.lucide-maximize-2').count(),1);
  assert.equal(await focusToggle.evaluate(e=>getComputedStyle(e).backgroundColor),'rgba(0, 0, 0, 0)');
  await page.getByRole('button',{name:'隐藏导航',exact:true}).click();
  assert.equal(await page.getByRole('navigation',{name:'主导航'}).isVisible(),false);
  assert.equal(await focusToggle.locator('svg.lucide-minimize-2').count(),1);
  assert.ok(await page.locator('.touchpad').evaluate(e=>e.clientHeight)>padHeight);await snap('remote-focus');
  await back();assert.ok(await page.getByRole('navigation',{name:'主导航'}).isVisible());
  const mic=page.getByRole('button',{name:'按住说话，上滑锁定',exact:true});
  await mic.click();assert.equal(await page.locator('.mic-control.recording').count(),0);
  const micBox=await mic.boundingBox();const micX=micBox.x+micBox.width/2,micY=micBox.y+micBox.height/2;
  await page.mouse.move(micX,micY);await page.mouse.down();await page.waitForTimeout(300);
  await page.locator('.mic-control.recording').waitFor();await snap('microphone-hold');
  await page.mouse.move(micX,micY-85,{steps:8});await page.waitForTimeout(400);await snap('microphone-locking');
  assert.equal(await page.locator('.mic-control.locked').count(),0);
  await page.waitForTimeout(800);await page.locator('.mic-control.locked').waitFor();await page.mouse.up();
  await page.locator('.mic-capsule').waitFor({state:'hidden'});await snap('microphone-locked');
  await page.evaluate(()=>window.dispatchEvent(new CustomEvent('plumelink-microphone', {
    detail:{id:window.__activeMic,phase:'live',locked:true,level:0,message:'系统暂停了此应用的收音，请检查通话或其他正在使用麦克风的应用'},
  })));
  await page.getByText('收音受限',{exact:true}).waitFor();
  for(const [width,height] of [[320,550],[390,844],[844,390]]) {
    await page.setViewportSize({width,height});
    assert.ok(await page.locator('.mic-compact').evaluate(e=>{const r=e.getBoundingClientRect(),pad=document.querySelector('.touchpad').getBoundingClientRect();return r.bottom<=pad.top&&r.left>=0&&r.right<=innerWidth;}));
    assert.ok(await page.locator('.focus-toggle').evaluate(e=>e.getBoundingClientRect().right<=innerWidth));
    if(width===320)await snap('microphone-silenced-320');
    if(width===844)await snap('microphone-landscape');
  }
  await page.evaluate(()=>window.dispatchEvent(new CustomEvent('plumelink-microphone', {
    detail:{id:window.__activeMic,phase:'live',locked:true,level:.58,message:''},
  })));
  await page.getByText('收音中',{exact:true}).waitFor();
  checks.push({microphone:'system-silenced warning keeps compact layout and clears on recovery'});
  await page.setViewportSize({width:390,height:844});
  const stopMic=page.locator('.mic-button[aria-pressed=true]');await stopMic.hover();
  await page.mouse.down();await page.waitForTimeout(900);await page.mouse.up();assert.ok(await stopMic.isVisible());
  await page.mouse.down();await page.waitForTimeout(2100);await page.mouse.up();
  assert.equal(await page.locator('.mic-control.recording').count(),0);
  assert.ok(await page.evaluate(()=>window.__nativeCalls.some(c=>c[0]==='mic-stop')));
  const lockMic=async()=>{
    const rect=await mic.boundingBox(), x=rect.x+rect.width/2, y=rect.y+rect.height/2;
    await page.mouse.move(x,y);await page.mouse.down();await page.waitForTimeout(300);
    await page.mouse.move(x,y-85,{steps:8});await page.waitForTimeout(1150);await page.mouse.up();
    await page.locator('.mic-control.locked').waitFor();
    await page.locator('.mic-capsule').waitFor({state:'hidden'});
  };
  await lockMic();await nav('设置');
  assert.equal(await page.locator('.mic-control.recording').count(),1);assert.ok(await page.locator('.mic-compact').isVisible());await nav('遥控');
  await page.evaluate(()=>window.dispatchEvent(new Event('pagehide')));
  assert.ok(await page.locator('.mic-control.locked').isVisible());
  await page.evaluate(()=>window.dispatchEvent(new Event('pageshow')));
  await page.locator('.mic-compact-stop').click();
  await page.locator('.mic-control.recording').waitFor({state:'hidden'});
  await mic.waitFor();await page.waitForFunction(()=>!document.querySelector('.mic-button').disabled);
  const socketsAfterBackground=frames.sockets;
  checks.push({microphone:'tap stays idle; hold-to-talk, one-second slide lock, two-second stop, navigation/background persistence and explicit stop'});
  const before=commands.length;
  for(const name of ['音量减','静音','音量加','后退','播放/暂停','前进','左键','右键'])await page.getByRole('button',{name,exact:true}).click();
  await page.waitForTimeout(100);assert.deepEqual(commands.slice(before).filter(c=>c.type!=='release').map(c=>c.type),['volume','volume','volume','key','key','key','click','click']);
  assert.ok(await page.evaluate(()=>window.__nativeCalls.some(c=>c[0]==='haptic')));
  await page.getByRole('button',{name:/键盘/}).click();await page.locator('#remote-text').fill('保留草稿，中文 🛰️');
  await back();await page.getByRole('button',{name:/键盘/}).click();
  assert.equal(await page.locator('#remote-text').inputValue(),'保留草稿，中文 🛰️');
  await page.locator('#remote-text').dispatchEvent('compositionstart');assert.ok(await page.getByRole('button',{name:'发送文字',exact:true}).isDisabled());
  await page.locator('#remote-text').dispatchEvent('compositionend');await page.getByRole('button',{name:'发送文字',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('#remote-text').value==='');assert.ok(commands.some(c=>c.type==='text'&&c.text==='保留草稿，中文 🛰️'));
  await snap('keyboard');
  await page.evaluate(()=>{const viewport=new EventTarget();Object.assign(viewport,{height:350,offsetTop:0,scale:1});Object.defineProperty(window,'visualViewport',{configurable:true,value:viewport});window.dispatchEvent(new Event('resize'));});
  await page.locator('.send-button').scrollIntoViewIfNeeded();
  assert.ok(await page.locator('.send-button').evaluate(e=>e.getBoundingClientRect().bottom<=350));await snap('keyboard-ime');
  await back();await page.evaluate(()=>{delete window.visualViewport;window.dispatchEvent(new Event('resize'));});
  await nav('设置');assert.equal(await page.locator('dialog[open]').count(),0);await snap('settings-light');
  const keepAlive = page.getByRole('switch',{name:'应用保活',exact:true});
  assert.equal(await keepAlive.getAttribute('aria-checked'),'true');
  assert.ok(await page.evaluate(()=>window.__nativeCalls.some(([name,enabled])=>name==='keep-alive'&&enabled===true)));
  await keepAlive.click();assert.equal(await keepAlive.getAttribute('aria-checked'),'false');
  assert.ok(await page.evaluate(()=>window.__nativeCalls.some(([name,enabled])=>name==='keep-alive'&&enabled===false)));
  await keepAlive.click();assert.equal(await keepAlive.getAttribute('aria-checked'),'true');
  await page.getByRole('switch',{name:'触觉反馈',exact:true}).click();assert.equal(await page.getByRole('switch',{name:'触觉反馈',exact:true}).getAttribute('aria-checked'),'false');
  await page.locator('#sensitivity').focus();await page.keyboard.press('ArrowRight');
  assert.equal(await page.locator('#sensitivity').inputValue(),'1.6');
  assert.equal(await page.locator('#sensitivity').getAttribute('aria-valuetext'),'1.6 倍');
  await page.getByRole('button',{name:'外观',exact:true}).click();
  await page.getByRole('radio',{name:'跟随系统',exact:true}).waitFor();
  await page.keyboard.press('End');assert.equal(await page.getByRole('radio',{name:'深色',exact:true}).getAttribute('aria-checked'),'true');
  await page.keyboard.press('ArrowUp');assert.equal(await page.getByRole('radio',{name:'浅色',exact:true}).getAttribute('aria-checked'),'true');
  await page.keyboard.press('Home');assert.equal(await page.getByRole('radio',{name:'跟随系统',exact:true}).getAttribute('aria-checked'),'true');
  await page.keyboard.press('End');await page.keyboard.press('Enter');
  assert.ok(await page.getByRole('button',{name:'外观',exact:true}).evaluate(e=>e===document.activeElement));
  assert.equal(await page.locator('html').getAttribute('data-theme'),'dark');await snap('settings-dark');
  await nav('遥控');await snap('remote-dark');
  await page.setViewportSize({width:320,height:550});await lockMic();await snap('microphone-dark-320');
  assert.ok(await page.locator('.mic-compact').evaluate(e=>{const r=e.getBoundingClientRect();return r.left>=0&&r.right<=innerWidth&&r.top>=0&&r.bottom<=document.querySelector('.touchpad').getBoundingClientRect().top;}));
  await page.locator('.mic-compact-stop').click();
  await page.setViewportSize({width:390,height:844});
  await nav('文件');await page.getByText('电脑文件会显示在这里',{exact:true}).waitFor();await snap('files-dark');
  const transferEvent = detail => page.evaluate(detail => window.dispatchEvent(new CustomEvent('plumelink-native-transfer', { detail })), detail);
  const checkTransferCard = async theme => {
    for (const width of [320,390]) {
      await page.setViewportSize({width,height:844});
      await transferEvent({done:false,progress:0,message:'正在接收 '+ '很长的共享文件名称_'.repeat(8)+'.zip'});
      await page.getByText('正在准备传输',{exact:true}).waitFor();
      await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
      const cancel = page.getByRole('button',{name:'取消传输',exact:true});
      const before = await cancel.boundingBox();
      await transferEvent({done:false,progress:42});
      await page.waitForFunction(()=>document.querySelector('.transfer-percent')?.textContent==='42%');
      const after = await cancel.boundingBox();
      assert.ok(Math.abs(after.x-before.x)<1,JSON.stringify({theme,width,before,after}));assert.ok(Math.abs(after.y-before.y)<1,JSON.stringify({theme,width,before,after}));
      assert.ok(await page.locator('.transfer-progress').evaluate(e=>{
        const r=e.getBoundingClientRect();return r.left>=0&&r.right<=innerWidth&&e.scrollWidth<=e.clientWidth;
      }));
      assert.ok(await cancel.evaluate(e=>e.getBoundingClientRect().height>=48));
      await snap(`transfer-${theme}-${width}`);
      await transferEvent({done:false,progress:100});
      await page.getByText('正在确认完成',{exact:true}).waitFor();
      await cancel.click();assert.ok(await page.getByRole('button',{name:'取消中…',exact:true}).isDisabled());
      await transferEvent({done:true,progress:0,message:'已取消传输'});
      await page.locator('.transfer-progress').waitFor({state:'hidden'});
    }
    await page.setViewportSize({width:390,height:844});
  };
  await checkTransferCard('dark');
  await back();assert.equal(await page.getByRole('button',{name:'遥控',exact:true}).getAttribute('aria-current'),'page');
  await nav('设置');await page.getByRole('button',{name:'外观',exact:true}).click();await page.getByRole('radio',{name:'浅色',exact:true}).click();
  await page.getByRole('button',{name:'快捷操作一',exact:true}).click();
  assert.equal(await page.getByRole('radio',{name:'回车',exact:true}).count(),0);
  assert.equal(await page.getByRole('radio',{name:'显示桌面',exact:true}).count(),0);
  await page.getByRole('radio',{name:'上一个桌面',exact:true}).click();
  await page.getByRole('button',{name:'图标一',exact:true}).click();
  await page.getByRole('radio',{name:'游戏手柄',exact:true}).click();
  await page.getByRole('button',{name:'快捷操作三',exact:true}).waitFor();
  await page.getByRole('button',{name:'手势与操作指南',exact:true}).click();await snap('gesture-help');
  assert.ok(await page.locator('dialog[open] .sheet-surface').evaluate(e=>e.getBoundingClientRect().height<=innerHeight*.88+2));
  await page.locator('dialog[open]').getByRole('button',{name:'展开抽屉'}).click();
  assert.equal(await page.locator('dialog[open] .sheet-surface.expanded').count(),1);
  await page.locator('dialog[open]').getByRole('button',{name:'收起抽屉'}).click();
  assert.equal(await page.locator('dialog[open] .sheet-surface.expanded').count(),0);
  const handle=await page.locator('dialog[open] .sheet-drag-area').boundingBox();
  await page.mouse.move(handle.x+handle.width/2,handle.y+12);await page.mouse.down();await page.mouse.move(handle.x+handle.width/2,handle.y+90,{steps:10});await page.mouse.up();
  await page.locator('dialog[open]').waitFor({state:'hidden'});
  await nav('文件');await snap('files-light');await checkTransferCard('light');await nav('遥控');
  await page.getByRole('button',{name:'更多',exact:true}).click();await snap('actions');
  assert.equal(await page.locator('dialog[open] .more-grid button').count(),9);
  assert.equal(await page.locator('dialog[open]').getByRole('button',{name:'任务视图',exact:true}).count(),0);
  assert.equal(await page.locator('dialog[open]').getByRole('button',{name:'上一个桌面',exact:true}).locator('svg.lucide-gamepad-2').count(),1);
  for(const name of ['Esc','显示桌面','删除（Backspace）','向上滚动','向下滚动','上一个桌面','通知中心','下一个应用'])await page.locator('dialog[open]').getByRole('button',{name,exact:true}).first().click();
  assert.ok(commands.some(c=>c.type==='notifications'));
  assert.ok(commands.some(c=>c.type==='appswitch'&&c.direction==='right'));
  assert.ok(commands.some(c=>c.type==='virtualdesktop'&&c.direction==='left'));
  await page.locator('dialog[open]').getByRole('button',{name:'切换窗口',exact:true}).click();await page.getByRole('button',{name:'下一个',exact:true}).click();await page.getByRole('button',{name:'确认窗口',exact:true}).click();
  await page.getByRole('button',{name:'回车',exact:true}).click();await page.waitForTimeout(100);assert.ok(commands.some(c=>c.type==='key'&&c.key==='enter'));
  await page.waitForTimeout(100);assert.ok(commands.some(c=>c.type==='switch'&&c.action==='confirm'));
  const cdp=await context.newCDPSession(page);
  const swipeTwoFingers=async (axis, sign)=>{
    const pad=await page.locator('.touchpad').boundingBox();
    const point=(id,d)=>({id,x:pad.x+60+id*50+(axis==='x'?d:0),y:pad.y+70+(axis==='y'?d:0)});
    const start=commands.length;
    await cdp.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[point(1,0),point(2,0)]});
    for(let d=6;d<=60;d+=6){
      await cdp.send('Input.dispatchTouchEvent',{type:'touchMove',touchPoints:[point(1,d),point(2,d)]});
      await page.waitForTimeout(20);
    }
    await cdp.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});
    await page.waitForTimeout(100);
    const values=commands.slice(start).filter(c=>axis==='y' ? c.type==='motion'&&c.scroll : c.type==='scroll'&&c.dx)
      .map(c=>axis==='y'?c.scroll:c.dx);
    assert.ok(values.length>0&&values.every(value=>Math.sign(value)===sign),JSON.stringify({axis,sign,values}));
  };
  await swipeTwoFingers('y',1);
  await nav('设置');
  await page.getByRole('button',{name:'滚动逻辑',exact:true}).click();
  assert.equal(await page.getByRole('radio',{name:'与手指方向相反',exact:true}).getAttribute('aria-checked'),'true');
  await page.getByRole('radio',{name:'与手指方向一致',exact:true}).click();
  assert.equal(await page.evaluate(()=>localStorage.getItem('plumelink-scroll-direction')),'follow');
  await nav('遥控');
  await swipeTwoFingers('y',-1);
  await swipeTwoFingers('x',-1);
  await page.getByRole('button',{name:'更多',exact:true}).click();const count=commands.filter(c=>c.type==='click').length;
  const focusBox=await page.locator('.focus-toggle').boundingBox();
  await page.touchscreen.tap(focusBox.x+focusBox.width/2,focusBox.y+focusBox.height/2);
  await page.waitForTimeout(100);assert.equal(await page.locator('dialog[open]').count(),0);assert.equal(commands.filter(c=>c.type==='click').length,count);
  assert.equal(await page.getByRole('navigation',{name:'主导航'}).isVisible(),true,'sheet backdrop tap must not activate focus mode');
  await nav('设置');await page.getByRole('button',{name:'外观',exact:true}).click();await back();assert.equal(await page.locator('dialog[open]').count(),0);await back();
  assert.equal(frames.sockets,socketsAfterBackground,'navigation and settings choices preserve the live session');
  // Approximate WebView text zoom without shrinking the viewport or its touch targets.
  await page.evaluate(()=>{
    window.__fontStyles=[...document.querySelectorAll('body *')].map(e=>[e,e.style.fontSize,getComputedStyle(e).fontSize]);
    for(const [e,,size] of window.__fontStyles)e.style.fontSize=`${parseFloat(size)*1.5}px`;
  });
  for(const [destination,control] of [['遥控','更多'],['设置','关闭电脑']]){
    await nav(destination);await page.getByRole('button',{name:control,exact:true}).scrollIntoViewIfNeeded();
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    assert.ok(await page.getByRole('button',{name:control,exact:true}).evaluate(e=>{const r=e.getBoundingClientRect();const content=document.querySelector('.app-content').getBoundingClientRect();return r.top>=content.top&&r.bottom<=content.bottom+1;}));
    await snap(destination==='遥控'?'remote-font150':'settings-font150');
  }
  await page.evaluate(()=>{for(const [e,size] of window.__fontStyles)e.style.fontSize=size;delete window.__fontStyles;});
  // Stub the upload endpoint to keep test files out of the configured receive directory.
  await nav('文件');
  await page.evaluate(() => { window.__folderSelected = false; window.dispatchEvent(new CustomEvent('plumelink-native-transfer',{detail:{folder:'请选择接收目录'}})); });
  await page.getByText('请先选择 Android 接收目录，再保存电脑文件。',{exact:true}).waitFor();
  const longFileName='shared-file-without-spaces-'.repeat(8)+'.zip';
  await nav('遥控');
  await page.route('**/api/files',route=>route.fulfill({status:200,contentType:'application/json',body:JSON.stringify({files:[{id:99,name:longFileName,size:1048576}]})}));
  await nav('文件');
  const saveFile=page.getByRole('button',{name:`保存 ${longFileName}`,exact:true});
  await saveFile.waitFor();assert.equal(await saveFile.isDisabled(),true);
  assert.equal(await page.locator('.transfer-progress').count(),0);
  await page.getByRole('button',{name:'选择目录',exact:true}).click();
  await page.getByText('文档 / 羽连',{exact:true}).waitFor();
  assert.equal(await saveFile.isEnabled(),true);
  await saveFile.click();
  assert.equal(await page.locator('.transfer-progress').count(),1);
  assert.ok(await page.evaluate(()=>window.__nativeCalls.some(c=>c[0]==='download')));
  await transferEvent({done:true,message:'已保存 '+longFileName});
  assert.equal(await page.locator('.transfer-progress').count(),0);
  assert.equal(await page.getByRole('button',{name:'选择目录',exact:true}).isEnabled(),true);
  await transferEvent({done:true,folder:'Documents/'.repeat(25),message:'无法保存 '+longFileName});
  await page.setViewportSize({width:320,height:640});
  await page.evaluate(()=>{
    window.__auditFontSizes=[...document.querySelectorAll('.file-folder *,.file-list *,.panel-feedback')].map(e=>[e,e.style.fontSize]);
    for(const [e] of window.__auditFontSizes)e.style.fontSize=`${parseFloat(getComputedStyle(e).fontSize)*1.5}px`;
  });
  assert.ok(await page.locator('.file-folder,.file-list li,.file-panel>.panel-feedback').evaluateAll(elements=>elements.every(e=>e.scrollWidth<=e.clientWidth)));
  assert.ok(await page.getByRole('button',{name:`保存 ${longFileName}`,exact:true}).evaluate(e=>{const r=e.getBoundingClientRect();return r.width>=48&&r.right<=innerWidth;}));
  await snap('files-long-text-font150');
  await page.evaluate(()=>{for(const [e,size] of window.__auditFontSizes)e.style.fontSize=size;delete window.__auditFontSizes;});
  await page.setViewportSize({width:390,height:844});
  await transferEvent({done:true,folder:'文档 / 羽连',message:''});
  await page.unroute('**/api/files');
  await page.getByRole('button',{name:'刷新',exact:true}).click();
  checks.push({files:'long paths, filenames and error messages wrap at 320px with 150 percent text; save controls retain their touch target'});
  let uploads=0,offset=0,size=0;
  const uploadRoute=async route=>{
    const method=route.request().method();
    if(method==='POST'&&new URL(route.request().url()).pathname==='/api/uploads')size=route.request().postDataJSON().size;
    if(method==='PUT'){uploads++;offset+=route.request().postDataBuffer().length;}
    const complete=method==='POST'&&new URL(route.request().url()).pathname!=='/api/uploads';
    await route.fulfill({status:200,contentType:'application/json',body:JSON.stringify({offset,size,chunkBytes:4*1024**2,complete,name:complete?'验证.txt':null})});
  };
  await page.route('**/api/uploads',uploadRoute);await page.route('**/api/uploads/*',uploadRoute);
  await page.evaluate(()=>{window.dispatchEvent(new Event('pagehide'));window.dispatchEvent(new Event('pageshow'));});
  await page.locator('input[type=file]').setInputFiles({name:'验证.txt',mimeType:'text/plain',buffer:Buffer.from('selected in Android picker')});
  await page.getByText('验证.txt 已保存到电脑',{exact:true}).waitFor();assert.equal(uploads,1);
  await page.unroute('**/api/uploads');await page.unroute('**/api/uploads/*');
  await nav('遥控');
  await page.evaluate(()=>window.dispatchEvent(new Event('pagehide')));await page.waitForTimeout(100);
  assert.equal(commands.at(-1).type,'release');
  assert.ok(await page.evaluate(()=>window.__nativeCalls.some(c=>c[0]==='awake'&&c[1]===false)));
  await nav('设置');await page.getByRole('button',{name:/当前电脑/}).click();
  await page.getByRole('button',{name:'忘记此电脑',exact:true}).click();
  await page.locator('#pair-code').waitFor();await snap('pair');
  await page.evaluate(() => {
    window.PlumeLinkAndroid.scanPairingCode = () => window.__nativeCalls.push(['scan']);
    window.PlumeLinkAndroid.takePairingCode = () => { const code = sessionStorage.getItem('scanned-code') || ''; sessionStorage.removeItem('scanned-code'); return code; };
    sessionStorage.setItem('scanned-code', '012345');
  });
  const pairRequests=[];
  await page.route('**/api/pair', async route => {
    pairRequests.push(route.request().postDataJSON());
    await route.fulfill({status:200,contentType:'application/json',body:JSON.stringify({error:'二维码已更新，请重新扫码'})});
  });
  await page.reload();
  await page.locator('.pair-status').filter({hasText:'电脑返回了无效凭证，请重试'}).waitFor();
  assert.equal(pairRequests.length,1);assert.equal(pairRequests[0].code,'012345');
  assert.equal(await page.locator('#pair-code').inputValue(),'012345');
  await page.getByRole('button',{name:'扫描电脑二维码',exact:true}).click();
  assert.ok(await page.evaluate(()=>window.__nativeCalls.some(c=>c[0]==='scan')));
  for(const [width,height] of [[320,550],[390,844],[844,390]]) {
    await page.setViewportSize({width,height});
    await page.getByRole('button',{name:'配对连接',exact:true}).scrollIntoViewIfNeeded();
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await snap(`pair-scan-${width}`);
  }
  await page.reload();
  await page.waitForTimeout(300);
  assert.equal(pairRequests.length,1,'a consumed QR must not auto-submit on another page load');
  await page.unroute('**/api/pair');
  await page.route('**/api/pair',async route=>{
    pairRequests.push(route.request().postDataJSON());
    await route.fulfill({status:200,contentType:'application/json',body:JSON.stringify({token:existingToken})});
  });
  await page.evaluate(()=>sessionStorage.setItem('scanned-code','000012'));
  await page.reload();
  await page.getByRole('button',{name:/^已连接/}).waitFor();
  assert.equal(pairRequests.length,2);assert.equal(pairRequests[1].code,'000012');
  assert.equal(await page.locator('dialog[open]').count(),0,'successful QR handoff closes pairing and starts the real QFT session');
  await page.unroute('**/api/pair');
  checks.push({pairing:'one-shot scanned code preserves leading zeros, exposes rescan, and never retries a rejected QR automatically'});
  await page.setViewportSize({width:320,height:550});
  await nav("设置");
  for (const [name, language, heading] of [["English","en","Settings"],["繁體中文","zh-TW","設定"],["简体中文","zh-CN","设置"]]) {
    await page.locator("#language").click();
    await page.getByRole("radio",{name,exact:true}).click();
    await page.getByRole("heading",{name:heading,exact:true}).first().waitFor();
    assert.equal(await page.locator("html").getAttribute("lang"), language);
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  }
  checks.push({locales:"switched through all three languages at 320px while connected"});
  await writeFile(resolve(out,'results.json'),JSON.stringify({passed:true,checks,transport:'real QFT2/WSS with fake Native',commandCount:commands.length,frames},null,2));
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const child=spawnSync(process.execPath,['scripts/verify-qft.mjs','--ui'],{stdio:'inherit',windowsHide:true});process.exitCode=child.status||0;
}
