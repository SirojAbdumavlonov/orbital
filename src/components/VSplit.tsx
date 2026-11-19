import { ChevronDown, ChevronRight } from "lucide-react";
import { ReactNode, useRef, useState } from "react";

type Layout = { ratio: number; top: boolean; bottom: boolean };
const DEFAULT: Layout = { ratio: 0.5, top: true, bottom: true };
const MIN = 0.15;

function load(key: string): Layout {
  try {
    const v = JSON.parse(localStorage.getItem(`orbital.split.${key}`) ?? "null");
    if (v && typeof v.ratio === "number") return { ...DEFAULT, ...v };
  } catch { /* defaults */ }
  return DEFAULT;
}

function store(key: string, l: Layout) {
  try {
    localStorage.setItem(`orbital.split.${key}`, JSON.stringify(l));
  } catch { /* not remembered */ }
}

/**
 * Two panes stacked in a column: drag the line between them to change their
 * heights (double-click resets), and fold either one away with its chevron.
 * The layout is remembered per `id`.
 */
export function VSplit({ id, top, bottom }: {
  id: string;
  top: { title: ReactNode; extra?: ReactNode; body: ReactNode };
  bottom: { title: ReactNode; extra?: ReactNode; body: ReactNode };
}) {
  const [l, setL] = useState<Layout>(() => load(id));
  const box = useRef<HTMLDivElement>(null);
  const update = (n: Layout) => {
    setL(n);
    store(id, n);
  };

  function drag(e: React.PointerEvent) {
    if (e.button !== 0 || !box.current) return;
    e.preventDefault();
    const r = box.current.getBoundingClientRect();
    let last = l;
    document.body.classList.add("resizing-v");
    const move = (m: PointerEvent) => {
      const ratio = Math.min(1 - MIN, Math.max(MIN, (m.clientY - r.top) / r.height));
      last = { ...last, ratio };
      setL(last);
    };
    const up = () => {
      document.body.classList.remove("resizing-v");
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      store(id, last);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  const both = l.top && l.bottom;
  const flex = (open: boolean, share: number) => (!open ? "none" : both ? `${share} 1 0` : "1 1 0");
  const head = (which: "top" | "bottom", p: { title: ReactNode; extra?: ReactNode }) => {
    const open = l[which];
    return (
      <div className="vsplit-head">
        <button
          className="icon-btn"
          aria-expanded={open}
          aria-label={open ? "Fold this pane" : "Show this pane"}
          title={open ? "Fold this pane" : "Show this pane"}
          onClick={() => update({ ...l, [which]: !open })}
        >
          {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </button>
        {p.title}
        <span className="spacer" />
        {open && p.extra}
      </div>
    );
  };

  return (
    <div className="vsplit" ref={box}>
      <section className="vsplit-pane" style={{ flex: flex(l.top, l.ratio) }}>
        {head("top", top)}
        {l.top && top.body}
      </section>
      {both && (
        <div
          className="vsplit-handle"
          role="separator"
          aria-orientation="horizontal"
          title="Drag to resize · double-click to reset"
          onPointerDown={drag}
          onDoubleClick={() => update({ ...l, ratio: DEFAULT.ratio })}
        />
      )}
      <section className="vsplit-pane" style={{ flex: flex(l.bottom, 1 - l.ratio) }}>
        {head("bottom", bottom)}
        {l.bottom && bottom.body}
      </section>
    </div>
  );
}
