import { createApp } from "vue";
import App from "./App.vue";
import "./styles/tokens.css";
import "./styles/base.css";

// Retro is the default look. Graphite and paper are finished themes waiting for
// a settings switch; until then this is the one place that picks.
document.documentElement.dataset.theme = "retro";

createApp(App).mount("#app");
