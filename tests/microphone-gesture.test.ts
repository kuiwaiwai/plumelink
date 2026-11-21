import { expect, it } from 'vitest';
import { MicrophoneGesture } from '../apps/mobile/microphone-gesture';
it('interrupted presses never capture; long holds capture only until release',()=>{
  const events:string[]=[];const mic=new MicrophoneGesture(e=>events.push(e));
  mic.down(0,100,0);mic.tick(150);mic.up();expect(events).toEqual([]);
  mic.down(0,100,1000);mic.tick(1220);expect(events).toEqual(['start']);mic.up();expect(events).toEqual(['start','stop']);
});
it('upward motion must stay within the dwell area for one uninterrupted second',()=>{
  const events:string[]=[];const mic=new MicrophoneGesture(e=>events.push(e));
  mic.down(100,200,0);mic.tick(220);mic.move(100,120,400);mic.tick(1399);expect(mic.mode).toBe('holding');
  mic.move(100,80,1400);mic.tick(2399);expect(mic.mode).toBe('holding');mic.tick(2400);expect(mic.mode).toBe('locked');
  mic.up();expect(events).toEqual(['start','lock']);
  mic.down(100,200,5000);mic.tick(6999);mic.up();expect(mic.mode).toBe('locked');
  mic.down(100,200,8000);mic.tick(10000);expect(events).toEqual(['start','lock','stop']);expect(mic.mode).toBe('idle');
});
it('leaving the lock zone resets progress and interruption stops capture exactly once',()=>{
  const events:string[]=[];const mic=new MicrophoneGesture(e=>events.push(e));
  mic.down(100,200,0);mic.tick(220);mic.move(100,120,400);mic.tick(900);expect(mic.progress).toBe(.5);
  mic.move(100,180,1450);mic.tick(4000);expect(mic.mode).toBe('holding');expect(mic.progress).toBe(0);
  mic.cancel();mic.cancel();mic.up();expect(events).toEqual(['start','stop']);
});

it('a short tap and swipe never start capture; tapping while locked stops it', () => {
  const events: string[] = [];
  const mic = new MicrophoneGesture(e => events.push(e));
  mic.down(20, 20, 0); mic.move(60, 20, 50); mic.up(70);
  expect(events).toEqual([]);
  mic.down(20, 20, 100); mic.up(170);
  expect(mic.mode).toBe('idle');
  expect(events).toEqual([]);
  mic.restoreLocked();
  mic.down(20, 20, 200); mic.up(270);
  expect(events).toEqual(['stop']);
});
