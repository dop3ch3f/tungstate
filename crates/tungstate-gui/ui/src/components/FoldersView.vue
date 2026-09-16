<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { api, bytes, type FolderView, type LayoutView, type PreviewView } from "../api";

/// Plain words on screen, precise ones underneath: a policy is "rules", a plan
/// is a "preview", applying is "tidying up". The glossary is in the tour.
const folders = ref<FolderView[]>([]);
const layouts = ref<LayoutView[]>([]);
const chosen = ref<FolderView | null>(null);
const preview = ref<PreviewView | null>(null);

const error = ref("");
const note = ref("");
const busy = ref("");
const showing = ref<"trees" | "list" | "rules">("trees");
const rules = ref("");
const confirmingTidy = ref(false);
const confirmingForget = ref<FolderView | null>(null);

async function refresh() {
  error.value = "";
  try {
    folders.value = await api.governed();
    if (chosen.value) {
      const still = folders.value.find((f) => f.root === chosen.value?.root);
      chosen.value = still ?? null;
      if (!still) preview.value = null;
    }
  } catch (e) {
    error.value = String(e);
  }
}

onMounted(async () => {
  await refresh();
  try {
    layouts.value = await api.layouts();
  } catch (e) {
    error.value = String(e);
  }
});

async function add() {
  error.value = "";
  note.value = "";
  busy.value = "#add";
  try {
    const picked = await api.pickFolder();
    if (!picked) return;
    await api.governFolder(picked);
    await refresh();
    const added = folders.value.find((f) => f.root === picked);
    if (added) await open(added);
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function open(folder: FolderView) {
  chosen.value = folder;
  preview.value = null;
  rules.value = "";
  showing.value = "trees";
  error.value = "";
  note.value = "";
  if (!folder.has_rules || folder.broken) return;
  await look();
}

/** Work out what the rules would do. Changes nothing. */
async function look() {
  if (!chosen.value) return;
  busy.value = "#preview";
  error.value = "";
  try {
    preview.value = await api.folderPreview(chosen.value.root);
  } catch (e) {
    error.value = String(e);
    preview.value = null;
  } finally {
    busy.value = "";
  }
}

async function choose(layout: LayoutView) {
  if (!chosen.value) return;
  busy.value = `#layout-${layout.name}`;
  error.value = "";
  try {
    await api.giveRules(chosen.value.root, layout.name);
    note.value = `“${layout.summary}” is now this folder's rules. Nothing has moved — here is what it would do.`;
    await refresh();
    const again = folders.value.find((f) => f.root === chosen.value?.root);
    if (again) {
      chosen.value = again;
      await look();
    }
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function tidy() {
  if (!chosen.value) return;
  confirmingTidy.value = false;
  busy.value = "#tidy";
  error.value = "";
  note.value = "";
  try {
    note.value = await api.tidyFolder(chosen.value.root);
    await look();
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function putBack() {
  if (!chosen.value || preview.value?.undoable == null) return;
  busy.value = "#undo";
  error.value = "";
  note.value = "";
  try {
    note.value = await api.putBack(chosen.value.root, preview.value.undoable);
    await look();
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function forget(folder: FolderView) {
  confirmingForget.value = null;
  busy.value = "#forget";
  try {
    await api.forgetFolder(folder.root);
    if (chosen.value?.root === folder.root) {
      chosen.value = null;
      preview.value = null;
    }
    note.value = "Forgotten. Its rules and everything done to it are untouched.";
    await refresh();
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function showRules() {
  if (!chosen.value) return;
  showing.value = "rules";
  try {
    rules.value = await api.rulesText(chosen.value.root);
  } catch (e) {
    error.value = String(e);
  }
}

/** One sentence saying what the button would do, before it is pressed. */
const summary = computed(() => {
  const p = preview.value;
  if (!p) return "";
  if (p.tidy && p.waiting > 0) {
    return `Nothing to do yet — ${p.waiting} file${p.waiting === 1 ? "" : "s"} ${
      p.waiting === 1 ? "was" : "were"
    } written too recently to touch. The longest has ${p.longest_wait}s to wait.`;
  }
  if (p.tidy) return "This folder already matches its rules. Nothing to do.";
  return `${p.files} of ${p.of} file${p.of === 1 ? "" : "s"} would move (${bytes(p.bytes)}).`;
});

const policyPath = computed(() =>
  chosen.value ? `${chosen.value.root}/.tungstate/policy.toml` : "",
);
</script>

<template>
  <div class="sheet">
    <h1>Folders</h1>
    <p class="sub">
      A folder tungstate looks after. You say the shape you want; it shows you what it
      would change, and only changes it when you say so. Everything here can be put back.
    </p>

    <div v-if="error" class="notice bad">{{ error }}</div>
    <div v-if="note" class="notice">{{ note }}</div>

    <div class="go" style="margin-bottom: 16px">
      <button class="btn" :disabled="busy === '#add'" @click="add">Add a folder…</button>
    </div>

    <!-- Nothing yet. The one screen a first-time user sees, so it says what
         this is for rather than showing an empty table. -->
    <div v-if="!folders.length" class="empty">
      <strong>No folders yet.</strong>
      <span>
        Pick a messy one — Downloads is the usual first choice. Nothing will move until you
        have seen exactly what would happen and pressed a button.
      </span>
    </div>

    <table v-else class="ledger">
      <thead>
        <tr>
          <th>Folder</th>
          <th>Where</th>
          <th>Rules</th>
          <th class="actions"></th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="folder in folders"
          :key="folder.root"
          :aria-current="chosen?.root === folder.root"
        >
          <td><button class="asname" @click="open(folder)"><b>{{ folder.name }}</b></button></td>
          <td class="addr root">{{ folder.root }}</td>
          <td>
            <span v-if="folder.broken" class="warn">rules will not load</span>
            <span v-else-if="folder.has_rules">set</span>
            <span v-else class="warn">none yet</span>
          </td>
          <td class="actions">
            <button class="btn small" @click="confirmingForget = folder">Forget</button>
          </td>
        </tr>
      </tbody>
    </table>

    <!-- One folder, opened. -->
    <template v-if="chosen">
      <h2 style="margin-top: 22px">{{ chosen.name }}</h2>

      <div v-if="chosen.broken" class="notice bad">
        These rules will not load, so nothing can happen to this folder.
        <div class="mono" style="margin-top: 6px">{{ chosen.broken }}</div>
        <p class="sub" style="margin-top: 6px">
          Fix it in <span class="path">{{ policyPath }}</span> and press Refresh.
        </p>
        <button class="btn" style="margin-top: 8px" @click="open(chosen)">Refresh</button>
      </div>

      <!-- No rules. The wall this whole slice exists to remove: pick one and
           the preview appears without another click. -->
      <div v-else-if="!chosen.has_rules">
        <p class="sub">
          This folder has no rules yet, so nothing will happen to it. Pick a starting point
          — you can edit it in a text editor afterwards.
        </p>
        <div class="layouts">
          <button
            v-for="layout in layouts"
            :key="layout.name"
            class="layout"
            :disabled="busy === `#layout-${layout.name}`"
            @click="choose(layout)"
          >
            <strong>{{ layout.summary }}</strong>
            <span>{{ layout.detail }}</span>
          </button>
        </div>
      </div>

      <template v-else-if="preview">
        <p class="lede">{{ summary }}</p>

        <div v-if="!preview.settles" class="notice bad">
          These rules never settle: tidying would leave
          {{ preview.files }} file(s) still wanting to move, for ever. A rename that reads
          <span class="mono">{{ "{name}" }}</span> and adds to it renders differently once
          the file is renamed. Fix the rule before tidying.
        </div>
        <div v-else-if="preview.large && !preview.tidy" class="notice calm">
          This is a big change — it touches most of the folder. Read the two views below
          before you agree to it. You can put it back afterwards, but reading now is easier.
        </div>

        <div class="go" style="margin: 12px 0">
          <button
            class="btn primary"
            :disabled="preview.tidy || !preview.settles || busy === '#tidy'"
            @click="confirmingTidy = true"
          >
            Tidy up
          </button>
          <button
            class="btn"
            :disabled="preview.undoable == null || busy === '#undo'"
            @click="putBack"
          >
            Put it back
          </button>
          <button class="btn" :disabled="busy === '#preview'" @click="look">Refresh</button>
          <span class="spacer"></span>
          <button class="btn small" :aria-current="showing === 'trees'" @click="showing = 'trees'">
            Before and after
          </button>
          <button class="btn small" :aria-current="showing === 'list'" @click="showing = 'list'">
            Every move
          </button>
          <button class="btn small" :aria-current="showing === 'rules'" @click="showRules">
            Rules
          </button>
        </div>

        <!-- The answer to the question somebody actually has: what will my
             folder look like. -->
        <div v-if="showing === 'trees'" class="pair">
          <div class="opt">
            <label>Now</label>
            <ul class="tree">
              <li v-for="e in preview.before" :key="e.path" :class="{ moving: e.moves }">
                <span class="path">{{ e.path }}</span>
                <span v-if="!e.is_dir" class="size">{{ bytes(e.size) }}</span>
              </li>
            </ul>
          </div>
          <div class="opt">
            <label>After tidying</label>
            <ul class="tree">
              <li v-for="e in preview.after" :key="e.path" :class="{ moving: e.moves }">
                <span class="path">{{ e.path }}</span>
                <span v-if="!e.is_dir" class="size">{{ bytes(e.size) }}</span>
              </li>
            </ul>
          </div>
        </div>

        <div v-else-if="showing === 'list'">
          <table class="ledger">
            <thead>
              <tr><th>Now</th><th>Would become</th><th>Why</th></tr>
            </thead>
            <tbody>
              <tr v-for="m in preview.moves" :key="m.from + m.to">
                <td class="path">{{ m.from }}</td>
                <td class="path">{{ m.to }}</td>
                <td class="sub">{{ m.why }}</td>
              </tr>
              <tr v-if="!preview.moves.length">
                <td colspan="3" class="sub">Nothing would move.</td>
              </tr>
            </tbody>
          </table>

          <h3 v-if="preview.left_alone.length" style="margin-top: 16px">Left alone</h3>
          <table v-if="preview.left_alone.length" class="ledger">
            <tbody>
              <tr v-for="l in preview.left_alone" :key="l.path">
                <td class="path">{{ l.path }}</td>
                <td class="sub">{{ l.why }}</td>
              </tr>
            </tbody>
          </table>
        </div>

        <div v-else>
          <p class="sub">
            These are this folder's rules, at
            <span class="path">{{ policyPath }}</span>. Edit them in a text editor — the
            window shows them and never changes them, so the file stays yours.
          </p>
          <pre class="mono rules">{{ rules }}</pre>
        </div>
      </template>

      <div v-else-if="busy === '#preview'" class="sub">Looking…</div>
    </template>

    <!-- Overlays, after the chain and beside each other, which is where the
         `v-if` bug in slice 5b taught us they belong. -->
    <div v-if="confirmingTidy && preview" class="veil" @click.self="confirmingTidy = false">
      <div class="modal" role="dialog" aria-modal="true">
        <div class="cap">
          <h2>Tidy up {{ chosen?.name }}?</h2>
          <p class="why">{{ summary }}</p>
        </div>
        <div class="body">
          <p class="sub">
            Nothing is deleted. Anything tungstate cannot decide about is set aside rather
            than overwritten, and you can put the whole thing back with one button.
          </p>
        </div>
        <div class="feet">
          <span class="spacer"></span>
          <button class="btn" @click="confirmingTidy = false">Cancel</button>
          <button class="btn primary" @click="tidy">Tidy up</button>
        </div>
      </div>
    </div>

    <div v-if="confirmingForget" class="veil" @click.self="confirmingForget = null">
      <div class="modal" role="dialog" aria-modal="true">
        <div class="cap">
          <h2>Forget {{ confirmingForget.name }}?</h2>
          <p class="why">
            Tungstate stops looking after it. Its rules stay in the folder and everything
            already done to it stays in the record, so you can still look it up or put it back.
          </p>
        </div>
        <div class="feet">
          <span class="spacer"></span>
          <button class="btn" @click="confirmingForget = null">Cancel</button>
          <button class="btn" @click="forget(confirmingForget)">Forget it</button>
        </div>
      </div>
    </div>
  </div>
</template>
