import { createApp } from "vue";
import App from "./App.vue";
import "./styles/tokens.css";
import "./styles/base.css";
import { startTheme } from "./state/useTheme";
import { onPhone } from "./phone/platform";

startTheme();

// A phone gets its own app, built for touch, on the same engine and stores.
// Loaded only there, so the desktop window carries none of it.
if (onPhone()) {
  void import("./phone/PhoneApp.vue").then(({ default: PhoneApp }) => createApp(PhoneApp).mount("#app"));
} else {
  createApp(App).mount("#app");
}
