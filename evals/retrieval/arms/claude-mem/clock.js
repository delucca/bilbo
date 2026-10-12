// The build's clock, loaded into claude-mem's worker and every hook process through BUN_OPTIONS: Date.now(),
// new Date() and Date() read the time of the event being replayed, which build.py writes to $BILBO_CLOCK_FILE, and
// never move backwards. With BILBO_VIRTUAL_IDLE=1, the observer's 3-minute idle wait (SessionMessageBuffer.ts:5)
// ends when this clock passes it instead of after three real minutes. Everything else keeps real time.
'use strict';
const { readFileSync } = require('node:fs');

if (!globalThis.__bilboClock) {
  globalThis.__bilboClock = true;
  const FILE = process.env.BILBO_CLOCK_FILE;
  if (!FILE) throw new Error('clock.js: BILBO_CLOCK_FILE is not set');
  const RealDate = Date;
  let last = 0;
  const now = () => {
    const value = Number(readFileSync(FILE, 'utf8'));
    if (!Number.isFinite(value) || value <= 0) throw new Error(`clock.js: ${FILE} holds no time`);
    if (value > last) last = value;
    return last;
  };
  globalThis.Date = new Proxy(RealDate, {
    construct: (target, args, newTarget) => Reflect.construct(target, args.length ? args : [now()], newTarget),
    apply: () => new RealDate(now()).toString(),
    get: (target, prop, receiver) => (prop === 'now' ? now : Reflect.get(target, prop, receiver)),
  });

  if (process.env.BILBO_VIRTUAL_IDLE === '1') {
    const IDLE_MS = 180000;
    const realSetTimeout = globalThis.setTimeout;
    const realClearTimeout = globalThis.clearTimeout;
    const waiting = new Map();
    globalThis.setTimeout = function setTimeout(fn, ms, ...args) {
      if (ms !== IDLE_MS || typeof fn !== 'function' || !new Error().stack.includes('waitForMessage')) {
        return realSetTimeout(fn, ms, ...args);
      }
      // A real handle that never fires keeps unref(), ref() and clearTimeout() working on what the caller holds.
      const handle = realSetTimeout(() => {}, 2 ** 31 - 1);
      waiting.set(handle, { due: now() + IDLE_MS, fn, args });
      return handle;
    };
    globalThis.clearTimeout = function clearTimeout(handle) {
      waiting.delete(handle);
      return realClearTimeout(handle);
    };
    setInterval(() => {
      if (!waiting.size) return;
      const t = now();
      for (const [handle, timer] of waiting) {
        if (timer.due <= t) {
          waiting.delete(handle);
          realClearTimeout(handle);
          timer.fn(...timer.args);
        }
      }
    }, 50).unref();
  }
}
