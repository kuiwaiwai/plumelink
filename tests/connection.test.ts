import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { isReadonly } from "vue";
import { languagePreference } from "../shared/i18n";
import { readFileSync } from "node:fs";
import { classifyConnectionQuality, useConnection } from "../apps/mobile/connection";
import { loadQftKernel, type Kernel } from "../apps/mobile/qft-transport";
import { supportsQft } from "../apps/mobile/qft-transport";
vi.mock("../apps/mobile/qft-transport", async original => ({ ...await original<typeof import("../apps/mobile/qft-transport")>(), loadQftKernel: vi.fn() }));
const module = new WebAssembly.Module(readFileSync("apps/mobile/generated/qft.wasm"));
const newKernel = () => new WebAssembly.Instance(module, {}).exports as unknown as Kernel;
const capability = { version: 2, frameBytes: 64, session: "7", leaseMs: 5000, motionTtlMs: 120, actionTtlMs: 200, maxCatchup: 16384, adapter: "logical-pointer-v1" };
function grant() { const b=new ArrayBuffer(64),v=new DataView(b);v.setUint32(0,0x51465432);v.setUint8(4,1);v.setBigUint64(8,7n,true);v.setUint32(16,1,true);v.setUint16(42,5000,true);return b; }
class Socket {
  static OPEN=1;static CLOSING=2;static sockets:Socket[]=[];
  readyState=0;bufferedAmount=0;frames:any[]=[];
  onopen=()=>{};onmessage=(event:{data:string|ArrayBuffer})=>{};onclose=(event:{code:number;reason?:string})=>{};onerror=()=>{};
  constructor(readonly url=""){Socket.sockets.push(this);}
  send(frame:string|ArrayBuffer){this.frames.push(typeof frame === "string" ? JSON.parse(frame):frame);}
  ready(extra:Record<string,unknown>={}){this.readyState=1;this.onopen();this.json({kind:"ready",qft:capability,permissions:{input:true,text:true,files:true,power:false,microphone:false},...extra});}
  async open(extra:Record<string,unknown>={}){this.ready(extra);await vi.advanceTimersByTimeAsync(0);if(this.frames.some(f=>f.kind==="qftOpen"))this.onmessage({data:grant()});}
  json(frame:unknown){this.onmessage({data:JSON.stringify(frame)});}
  close(code=1000,reason=""){this.readyState=3;this.onclose({code,reason});}
}
beforeEach(()=>{
  vi.useFakeTimers({toFake:["setTimeout","clearTimeout","setInterval","clearInterval","Date","performance"]});
  vi.mocked(loadQftKernel).mockReset().mockImplementation(async()=>newKernel()); Socket.sockets=[];
  vi.stubGlobal("WebSocket",Socket);vi.stubGlobal("document",{hidden:false});vi.stubGlobal("location",{protocol:"https:",host:"192.168.1.2:19827"});
  const values=new Map([["plumelink-token","a".repeat(64)]]);
  vi.stubGlobal("localStorage",{getItem:(k:string)=>values.get(k),setItem:(k:string,v:string)=>values.set(k,v),removeItem:(k:string)=>values.delete(k)});
});
afterEach(()=>{languagePreference.value="system";vi.clearAllTimers();vi.useRealTimers();vi.unstubAllGlobals();});
it("reports the Android name after authentication without delaying control",async()=>{
  vi.stubGlobal("PlumeLinkAndroid",{getDeviceName:()=>"我的手机",getDeviceModel:()=>"2206122SC"});
  const post=vi.fn(async()=>new Response(JSON.stringify({deviceName:"PC alias"}),{headers:{"Content-Type":"application/json"}}));
  vi.stubGlobal("fetch",post);
  const remote=useConnection();remote.resume();await Socket.sockets[0].open();
  await vi.advanceTimersByTimeAsync(0);
  expect(post).toHaveBeenCalledTimes(1);
  const [path,init]=post.mock.calls[0] as unknown as [string,RequestInit];
  expect(path).toBe("/api/device/name");
  expect(JSON.parse(init.body as string)).toEqual({name:"我的手机",model:"2206122SC"});
  expect((init.headers as Headers).get("Authorization")).toBe("Bearer "+"a".repeat(64));
  expect(remote.deviceName.value).toBe("PC alias");expect(remote.status.value).toBe("connected");remote.suspend();
});
it("a failed optional name report leaves pairing and control usable",async()=>{
  vi.stubGlobal("PlumeLinkAndroid",{getDeviceName:()=>"My phone"});
  vi.stubGlobal("fetch",vi.fn().mockRejectedValue(new TypeError("offline")));
  const remote=useConnection();remote.resume();await Socket.sockets[0].open();
  expect(remote.status.value).toBe("connected");expect(remote.token()).toBe("a".repeat(64));expect(remote.message.value).not.toContain("offline");remote.suspend();
});
it("requires the WASM lease before input and uses QFT2 exclusively",async()=>{
  const r=useConnection();r.resume();const ws=Socket.sockets[0];ws.ready();
  await expect(r.send({type:"key",key:"enter"})).rejects.toThrow("尚未连接");
  await vi.advanceTimersByTimeAsync(0);expect(ws.frames.at(-1)).toEqual({kind:"qftOpen"});ws.onmessage({data:grant()});
  r.sendMotion({dx:2,dy:1,scroll:0});expect(new DataView(ws.frames.at(-1)).getUint32(0)).toBe(0x51465432);
  expect(r.qft2.value).toBe(true);r.suspend();
});
it.each([undefined,{version:1}])("refuses an incompatible server without fallback: %j",async qft=>{
  const r=useConnection();r.resume();const ws=Socket.sockets[0];ws.ready({qft});
  expect(ws.readyState).toBe(3);expect(r.message.value).toContain("版本不匹配");
  await vi.advanceTimersByTimeAsync(10000);expect(Socket.sockets).toHaveLength(1);r.suspend();
});
it("bounds loading and ignores a late loader after failure",async()=>{
  let finish!:(k:Kernel)=>void;vi.mocked(loadQftKernel).mockReturnValue(new Promise(r=>finish=r));
  const r=useConnection();r.resume();const ws=Socket.sockets[0];ws.ready();await vi.advanceTimersByTimeAsync(3500);
  expect(r.message.value).toContain("传输组件");finish(newKernel());await vi.advanceTimersByTimeAsync(0);
  expect(ws.frames.some(f=>f.kind==="qftOpen")).toBe(false);r.suspend();
});
it("a rejected WASM loader stops input with an actionable error",async()=>{
  vi.mocked(loadQftKernel).mockRejectedValue(new Error("compile failed"));const r=useConnection();r.resume();Socket.sockets[0].ready();
  await vi.advanceTimersByTimeAsync(0);expect(r.status.value).toBe("offline");expect(r.message.value).toContain("WebView");r.suspend();
});
it("a late old loader cannot clear the current connection timeout",async()=>{
  let finish!:(k:Kernel)=>void;vi.mocked(loadQftKernel).mockReturnValueOnce(new Promise(r=>finish=r)).mockReturnValueOnce(new Promise(()=>{}));
  const r=useConnection();r.resume();Socket.sockets[0].ready();r.suspend();r.resume();Socket.sockets[1].ready();
  finish(newKernel());await vi.advanceTimersByTimeAsync(3500);expect(Socket.sockets[1].readyState).toBe(3);r.suspend();
});
it("changing response mode preserves the live socket, grant and QFT lease",async()=>{
  const r=useConnection();r.resume();const ws=Socket.sockets[0];await ws.open();
  ws.json({kind:"pong",presence:{adminGranted:true}});expect(r.adminGranted.value).toBe(true);expect(r.qft2.value).toBe(true);
  expect(ws.frames.some(f=>f.kind==="qftOff")).toBe(false);
  r.setResponseMode("responsive");
  expect(Socket.sockets).toHaveLength(1);expect(ws.readyState).toBe(1);
  expect(r.adminGranted.value).toBe(true);
  r.sendMotion({dx:1,dy:0,scroll:0});
  expect(new DataView(ws.frames.at(-1)).getUint32(16,true)).toBe(1);
  r.suspend();
});
it("ignores unsupported response modes", () => {
  const r = useConnection();
  r.setResponseMode("turbo");
  r.resume();
  expect(Socket.sockets).toHaveLength(1);
  r.suspend();
});
it("suspend cancels the kernel, rejects pending action, and never replays",async()=>{
  const r=useConnection();r.resume();const ws=Socket.sockets[0];await ws.open();
  const done=r.send({type:"click",button:"left",count:1}).catch(e=>e.message);r.suspend();
  expect(await done).toContain("不会自动重发");expect(new DataView(ws.frames.at(-1)).getUint8(4)).toBe(7);
  r.resume();await Socket.sockets[1].open();expect(Socket.sockets[1].frames.filter(f=>f.kind==="qftStage")).toHaveLength(0);r.suspend();
});
it("background handoff cancels input without a WebView reconnect and never replays on return",async()=>{
  const remote=useConnection();remote.resume();const first=Socket.sockets[0];await first.open();
  const pending=remote.send({type:"click",button:"left",count:1}).catch(error=>error.message);remote.releaseInput();
  expect(await pending).toContain("不会自动重发");expect(new DataView(first.frames.at(-1)).getUint8(4)).toBe(7);
  vi.stubGlobal("document",{hidden:true});await vi.advanceTimersByTimeAsync(120000);expect(Socket.sockets).toHaveLength(1);
  vi.stubGlobal("document",{hidden:false});remote.resume();const second=Socket.sockets[1];await second.open();
  expect(second.frames.some(frame=>frame.kind==="qftStage")).toBe(false);expect(remote.token()).toBe("a".repeat(64));remote.suspend();
});
it("waits for the native foreground owner before opening an input session",async()=>{
  let foreground=false;
  vi.stubGlobal("PlumeLinkAndroid",{isControlForegroundReady:()=>foreground});
  const remote=useConnection();remote.resume();expect(Socket.sockets).toHaveLength(0);
  foreground=true;remote.resume();await Socket.sockets[0].open();expect(remote.status.value).toBe("connected");remote.suspend();
});
it("processes a background credential revocation before reconnecting",()=>{
  const setStoredToken=vi.fn();
  vi.stubGlobal("PlumeLinkAndroid",{getConnectionKeepAliveState:()=>JSON.stringify({terminalCode:4003}),setStoredToken});
  const remote=useConnection();remote.resume();expect(Socket.sockets).toHaveLength(0);
  expect(remote.token()).toBe("");expect(setStoredToken).toHaveBeenCalledWith("");expect(remote.message.value).toContain("配对码");remote.suspend();
});
it("keeps pairing but blocks an expired background USB lease before reconnecting",()=>{
  vi.stubGlobal("location",{protocol:"https:",host:"127.0.0.1:17840",hostname:"127.0.0.1"});
  vi.stubGlobal("PlumeLinkAndroid",{getConnectionKeepAliveState:()=>JSON.stringify({terminalCode:4409})});
  const remote=useConnection();remote.resume();expect(Socket.sockets).toHaveLength(0);expect(remote.usbExpired.value).toBe(true);
  expect(remote.token()).toBe("a".repeat(64));remote.resume();expect(Socket.sockets).toHaveLength(0);remote.suspend();
});
it("duplicate ready and malformed evidence close the connection",async()=>{
  const r=useConnection();r.resume();const ws=Socket.sockets[0];await ws.open();ws.ready();expect(ws.readyState).toBe(3);
  r.resume();await Socket.sockets[1].open();Socket.sockets[1].onmessage({data:new ArrayBuffer(20)});expect(Socket.sockets[1].readyState).toBe(3);r.suspend();
});
it("old close and ready events cannot clear the current session",async()=>{
  const r=useConnection();r.resume();const first=Socket.sockets[0];await first.open();first.close=()=>{first.readyState=2;};
  r.suspend();r.resume();await Socket.sockets[1].open();first.onclose({code:4003});first.json({kind:"ready"});
  expect(r.status.value).toBe("connected");expect(r.token()).toBe("a".repeat(64));r.suspend();
});
it("replacing a closed socket before its close event releases the old heartbeat",async()=>{
  const r=useConnection();r.resume();const first=Socket.sockets[0];await first.open();
  first.readyState=3;r.resume();const second=Socket.sockets[1];await second.open();
  first.onclose({code:1006});await vi.advanceTimersByTimeAsync(1000);
  expect(second.frames.some(frame=>frame.kind==="ping")).toBe(true);
  r.suspend();expect(vi.getTimerCount()).toBe(0);
});
it("network loss reconnects without replaying any action",async()=>{
  const r=useConnection();r.resume();await Socket.sockets[0].open();Socket.sockets[0].close(1006);
  await vi.advanceTimersByTimeAsync(750);await Socket.sockets[1].open({permissions:{input:true,text:true,files:true,power:false,microphone:false}});expect(r.status.value).toBe("connected");
  expect(Socket.sockets[1].frames).toEqual([{kind:"auth",token:"a".repeat(64)},{kind:"qftOpen"}]);r.suspend();
});
it("PC resume keeps pairing and reconnects without replaying a pending action",async()=>{
  const remote=useConnection();remote.resume();const old=Socket.sockets[0];await old.open();
  const pending=remote.send({type:"key",key:"enter"}).catch(error=>error.message);
  old.close(4004);
  expect(await pending).toContain("不会自动重发");expect(remote.token()).toBe("a".repeat(64));
  await vi.advanceTimersByTimeAsync(750);const fresh=Socket.sockets[1];await fresh.open();
  expect(remote.status.value).toBe("connected");
  expect(fresh.frames).toEqual([{kind:"auth",token:"a".repeat(64)},{kind:"qftOpen"}]);remote.suspend();
});
it("permission and primary presence are updated without downgrading",async()=>{
  const r=useConnection();r.resume();await Socket.sockets[0].open({permissions:{input:true,text:false,files:true,power:false}});
  Socket.sockets[0].json({kind:"pong",presence:{controlMode:"primary",primary:false,primaryName:"客厅手机",connectedCount:2}});
  expect(r.permissions.value.text).toBe(false);expect(r.primaryName.value).toBe("客厅手机");expect(r.connectedCount.value).toBe(2);r.suspend();
});
it("fails closed when a primary presence update omits the grant bit", async () => {
  const r = useConnection(); r.resume(); const ws = Socket.sockets[0]; await ws.open({ primary: true, controlMode: "primary" });
  expect(r.primary.value).toBe(true);
  ws.json({ kind: "pong", presence: { controlMode: "primary", primaryName: "另一台设备" } });
  expect(r.controlMode.value).toBe("primary");
  expect(r.primary.value).toBe(false);
  r.suspend();
});
it("fails closed when a presence update omits control mode", async () => {
  const r = useConnection(); r.resume(); const ws = Socket.sockets[0]; await ws.open({ primary: true, controlMode: "primary" });
  expect(r.primary.value).toBe(true);
  ws.json({ kind: "pong", presence: { primaryName: "未知设备" } });
  expect(r.controlMode.value).toBe("free");
  expect(r.primary.value).toBe(false);
  r.suspend();
});
it("fails closed when missing control mode contradicts the primary grant", async () => {
  const r = useConnection(); r.resume(); const ws = Socket.sockets[0];
  await ws.open({ primary: true, controlMode: "primary" });
  expect(r.primary.value).toBe(true);
  ws.json({ kind: "pong", presence: { primary: true, primaryName: "异常设备" } });
  expect(r.controlMode.value).toBe("free");
  expect(r.primary.value).toBe(false);
  r.suspend();
});
it("fails closed when free mode contradicts the primary grant", async () => {
  const r = useConnection(); r.resume(); const ws = Socket.sockets[0];
  await ws.open({ primary: true, controlMode: "primary" });
  expect(r.primary.value).toBe(true);
  ws.json({ kind: "pong", presence: { controlMode: "free", primary: true } });
  expect(r.controlMode.value).toBe("free");
  expect(r.primary.value).toBe(false);
  r.suspend();
});
it("bounds malformed presence labels and device counts", async () => {
  const r = useConnection(); r.resume(); const ws = Socket.sockets[0];
  await ws.open({ permissions: { input: true, text: true, files: true, power: false, microphone: false }, deviceName: "坏\u0001设备" });
  ws.json({ kind: "pong", presence: {
    connectedCount: 999999,
    primaryName: "坏\u0000名称",
    recentController: "x".repeat(81),
  } });
  expect(r.connectedCount.value).toBe(32);
  expect(r.primaryName.value).toBe("");
  expect(r.recentController.value).toBe("");
  expect(r.deviceName.value).toBe("");
  r.suspend();
});
it("fails closed when the ready frame grants primary without primary mode", async () => {
  const r = useConnection(); r.resume(); const ws = Socket.sockets[0];
  await ws.open({ primary: true, controlMode: "free" });
  expect(r.controlMode.value).toBe("free");
  expect(r.primary.value).toBe(false);
  r.suspend();
});
it("ignores malformed ready metadata without granting permissions", async () => {
  const r = useConnection(); r.resume();
  await Socket.sockets[0].open({ permissions: { input: "yes", text: 1, files: true, power: false }, primary: "yes", deviceName: 42 });
  expect(r.permissions.value.input).toBe(false);
  expect(r.permissions.value.text).toBe(false);
  expect(r.permissions.value.files).toBe(true);
  expect(r.permissions.value.power).toBe(false);
  expect(r.deviceName.value).toBe("");
  expect(r.primary.value).toBe(false);
  expect(r.controlMode.value).toBe("free");
  r.suspend();
});
it("file-only devices connect without opening an input kernel",async()=>{
  const r=useConnection();r.resume();await Socket.sockets[0].open({permissions:{input:false,text:false,files:true,power:false}});
  expect(r.status.value).toBe("connected");expect(r.qft2.value).toBe(false);
  await expect(r.send({type:"key",key:"enter"})).rejects.toThrow("尚未连接");r.suspend();
});
it.each(["text", "power"])("%s-only devices open QFT2 for their allowed actions",async permission=>{
  const r=useConnection();r.resume();await Socket.sockets[0].open({permissions:{input:false,text:permission==="text",files:true,power:permission==="power"}});
  expect(r.status.value).toBe("connected");expect(r.qft2.value).toBe(true);r.suspend();
});

