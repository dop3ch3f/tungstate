// Which app to open. Read from the web view's own name for itself, which is
// known before anything loads, so desktop start-up waits on nothing.
// `?phone` opens the phone app in a browser, for the harness.

export function onPhone(): boolean {
  return /iPhone|iPad|Android/.test(navigator.userAgent) || new URLSearchParams(location.search).has("phone");
}

/** Android draws under its status bar and gives the web view no inset for
 *  it; iOS reports its insets through `env(safe-area-inset-*)`. `?android`
 *  says so in a browser, for the harness. */
export const isAndroid = (): boolean =>
  /Android/.test(navigator.userAgent) || new URLSearchParams(location.search).has("android");
