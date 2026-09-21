// Find the class of defect this project keeps shipping: a selector that
// matches nothing.
//
// Slice 7b opened the window and found eleven defects. Four were this: `.mono`
// used three times and defined nowhere, `.path` scoped so tightly it missed
// the two places that needed it, `.spacer` defined only under `.topbar` so the
// toolbar it was written for matched nothing, and `aria-current` bound in three
// places with no rule to draw it. Every one of them renders as "slightly
// wrong" rather than as an error. Nothing fails, so nothing tells you.
//
// Zero dependencies, and wired into `npm run build` ahead of `vue-tsc`, so the
// same check runs on all three platforms in CI without a workflow change.
//
// What it cannot do: it proves existence, not correctness. It cannot see that
// a row is three pixels too tall or that a layout breaks at 860px. That is
// what looking is for.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

// `new URL(..).pathname` is a URL path, not a filesystem path. On Windows it
// is `/D:/a/tungstate/...`, and joining that produces `D:\D:\a\...` -- a
// doubled drive letter and an ENOENT. `fileURLToPath` is the one that knows
// the difference. Windows has now caught a real bug in three slices running,
// which is the argument for the matrix.
const ROOT = fileURLToPath(new URL("..", import.meta.url));
const SRC = join(ROOT, "src");

// Classes that are legitimately used without a rule of their own, each with a
// reason. Anything not here and not styled is a defect.
const ALLOW = new Set([]);

// Files still on the old stylesheet, skipped until they are deleted. This list
// only ever shrinks; it is empty now, which is what "the migration is done"
// looks like.
const LEGACY = new Set([]);

const problems = [];
const fail = (file, line, rule, message) =>
  problems.push({ file: relative(ROOT, file), line, rule, message });

/** Every file under a directory, recursively. */
function walk(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...walk(path));
    else out.push(path);
  }
  return out;
}

const lineOf = (text, index) => text.slice(0, index).split("\n").length;

/** An SFC's template, up to its LAST `</template>`.
 *
 *  Not its first: Vue templates contain nested `<template v-if>` elements, and
 *  stopping at the first close reads a third of the file and calls the rest
 *  clean. That bug was written twice here before it was noticed. */
function templateOf(text) {
  const open = text.indexOf("<template>");
  const close = text.lastIndexOf("</template>");
  return open === -1 || close <= open ? null : text.slice(open, close);
}

function styleOf(text) {
  const open = text.indexOf("<style");
  return open === -1 ? null : text.slice(open);
}

