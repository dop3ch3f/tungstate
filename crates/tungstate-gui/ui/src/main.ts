import { createApp } from "vue";
import App from "./App.vue";
import "./styles.css";
// After the old sheet on purpose: where the two overlap, the new tokens and
// base rules win. The old screens are reachable on `0` during the rebuild and
// will pick up the new ground colour until they are deleted, which is the
// cheaper of the two wrong answers.
import "./styles/tokens.css";
import "./styles/base.css";

createApp(App).mount("#app");