it("forgetting credentials prevents a delayed pairing response from reconnecting", async () => {
  let finish!: (value: unknown) => void;
  vi.stubGlobal("fetch", vi.fn(() => new Promise(resolve => { finish = resolve; })));
  const remote = useConnection();
  const pairing = remote.pair("123456");
  remote.forget();
  finish({ ok: true, json: async () => ({ token: "b".repeat(64) }) });
  await pairing;
  expect(remote.token()).toBe("");
  expect(Socket.sockets).toHaveLength(0);
});
it("does not erase fresh credentials when an old API request returns unauthorized", async () => {
  let finish!: (value: unknown) => void;
  vi.stubGlobal("fetch", vi.fn(() => new Promise(resolve => { finish = resolve; })));
  const remote = useConnection();
  const request = remote.api("/api/files").catch(error => error.message);
  remote.suspend();
  remote.resume();
  finish({ status: 401 });
  expect(await request).toContain("连接已变化");
  expect(remote.token()).toBe("a".repeat(64));
  remote.suspend();
});
it("times out stalled API requests without changing credentials", async () => {
  vi.stubGlobal("fetch", vi.fn((_url: string, init: RequestInit) => new Promise((_resolve, reject) => {
    init.signal?.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")), { once: true });
  })));
  const remote = useConnection();
  const request = remote.api("/api/files").catch(error => error);
  await vi.advanceTimersByTimeAsync(12_000);
  await expect(request).resolves.toMatchObject({ message: "请求电脑超时，请检查连接后重试" });
  expect(remote.token()).toBe("a".repeat(64));
  remote.suspend();
});
it("native credentials remain authoritative when browser storage is stale or denied", () => {
  vi.stubGlobal("PlumeLinkAndroid", { getStoredToken: () => "" });
  const remote = useConnection();
  remote.resume();
  expect(Socket.sockets).toHaveLength(0);
  const save = vi.fn();
  vi.stubGlobal("PlumeLinkAndroid", { getStoredToken: () => "b".repeat(64), setStoredToken: save });
  vi.stubGlobal("localStorage", { getItem: () => { throw new Error("denied"); }, removeItem: () => { throw new Error("denied"); } });
  const native = useConnection();
  native.forget();
  expect(save).toHaveBeenCalledWith("");
});
it("pairing rejects incomplete codes and prevents duplicate requests, preserving leading zero", async () => {
  let finish!: (value: unknown) => void;
  const fetcher = vi.fn((_url: string, _options: { body: string }) => new Promise(resolve => { finish = resolve; }));
  vi.stubGlobal("fetch", fetcher);
  const remote = useConnection();
  await remote.pair("123");
  expect(fetcher).not.toHaveBeenCalled();
  const request = remote.pair("012345");
  await remote.pair("012345");
  expect(fetcher).toHaveBeenCalledTimes(1);
  expect(JSON.parse(fetcher.mock.calls[0][1].body).code).toBe("012345");
  finish({ ok: false, json: async () => ({ error: "配对码不正确，请核对电脑上显示的 6 位数字" }) });
  await request;
  expect(remote.message.value).toContain("配对码不正确");
});
it("pairing network failure does not claim credentials expired", async () => {
  vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
  const remote = useConnection();
  await remote.pair("123456");
  expect(remote.message.value).toContain("网络连接失败");
  expect(remote.message.value).not.toContain("过期");
});
it.each([[null,null,"unknown"],[80,null,"good"],[120,30,"good"],[121,30,"fair"],[300,100,"fair"],[301,10,"poor"],[100,31,"fair"],[100,101,"poor"]] as const)("classifies %s ms and %s jitter",(latency,jitter,quality)=>{expect(classifyConnectionQuality(latency,jitter)).toBe(quality);});
it.each([[NaN, 0], [Infinity, 0], [-1, 0]] as const)("rejects invalid latency %s", (latency, jitter) => {
  expect(classifyConnectionQuality(latency, jitter)).toBe("unknown");
});
it("rejects malformed QFT session values without throwing", () => {
  expect(() => supportsQft({ version: 2, frameBytes: 64, session: "not-a-number", leaseMs: 5000, motionTtlMs: 120, actionTtlMs: 200, maxCatchup: 16384, adapter: "logical-pointer-v1" })).not.toThrow();
  expect(supportsQft({ version: 2, frameBytes: 64, session: "not-a-number", leaseMs: 5000, motionTtlMs: 120, actionTtlMs: 200, maxCatchup: 16384, adapter: "logical-pointer-v1" })).toBe(false);
});
it.each([4001,4003])("takeover/revocation does not enter reconnect loops: %s",async code=>{const r=useConnection();r.resume();await Socket.sockets[0].open();Socket.sockets[0].close(code);await vi.advanceTimersByTimeAsync(10000);expect(Socket.sockets).toHaveLength(1);if(code===4003)expect(r.token()).toBe("");r.suspend();});
it("heartbeat records latency and clears stale quality",async()=>{const r=useConnection();r.resume();const ws=Socket.sockets[0];await ws.open();await vi.advanceTimersByTimeAsync(1000);await vi.advanceTimersByTimeAsync(180);ws.json({kind:"pong"});expect(r.latency.value).toBe(180);ws.close(1006);expect(r.latency.value).toBeNull();r.suspend();});
it("clears connection permissions and control state after disconnect", async () => {
  const r = useConnection(); r.resume(); const ws = Socket.sockets[0];
  await ws.open({ permissions: { input: true, text: true, files: true, power: true, microphone: true }, primary: true, controlMode: "primary" });
  ws.close(1006);
  expect(r.permissions.value).toEqual({ input: false, text: false, files: false, power: false, microphone: false });
  expect(r.primary.value).toBe(false);
  expect(r.controlMode.value).toBe("free");
  expect(r.connectedCount.value).toBe(0);
  r.suspend();
});
it("keeps invalid latency samples out of connection quality", async () => {
  const r = useConnection(); r.resume(); const ws = Socket.sockets[0]; await ws.open();
  await vi.advanceTimersByTimeAsync(1000);
  await vi.advanceTimersByTimeAsync(61_000);
  ws.json({ kind: "pong" });
  expect(r.latency.value).toBeNull();
  expect(r.quality.value).toBe("unknown");
  r.suspend();
});
it("treats mid-session error followed by close as retrying instead of failed",async()=>{const r=useConnection();r.resume();const ws=Socket.sockets[0];await ws.open();ws.onerror?.();ws.close(1006);expect(r.connectionLabel.value).toBe("连接已断开，正在重连…");expect(r.message.value).toContain("连接已断开");r.suspend();});
it("preserves custom 4000 close reasons and resets failed state on resume",async()=>{const r=useConnection();r.resume();const ws=Socket.sockets[0];ws.onerror?.();ws.close(4000,"主机正在关闭");expect(r.message.value).toBe("主机正在关闭");expect(r.connectionLabel.value).toBe("连接失败，点击重试");r.resume();expect(r.connectionLabel.value).toBe("正在连接…");r.suspend();});
it("transitions failed state to retrying on scheduled reconnect and reacts to online events",async()=>{
  const listeners=new Map<string,Array<()=>void>>();
  vi.stubGlobal("window",{addEventListener:(n:string,fn:()=>void)=>listeners.set(n,[...(listeners.get(n)||[]),fn]),removeEventListener:(n:string,fn:()=>void)=>listeners.set(n,(listeners.get(n)||[]).filter(f=>f!==fn))});
  const r=useConnection();r.resume();
  Socket.sockets[0].onerror?.();Socket.sockets[0].close(1006);
  expect(r.connectionLabel.value).toBe("连接失败，点击重试");
  await vi.advanceTimersByTimeAsync(750);
  expect(Socket.sockets).toHaveLength(2);
  Socket.sockets[1].close(1006);
  expect(r.connectionLabel.value).toBe("连接已断开，正在重连…");
  listeners.get("online")?.forEach(fn=>fn());
  expect(Socket.sockets).toHaveLength(3);
  r.suspend();
  expect(listeners.get("online")).toHaveLength(0);
});

