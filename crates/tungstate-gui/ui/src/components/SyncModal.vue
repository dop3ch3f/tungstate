<script setup lang="ts">
import { ref, computed, watch, onMounted } from "vue";
import { api, bytes, type Preview, type Leg } from "../api";

const props = defineProps<{
  legs: Leg[];
  totalBytes: number;
  count: number;
  intent: "move" | "copy";
}>();

const exchange = computed(() => props.legs.length > 1);
const emit = defineEmits<{ cancel: []; start: [payload: Payload] }>();

export interface Payload {
  source_policy: string;
  verify: string;
  on_conflict: string;
  save_as: string | null;
}

const policy = ref(props.intent === "move" ? "delete" : "keep");
const verify = ref("hash");
const conflict = ref("quarantine");
const remember = ref(false);
const name = ref("");

const removes = computed(() => policy.value !== "keep");

const preview = ref<Preview | null>(null);
const looking = ref(true);
const previewError = ref("");

/// Recomputed when the originals setting changes, since that is what decides
/// whether the summary should warn about deletion.
async function look() {
  looking.value = true;
  try {
    preview.value = await api.previewTransfer({
      legs: props.legs,
      source_policy: policy.value,
      verify: verify.value,
      on_conflict: conflict.value,
      save_as: null,
    });
    previewError.value = "";
  } catch (e) {
    previewError.value = String(e);
    preview.value = null;
  } finally {
    looking.value = false;
  }
}

onMounted(look);
watch(policy, look);

const word: Record<string, string> = {
  move: "will move",
  check: "same size there",
  clash: "name taken",
  hold: "too recent",
};

function start() {
  emit("start", {
    source_policy: policy.value,
    verify: verify.value,
    on_conflict: conflict.value,
    save_as: remember.value && name.value.trim() ? name.value.trim() : null,
  });
}
</script>

<template>
  <div class="veil" @click.self="emit('cancel')">
    <div class="modal" role="dialog" aria-modal="true">
      <div class="cap">
        <h2>
          {{ props.intent === "move" ? "Move" : "Copy" }} {{ props.count }}
          item{{ props.count === 1 ? "" : "s" }}{{ exchange ? " both ways" : "" }}
        </h2>
        <p class="why">{{ bytes(props.totalBytes) }} in total. Every file is checked at the far
          end before anything is removed.</p>
      </div>

      <div class="body">
        <div v-for="(l, i) in props.legs" :key="i" class="route">
          <div class="end">
            <span>{{ exchange ? (i === 0 ? "Ticked on the left" : "Ticked on the right") : "From" }}</span>
            <b>{{ l.source }}</b>
          </div>
          <div class="arrow">{{ exchange && i === 1 ? "←" : "→" }}</div>
          <div class="end"><span>To</span><b>{{ l.destination }}</b></div>
        </div>

        <div v-if="preview?.overlapping.length" class="notice bad">
          <b>{{ preview.overlapping.length }} name{{ preview.overlapping.length === 1 ? " is" : "s are" }}
          ticked on both sides.</b>
          Each would clash in both directions, and resolving it twice is unlikely to give you what
          you want. Consider unticking one side:
          <span style="font-family: var(--mono)">{{ preview.overlapping.slice(0, 6).join(", ") }}</span>
        </div>

        <div v-if="previewError" class="notice bad">{{ previewError }}</div>

        <div v-else-if="looking" class="notice calm">Looking at both sides…</div>

        <div v-else-if="preview" class="prospect">
          <div class="tallies">
            <div><b>{{ preview.fresh }}</b><span>will move</span></div>
            <div v-if="preview.same_size"><b>{{ preview.same_size }}</b><span>same size there</span></div>
            <div v-if="preview.clashes"><b class="warn">{{ preview.clashes }}</b><span>name taken</span></div>
            <div v-if="preview.too_recent"><b class="warn">{{ preview.too_recent }}</b><span>too recent</span></div>
          </div>
          <ul class="lines">
            <li v-for="(item, i) in preview.items.slice(0, 60)" :key="item.path + i" :class="item.outcome">
              <span class="p">
                <span v-if="exchange" class="dir">{{ item.towards === "forward" ? "→" : "←" }}</span>
                {{ item.path }}
              </span>
              <span class="w">{{ word[item.outcome] }}</span>
              <span class="s">{{ bytes(item.size) }}</span>
            </li>
          </ul>
          <p v-if="preview.items.length > 60" class="note">
            and {{ preview.items.length - 60 }} more
          </p>
          <p class="note">
            {{ preview.removes_originals
              ? "Each original here is removed only after its copy passes the check."
              : "Nothing here is removed." }}
            Nothing has happened yet.
          </p>
        </div>

        <div class="pair">
          <div class="opt">
            <label for="policy">The originals</label>
            <select id="policy" v-model="policy">
              <option value="delete">Delete once the copy is verified</option>
              <option value="trash">Move to the trash</option>
              <option value="keep">Leave them here</option>
            </select>
            <span class="note">{{ removes
              ? "Space is reclaimed only after each copy passes its check."
              : "Nothing here is removed." }}</span>
          </div>

          <div class="opt">
            <label for="verify">Check each copy by</label>
            <select id="verify" v-model="verify">
              <option value="hash">Fingerprint taken while copying</option>
              <option value="readback">Reading it back again</option>
              <option value="size">Size only</option>
            </select>
            <span class="note">{{ verify === "readback"
              ? "Strongest, and reads every file a second time."
              : verify === "size" ? "Weakest: catches truncation only."
              : "Good default. Trusts the far end to store what it acknowledged." }}</span>
          </div>
        </div>

        <div class="opt">
          <label for="conflict">If a name is already taken over there</label>
          <select id="conflict" v-model="conflict">
            <option value="quarantine">Ask me, and set aside if I am away</option>
            <option value="rename">Keep both, renaming the arriving file</option>
            <option value="skip">Leave that one here</option>
          </select>
          <span class="note">An identical file already there counts as done, not a clash.</span>
        </div>

        <label v-if="!exchange" class="check">
          <input type="checkbox" v-model="remember" />
          <span>Remember this pair so I can run it again later</span>
        </label>
        <p v-else class="note">
          A remembered pair is one source and one destination, so an exchange cannot be saved.
        </p>
        <div v-if="remember" class="opt" style="margin-top: 10px">
          <input type="text" v-model="name" placeholder="laptop-to-nas" />
        </div>
      </div>

      <div class="feet">
        <span class="spacer"></span>
        <button class="btn" @click="emit('cancel')">Cancel</button>
        <button class="btn primary" :disabled="remember && !name.trim()" @click="start">
          {{ props.intent === "move" ? "Move" : "Copy" }} {{ props.count }}{{ exchange ? " both ways" : "" }}
        </button>
      </div>
    </div>
  </div>
</template>
