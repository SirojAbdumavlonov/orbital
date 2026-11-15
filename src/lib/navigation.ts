import { useEffect } from "react";
import { useStore } from "../store";

/** Places where horizontal scrolling or the keys belong to the content, not to navigation. */
const OWN_INPUT = ".monaco-editor, .xterm, input, textarea, select";

/** Can `el` or a parent still scroll sideways in the direction of `dx`? */
function scrollsSideways(el: Element | null, dx: number): boolean {
  for (let n = el; n && n !== document.body; n = n.parentElement) {
    const cs = getComputedStyle(n);
    if (!/(auto|scroll)/.test(cs.overflowX) || n.scrollWidth <= n.clientWidth) continue;
    if (dx < 0 ? n.scrollLeft > 0 : n.scrollLeft + n.clientWidth < n.scrollWidth - 1) return true;
  }
  return false;
}

/**
 * Back and forward like other Mac apps: a two-finger swipe right on the
 * trackpad (left for forward), ⌘[ and ⌘], and the mouse's back and forward buttons.
 */
export function useNavigationGestures() {
  useEffect(() => {
    let sum = 0;
    let fired = false;
    let idle: number | undefined;
    const onWheel = (e: WheelEvent) => {
      if (e.ctrlKey || Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
      const t = e.target as Element;
      if (t.closest?.(OWN_INPUT) || t.closest?.(".modal") || scrollsSideways(t, e.deltaX)) return;
      window.clearTimeout(idle);
      // One gesture triggers one step; it ends when the wheel events stop.
      idle = window.setTimeout(() => { sum = 0; fired = false; }, 250);
      if (fired) return;
      sum += e.deltaX;
      if (sum < -140) { fired = true; useStore.getState().goBack(); }
      else if (sum > 140) { fired = true; useStore.getState().goForward(); }
    };
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || (e.key !== "[" && e.key !== "]")) return;
      if ((e.target as Element).closest?.(OWN_INPUT)) return;
      e.preventDefault();
      if (e.key === "[") useStore.getState().goBack();
      else useStore.getState().goForward();
    };
    const onMouse = (e: MouseEvent) => {
      if (e.button === 3) useStore.getState().goBack();
      if (e.button === 4) useStore.getState().goForward();
    };
    window.addEventListener("wheel", onWheel, { passive: true });
    window.addEventListener("keydown", onKey);
    window.addEventListener("mouseup", onMouse);
    return () => {
      window.removeEventListener("wheel", onWheel);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mouseup", onMouse);
    };
  }, []);
}
