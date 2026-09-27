import { createApp } from "vue";
import App from "./App.vue";
import "./styles/tokens.css";
import "./styles/base.css";
import { startTheme } from "./state/useTheme";

startTheme();

createApp(App).mount("#app");