it("keeps an accepted control request pending until a fresh heartbeat confirms resolution", async () => {
  const r = useConnection();
  const post = vi.fn().mockResolvedValue(new Response(null, { status: 202 }));
  vi.stubGlobal("fetch", post);
  r.resume();
  const ws = Socket.sockets[0];
  const presence = { controlMode: "primary", primary: false, controlRequestPending: false };
  await ws.open({ presence, permissions: { input: true } });
  await vi.advanceTimersByTimeAsync(1050);
  await r.requestControl();
  await r.requestControl();
  expect(post).toHaveBeenCalledTimes(1);
  ws.json({ kind: "pong", presence });
  expect(r.controlRequestPending.value).toBe(true);
  await vi.advanceTimersByTimeAsync(1000);
  ws.json({ kind: "pong", presence: { ...presence, controlRequestPending: true } });
  expect(r.controlRequestPending.value).toBe(true);
  await vi.advanceTimersByTimeAsync(1000);
  ws.json({ kind: "pong", presence });
  expect(r.controlRequestPending.value).toBe(false);
  await r.requestControl();
  r.suspend();
  expect(r.controlRequestPending.value).toBe(false);
});

it("expires an unanswered request on a PC that omits the pending field", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(null, { status: 202 })));
  const r = useConnection();
  r.resume();
  const ws = Socket.sockets[0];
  const presence = { controlMode: "primary", primary: false };
  await ws.open({ presence, permissions: { files: true, input: true } });
  const timers = vi.spyOn(globalThis, "setTimeout");
  try {
    await r.requestControl();
    await vi.advanceTimersByTimeAsync(1000);
    ws.json({ kind: "pong", presence });
    expect(r.controlRequestPending.value).toBe(true);
    const expiry = timers.mock.calls.find(([, delay]) => delay === 60_000)?.[0];
    expect(typeof expiry).toBe("function");
    if (typeof expiry === "function") expiry();
    expect(r.controlRequestPending.value).toBe(false);
    expect(r.status.value).toBe("connected");
  } finally {
    timers.mockRestore();
  }
  r.suspend();
});

