// Drawing only the rows on screen. A phone's web view slows down badly with
// thousands of rows; a camera folder has thousands. Every row in one view is
// the same height, so which rows are visible is arithmetic, not measurement.

import { computed, onMounted, onUnmounted, ref, type Ref } from "vue";

/** Rows drawn above and below the screen, so a quick flick lands on drawn rows. */
const SPARE = 6;

/** The rows to draw, as `[first, end)`, for a list scrolled to `top`. */
export function rowsInView(top: number, height: number, rowHeight: number, rows: number, spare = SPARE): [number, number] {
  if (rows <= 0 || rowHeight <= 0) return [0, 0];
  const first = Math.max(0, Math.floor(top / rowHeight) - spare);
  const end = Math.min(rows, Math.ceil((top + height) / rowHeight) + spare);
  return [first, Math.max(first, end)];
}

/** Follows the scroll position and size of `box`, the list's scroller. */
export function useScroll(box: Ref<HTMLElement | null>) {
  const top = ref(0);
  const height = ref(800);
  const width = ref(360);
  const measure = () => {
    const el = box.value;
    if (!el) return;
    top.value = el.scrollTop;
    height.value = el.clientHeight;
    width.value = el.clientWidth;
  };
  let watcher: ResizeObserver | undefined;
  onMounted(() => {
    measure();
    box.value?.addEventListener("scroll", measure, { passive: true });
    if (typeof ResizeObserver !== "undefined" && box.value) {
      watcher = new ResizeObserver(measure);
      watcher.observe(box.value);
    }
  });
  onUnmounted(() => {
    box.value?.removeEventListener("scroll", measure);
    watcher?.disconnect();
  });
  return { top, height, width, measure, at: computed(() => top.value) };
}
