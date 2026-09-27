// A stand-in for the Rust side, under the same hook Tauri's `invoke` calls.
// Each test sets `engine.answer` to decide what a command returns and when,
// and `engine.emit` sends an event to whatever the window is listening with.
const callbacks = new Map();
const listeners = [];
let nextId = 1;

export const engine = {
  calls: [],
  answer: async () => null,
  emit(event, payload) {
    for (const l of listeners.filter((l) => l.event === event)) {
      callbacks.get(l.handler)?.({ event, id: 0, payload });
    }
  },
};
globalThis.window = {
  __TAURI_INTERNALS__: {
    invoke: (cmd, args) => {
      engine.calls.push({ cmd, args });
      if (cmd === "plugin:event|listen") {
        listeners.push({ event: args.event, handler: args.handler });
        return Promise.resolve(listeners.length);
      }
      return engine.answer(cmd, args);
    },
    transformCallback: (cb) => {
      const id = nextId++;
      callbacks.set(id, cb);
      return id;
    },
  },
};
// No screen to paint in a test: a frame is the next turn of the loop.
globalThis.requestAnimationFrame = (cb) => setTimeout(cb, 0);