it("ignores a control request acknowledgement after disconnect", async () => {
  let accept!: (value: Response) => void;
  vi.stubGlobal("fetch", () => new Promise<Response>(resolve => { accept = resolve; }));
  const r = useConnection();
  r.resume();
  await Socket.sockets[0].open({ presence: { controlMode: "primary", primary: false }, permissions: { input: true } });
  const pending = r.requestControl();
  expect(r.controlRequestPending.value).toBe(true);
  r.suspend();
  accept(new Response(null, { status: 202 }));
  await pending;
  expect(r.controlRequestPending.value).toBe(false);
});

it("keeps the window-list deadline active until the JSON body finishes", async () => {
  const r = useConnection();
  let aborted = false;
  vi.stubGlobal("fetch", vi.fn(async (_url, init: RequestInit) => ({
    ok: true, status: 200,
    json: () => new Promise((_resolve, reject) => init.signal!.addEventListener("abort", () => {
      aborted = true; reject(new Error("aborted"));
    })),
  })));
  const result = r.listWindows().catch(e => e.message);
  await vi.advanceTimersByTimeAsync(12_001);
  expect(aborted).toBe(true);
  expect(await result).toContain("超时");
});

it("normalizes window metadata and images without silently cutting the list", async () => {
  const windows = Array.from({ length: 160 }, (_, i) => ({ hwnd: i + 1, title: "Window " + i, app: "sample.exe", minimized: false }));
  const get = vi.fn(async () => new Response(JSON.stringify({ windows: [
    ...windows,
    { hwnd: 1, title: "Duplicate" },
    { hwnd: 0, title: "Invalid" },
    { hwnd: 1.5, title: "Invalid" },
    { hwnd: Number.MAX_SAFE_INTEGER + 1, title: "Invalid" },
    { hwnd: 500, title: "" },
    { hwnd: 501, title: "x".repeat(2049) },
    { hwnd: 502, title: "资料 📁", app: { path: "private" }, minimized: "false", icon: "https://example.com/tracker.png", extra: "discard" },
    { hwnd: 503, title: "Unsupported image", app: "x".repeat(513), icon: "data:image/svg+xml,<svg/>", minimized: true },
  ] })));
  vi.stubGlobal("fetch", get);
  const r = useConnection();
  const entries = await r.listWindows();
  expect(entries).toHaveLength(162);
  expect(entries[159]).toEqual({ ...windows[159], icon: null });
  expect(entries[160]).toEqual({ hwnd: 502, title: "资料 📁", app: "", minimized: false, icon: null });
  expect(entries[161]).toEqual({ hwnd: 503, title: "Unsupported image", app: "", minimized: true, icon: null });
  expect(get).toHaveBeenCalledTimes(1);
  r.suspend();
});