/** Class names a template asks for, from every binding form Vue allows. */
function classesUsed(tpl, file) {
  const out = new Set();
  // Plain `class="a b"`, skipping the `:class="{...}"` the same regex sees.
  for (const m of tpl.matchAll(/(?<!:)\bclass="([^"]*)"/g)) {
    for (const word of m[1].split(/\s+/)) {
      if (/^[a-zA-Z][\w-]*$/.test(word)) out.add(word);
    }
  }
  // `:class="{ 'a-b': cond, c: cond }"`, `:class="[a, 'b']"`, ternaries.
  for (const m of tpl.matchAll(/:class="([^"]*)"/g)) {
    const body = m[1];
    if (body.includes("`")) {
      fail(
        file,
        lineOf(tpl, m.index),
        "template-literal-class",
        "a backtick class binding cannot be checked; use a lookup object instead",
      );
      continue;
    }
    if (body.trim().startsWith("{")) {
      // An object binding: only the keys are class names. A quoted string on
      // the right of a `:` is a value being compared, not a class -- reading
      // those made `nav.view === 'history'` look like a class nobody styled.
      for (const k of body.matchAll(/(?:^|[{,])\s*'([\w-]+)'\s*:/g)) out.add(k[1]);
      for (const k of body.matchAll(/(?:^|[{,])\s*"([\w-]+)"\s*:/g)) out.add(k[1]);
      for (const k of body.matchAll(/(?:^|[{,])\s*([a-zA-Z][\w-]*)\s*:/g)) out.add(k[1]);
    } else {
      // An array or an expression: every quoted string in it is a candidate.
      for (const q of body.matchAll(/'([\w-]+)'|"([\w-]+)"/g)) out.add(q[1] ?? q[2]);
    }
  }
  return out;
}

/** Quoted string tokens anywhere in a file.
 *
 *  A lookup object is the pattern this script pushes people towards, since a
 *  backtick binding cannot be checked at all. So `const LOOK = { primary:
 *  "btn-primary" }` has to count as using `.btn-primary`, or the rule against
 *  dead selectors would punish the very fix it recommends. Only consulted for
 *  that rule: markup still has to name a class outright to satisfy the rule
 *  about classes nobody styled. */
const quotedIn = (text) =>
  new Set([...text.matchAll(/['"]([a-zA-Z][\w-]*)['"]/g)].map((m) => m[1]));

/** Class names a stylesheet defines. */
const classesDefined = (css) =>
  new Set([...css.matchAll(/\.(-?[a-zA-Z][\w-]*)/g)].map((m) => m[1]));

const varsDeclared = (css) =>
  new Set([...css.matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]));

const varsUsed = (css) => [...css.matchAll(/var\((--[\w-]+)/g)];

// --- the tokens every file may reach for --------------------------------

let tokenText = "";
try {
  tokenText = readFileSync(join(SRC, "styles/tokens.css"), "utf8");
} catch {
  // Before commit 4 there is no token file; the other rules still apply.
}
const tokens = varsDeclared(tokenText);

let baseText = "";
try {
  baseText = readFileSync(join(SRC, "styles/base.css"), "utf8");
} catch {}
const baseClasses = classesDefined(baseText);

/** Every class any global stylesheet defines.
 *
 *  Scoped styles do not protect against these. Vue rewrites `.head` to
 *  `.head[data-v-x]`, which wins on specificity for the properties it sets --
 *  and leaks every property it does not. The old sheet's `.head` is
 *  `display: grid` with five fixed columns; a component that styled its own
 *  `.head` without declaring `display` got the grid, and its heading rendered
 *  one word per line inside a 26px column. Nothing failed.
 *
 *  So a component may *use* a global class, and may not *redefine* one. */
let legacyClasses = new Set();
try {
  legacyClasses = classesDefined(readFileSync(join(SRC, "styles.css"), "utf8"));
} catch {}
for (const c of baseClasses) legacyClasses.delete(c);

// --- every file ----------------------------------------------------------

for (const file of walk(SRC)) {
  if (LEGACY.has(file)) continue;
  const text = readFileSync(file, "utf8");
  const isVue = file.endsWith(".vue");
  const isCss = file.endsWith(".css");
  if (!isVue && !isCss) continue;

  const css = isCss ? text : (styleOf(text) ?? "");
  const local = varsDeclared(css);

  // 1. a `var(--x)` nothing declares. It falls back to nothing, silently.
  for (const m of varsUsed(css)) {
    const name = m[1];
    if (!tokens.has(name) && !local.has(name)) {
      fail(file, lineOf(text, text.indexOf(m[0])), "undeclared-token", `${name} is declared nowhere`);
    }
  }

  // 2. the raw palette is the implementation; the semantic layer is the interface.
  if (!file.endsWith("tokens.css")) {
    for (const m of varsUsed(css)) {
      if (m[1].startsWith("--raw-")) {
        fail(file, lineOf(text, text.indexOf(m[0])), "raw-token-leak", `${m[1]} is a raw palette token; use a semantic one`);
      }
    }
  }

  if (!isVue) continue;
  const tpl = templateOf(text);
  if (tpl === null) continue;

  const used = classesUsed(tpl, file);
  const defined = classesDefined(css);
  const quoted = quotedIn(text);

  // 3. a rule written against markup that is not there.
  for (const name of defined) {
    if (!used.has(name) && !quoted.has(name) && !baseClasses.has(name)) {
      fail(file, 0, "orphan-selector", `.${name} is styled here but used nowhere in this file`);
    }
  }

  // 4. an element wearing a class the old global sheet also claims. Naming it
  //    is what invites the leak, whether or not this file also styles it.
  for (const name of used) {
    if (legacyClasses.has(name)) {
      fail(file, 0, "shadows-global", `.${name} is claimed by the old global stylesheet; whatever it sets and this file does not will leak in`);
    }
  }

  // 5. markup asking for a rule nobody wrote. This is the one that shipped
  //    four of slice 7b's eleven.
  for (const name of used) {
    if (!defined.has(name) && !baseClasses.has(name) && !ALLOW.has(name)) {
      fail(file, 0, "unstyled-class", `.${name} is used here but styled nowhere`);
    }
  }

  // 6. a state bound and never drawn. "The folder you opened" and "the view
  //    you are in" were both marked this way and neither was visible.
  for (const m of tpl.matchAll(/:?(aria-(?:current|busy|invalid|selected|expanded))=/g)) {
    const attr = m[1];
    if (!css.includes(`[${attr}`)) {
      fail(file, lineOf(tpl, m.index), "undrawn-state", `${attr} is bound but no [${attr}] rule draws it`);
    }
  }
}

// --- say what is wrong ---------------------------------------------------

if (process.argv.includes("--list")) {
  console.log(`tokens declared: ${tokens.size}`);
  console.log(`legacy files skipped: ${LEGACY.size}`);
}

if (problems.length === 0) {
  console.log("check-css: clean");
  process.exit(0);
}

for (const p of problems) {
  const where = p.line ? `${p.file}:${p.line}` : p.file;
  console.error(`${where}  [${p.rule}]  ${p.message}`);
}
console.error(`\ncheck-css: ${problems.length} problem(s)`);
process.exit(1);
