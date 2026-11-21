import { afterEach, expect, it, vi } from 'vitest';
import { uploadResumable, MAX_FILE_BYTES } from '../apps/mobile/file-transfer';
afterEach(()=>vi.unstubAllGlobals());
it('keeps upload requests small during audio and cancels without another chunk',async()=>{
  const source=file(256*1024),controller=new AbortController();const sizes:number[]=[];let offset=0;
  vi.stubGlobal('fetch',vi.fn(async(path:string,init:RequestInit)=>{
    if(path==='/api/uploads')return status(0,source.size);
    if(init.method==='PUT'){const size=(init.body as ArrayBuffer).byteLength;sizes.push(size);offset+=size;controller.abort();return status(offset,source.size);}
    return status(offset,source.size);
  }));
  await expect(uploadResumable(source,{token:'token',signal:controller.signal,valid:()=>true,progress:()=>{},realtime:()=>true})).rejects.toThrow('取消');
  expect(sizes).toEqual([64*1024]);
});
function file(size:number){const bytes=new Blob([new Uint8Array(size)]);return {size,name:'test.bin',slice:bytes.slice.bind(bytes)} as File;}
const status=(offset:number,size:number,complete=false)=>new Response(JSON.stringify({offset,size,chunkBytes:4*1024**2,complete,name:complete?'test.bin':null}),{status:200,headers:{'Content-Type':'application/json'}});
it('resumes at confirmed offsets after a lost chunk response and completes once',async()=>{
  const source=file(4*1024**2+8);let offset=0;let writes=0;let finishes=0;const offsets:number[]=[];
  vi.stubGlobal('fetch',vi.fn(async(path:string,init:RequestInit)=>{
    if(path==='/api/uploads')return status(0,source.size);
    if(init.method==='GET')return status(offset,source.size);
    if(init.method==='PUT'){
      expect((init.headers as Record<string,string>)['X-Upload-Offset']).toBe(String(offset));
      expect((init.headers as Record<string,string>)['X-Chunk-SHA256']).toMatch(/^[a-f0-9]{64}$/);
      offset+=(init.body as ArrayBuffer).byteLength;writes++;
      if(writes===1)throw new TypeError('lost response after write');return status(offset,source.size);
    }
    finishes++;return status(offset,source.size,true);
  }));
  expect(await uploadResumable(source,{token:'token',signal:new AbortController().signal,valid:()=>true,progress:b=>offsets.push(b)})).toBe('test.bin');
  expect(writes).toBe(2);expect(finishes).toBe(1);expect(offsets.at(-1)).toBe(source.size);
});
it('resolves a lost completion response without publishing again',async()=>{
  let offset=0,complete=false,finishes=0;
  vi.stubGlobal('fetch',vi.fn(async(path:string,init:RequestInit)=>{
    if(path==='/api/uploads')return status(0,10);
    if(init.method==='GET')return status(offset,10,complete);
    if(init.method==='PUT'){offset=10;return status(10,10);}
    finishes++;complete=true;throw new TypeError('lost final response');
  }));
  expect(await uploadResumable(file(10),{token:'token',signal:new AbortController().signal,valid:()=>true,progress:()=>{}})).toBe('test.bin');expect(finishes).toBe(1);
});
it('publishes a zero-byte file without sending a data chunk', async () => {
  let finishes = 0;
  const progress: number[] = [];
  vi.stubGlobal('fetch', vi.fn(async (_path: string, init: RequestInit) => {
    if (init.method === 'POST' && init.body) return status(0, 0);
    finishes++;
    return status(0, 0, true);
  }));
  const empty = file(0);
  expect(await uploadResumable(empty, { token: 'token', signal: new AbortController().signal, valid: () => true, progress: bytes => progress.push(bytes) })).toBe('test.bin');
  expect(finishes).toBe(1);
  expect(progress).toEqual([0, 0]);
});
it('rejects over-2GB files before reading or sending and aborts revoked sessions',async()=>{
  const fetch=vi.fn();vi.stubGlobal('fetch',fetch);
  await expect(uploadResumable({size:MAX_FILE_BYTES+1,name:'large'} as File,{token:'token',signal:new AbortController().signal,valid:()=>true,progress:()=>{}})).rejects.toThrow('2 GB');expect(fetch).not.toHaveBeenCalled();
  const controller=new AbortController();controller.abort();fetch.mockResolvedValue(status(0,10));
  await expect(uploadResumable(file(10),{token:'token',signal:controller.signal,valid:()=>true,progress:()=>{}})).rejects.toThrow('取消');
  expect(fetch.mock.calls.every(([,init])=>init.method==='DELETE')).toBe(true);
});
it('rejects invalid upload names and non-finite sizes before sending', async () => {
  const fetch = vi.fn(); vi.stubGlobal('fetch', fetch);
  await expect(uploadResumable({ size: 10, name: 'bad\nname', slice: () => new Blob() } as unknown as File, { token: 'token', signal: new AbortController().signal, valid: () => true, progress: () => {} })).rejects.toThrow('文件名无效');
  await expect(uploadResumable({ size: Number.NaN, name: 'bad-size' } as File, { token: 'token', signal: new AbortController().signal, valid: () => true, progress: () => {} })).rejects.toThrow('大小无效');
  expect(fetch).not.toHaveBeenCalled();
});
it('rejects Windows device and ambiguous upload names before sending', async () => {
  const fetch = vi.fn(); vi.stubGlobal('fetch', fetch);
  for (const name of ['CON.txt', 'file.', 'file ', 'a/b.txt', '.plumelink-1.part']) {
    await expect(uploadResumable({ size: 10, name } as File, { token: 'token', signal: new AbortController().signal, valid: () => true, progress: () => {} })).rejects.toThrow('文件名无效');
  }
  expect(fetch).not.toHaveBeenCalled();
});
it('uses UTF-8 byte limits for multibyte names', async () => {
  const fetch = vi.fn(); vi.stubGlobal('fetch', fetch);
  const name = '界'.repeat(81); // 243 UTF-8 bytes, under 260 code units
  await expect(uploadResumable({ size: 1, name } as File, { token: 'token', signal: new AbortController().signal, valid: () => true, progress: () => {} })).rejects.toThrow('文件名无效');
  expect(fetch).not.toHaveBeenCalled();
});