it("keeps the API timeout active when headers arrive but the body stalls", async () => {
  vi.stubGlobal("fetch", vi.fn(async (_url: string, init: RequestInit) =>
    new Response(new ReadableStream({
      start(controller) {
        init.signal?.addEventListener("abort", () => controller.error(new DOMException("Aborted", "AbortError")), { once: true });
      },
    }), { status: 200 })));
  const remote = useConnection();
  const pending = remote.api("/api/files").catch(error => error);
  await vi.advanceTimersByTimeAsync(12_000);
  await expect(pending).resolves.toMatchObject({ message: "请求电脑超时，请检查连接后重试" });
  expect(remote.token()).toBe("a".repeat(64));
  remote.suspend();
});

it("preserves healthy LAN connections and reconnects once after confirmed loss", async () => {
  const events = new EventTarget();
  vi.stubGlobal("window", events);
  const network = { onLine: true };
  vi.stubGlobal("navigator", network);
  const remote = useConnection();
  remote.resume();
  const old = Socket.sockets[0];
  await old.open();
  network.onLine = false;
  events.dispatchEvent(new Event("offline"));
  expect(remote.status.value).toBe("connected");
  expect(old.readyState).toBe(Socket.OPEN);
  old.close(1006);
  expect(remote.status.value).toBe("offline");
  expect(remote.permissions.value.input).toBe(false);
  expect(remote.connectionBadgeLabel.value).toBe("无网络");
  await expect(remote.send({ type: "key", key: "enter" })).rejects.toThrow("尚未连接");
  await vi.advanceTimersByTimeAsync(4999);
  expect(Socket.sockets).toHaveLength(1);
  network.onLine = true;
  events.dispatchEvent(new Event("online"));
  events.dispatchEvent(new Event("online"));
  expect(Socket.sockets).toHaveLength(2);
  await Socket.sockets[1].open();
  expect(remote.status.value).toBe("connected");
  remote.suspend();
});
it("attempts a LAN connection despite offline hints and rereads the network after suspension", () => {
  const network = { onLine: false };
  vi.stubGlobal("navigator", network);
  const remote = useConnection();
  remote.resume();
  expect(Socket.sockets).toHaveLength(1);
  remote.suspend();
  network.onLine = true;
  remote.resume();
  expect(Socket.sockets).toHaveLength(2);
  remote.suspend();
});
it("keeps USB independent of Wi-Fi hints and retries a broken tunnel without replaying commands", async () => {
  vi.stubGlobal("location",{protocol:"https:",host:"127.0.0.1:49821",hostname:"127.0.0.1"});
  vi.stubGlobal("navigator",{onLine:false});const events=new EventTarget();vi.stubGlobal("window",events);
  const remote=useConnection();remote.resume();const first=Socket.sockets[0];await first.open();
  events.dispatchEvent(new Event("offline"));expect(remote.status.value).toBe("connected");first.close(1006);
  expect(remote.connectionBadgeLabel.value).not.toBe("无网络");await vi.advanceTimersByTimeAsync(700);
  expect(Socket.sockets).toHaveLength(2);await Socket.sockets[1].open();expect(remote.status.value).toBe("connected");
  expect(Socket.sockets[1].frames.filter(f=>f.kind==="qftStage")).toEqual([]);remote.suspend();
});
it("recovers from a throwing WebSocket constructor without losing credentials", async () => {
  vi.stubGlobal("WebSocket", class { constructor() { throw new Error("blocked"); } });
  const remote = useConnection();
  expect(() => remote.resume()).not.toThrow();
  expect(remote.status.value).toBe("offline");
  expect(remote.connectionBadgeLabel.value).toBe("连接失败");
  vi.stubGlobal("WebSocket", Socket);
  await vi.advanceTimersByTimeAsync(700);
  expect(Socket.sockets).toHaveLength(1);
  expect(remote.token()).toBe("a".repeat(64));
  remote.suspend();
});

