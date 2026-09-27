// Vite resolves `../engine/commands` to `commands.ts`; Node does not. This
// teaches the test runner the same rule, so tests can import the state
// modules the window uses rather than copies of them.
import { registerHooks } from "node:module";

registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (e) {
      if (!specifier.startsWith(".") || /\.[cm]?[jt]s$/.test(specifier)) throw e;
      return next(`${specifier}.ts`, context);
    }
  },
});
