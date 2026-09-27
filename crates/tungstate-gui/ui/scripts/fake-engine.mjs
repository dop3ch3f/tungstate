// A stand-in for the Rust side, under the same hook Tauri's `invoke` calls.
// Each test sets `engine.answer` to decide what a command returns and when.
export const engine = {
  calls: [],
  answer: async () => null,
};
globalThis.window = {
  __TAURI_INTERNALS__: {
    invoke: (cmd, args) => {
      engine.calls.push({ cmd, args });
      return engine.answer(cmd, args);
    },
    transformCallback: () => 0,
  },
};
// No screen to paint in a test: a frame is the next turn of the loop.
globalThis.requestAnimationFrame = (cb) => setTimeout(cb, 0);