it("does not inherit permissions when a new ready frame omits them", async () => {
  const remote = useConnection();
  remote.resume();
  Socket.sockets[0].ready({ permissions: undefined });
  expect(remote.permissions.value).toEqual({ input: false, text: false, files: false, power: false, microphone: false });
  expect(loadQftKernel).not.toHaveBeenCalled();
  await expect(remote.send({ type: "key", key: "enter" })).rejects.toThrow("尚未连接");
  remote.suspend();
});

it("gates new media commands without disrupting an older PC connection", async () => {
  const remote = useConnection(); remote.resume(); const ws = Socket.sockets[0]; await ws.open();
  const before = ws.frames.length;
  await expect(remote.send({ type: "media", action: "next" })).rejects.toThrow("请更新电脑端");
  expect(ws.frames).toHaveLength(before);
  expect(remote.status.value).toBe("connected");
  remote.suspend();
});
it("accepts advertised media controls and forgets capabilities on disconnect", async () => {
  const remote = useConnection(); remote.resume(); const ws = Socket.sockets[0];
  await ws.open({ features: ["unknown", "media", "media", "custom-keys", "custom-keys"] });
  expect(remote.features.value).toEqual(["media", "custom-keys"]);
  const result = remote.send({ type: "media", action: "playpause" }).catch(() => {});
  expect(ws.frames.some(f => f.kind === "qftStage" && f.command?.type === "media")).toBe(true);
  remote.suspend(); await result;
  expect(remote.features.value).toEqual([]);
  remote.resume(); await Socket.sockets[1].open({ features: { media: true } });
  expect(remote.features.value).toEqual([]);
  remote.suspend();
});
it("uses an immutable native USB credential and preserves pairing when the endpoint expires", async () => {
  vi.stubGlobal("location",{protocol:"https:",host:"127.0.0.1:49821",hostname:"127.0.0.1"});
  const save=vi.fn(), key="b".repeat(64);
  vi.stubGlobal("PlumeLinkAndroid",{getStoredToken:()=>"a".repeat(64),getUsbAccessKey:()=>key,setStoredToken:save});
  const remote=useConnection();remote.resume();const ws=Socket.sockets[0];expect(ws.url).toBe("wss://127.0.0.1:49821/ws?usb="+key);
  await ws.open();
  expect(remote.usbExpired.value).toBe(false);
  expect(remote.connectionLabel.value).toBe("已连接");
  expect(isReadonly(remote.usbExpired)).toBe(true);
  ws.close(4409);
  expect(remote.usbExpired.value).toBe(true);
  for (const [language, label] of [["zh-CN", "USB 连接已失效"], ["zh-TW", "USB 連線已失效"], ["en", "USB connection expired"]] as const) {
    languagePreference.value = language;
    expect(remote.connectionLabel.value).toBe(label);
    expect(remote.connectionBadgeLabel.value).toBe(label);
  }
  remote.resume();await vi.advanceTimersByTimeAsync(15000);remote.resume();
  expect(Socket.sockets).toHaveLength(1);expect(remote.token()).toBe("a".repeat(64));expect(save).not.toHaveBeenCalled();expect(remote.message.value).toContain("USB 有线连接");remote.suspend();
});
it("keeps the pairing credential after a USB HTTP expiry and supplies the key header", async () => {
  vi.stubGlobal("location",{protocol:"https:",host:"127.0.0.1:49821",hostname:"127.0.0.1"});const key="b".repeat(64),save=vi.fn();
  vi.stubGlobal("PlumeLinkAndroid",{getStoredToken:()=>"a".repeat(64),getUsbAccessKey:()=>key,setStoredToken:save});
  const fetcher=vi.fn(async(_path:unknown,_init:RequestInit)=>({status:410}));vi.stubGlobal("fetch",fetcher);
  const remote=useConnection();
  expect(remote.usbExpired.value).toBe(false);
  expect(remote.connectionLabel.value).not.toContain("失效");
  await expect(remote.api("/api/status")).rejects.toThrow("USB 有线连接");
  expect(remote.usbExpired.value).toBe(true);
  expect(remote.connectionLabel.value).toBe("USB 连接已失效");
  expect(remote.connectionBadgeLabel.value).toBe("USB 连接已失效");
  const headers=fetcher.mock.calls[0][1].headers as Headers;expect(headers.get("X-PlumeLink-USB")).toBe(key);expect(headers.get("Authorization")).toBe("Bearer "+"a".repeat(64));
  expect(remote.token()).toBe("a".repeat(64));expect(save).not.toHaveBeenCalled();await expect(remote.api("/api/status")).rejects.toThrow("USB 有线连接");expect(fetcher).toHaveBeenCalledTimes(1);remote.suspend();
});
