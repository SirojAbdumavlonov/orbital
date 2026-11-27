import { CalendarRange } from "lucide-react";
import { useState } from "react";
import { bounds, fromLocalInput, PRESETS, toLocalInput, useRange } from "../lib/range";

/** The one date range all monitoring dashboards follow (Monitor, Home, Uptime). */
export function RangePicker() {
  const { range, set } = useRange();
  const [custom, setCustom] = useState(false);
  const b = bounds(range);
  const [from, setFrom] = useState(toLocalInput(b.from));
  const [to, setTo] = useState(toLocalInput(b.to));
  const isCustom = !("preset" in range);
  const apply = () => {
    const f = fromLocalInput(from);
    const t = fromLocalInput(to);
    if (f !== null && t !== null && f !== t) { set({ from: f, to: t }); setCustom(false); }
  };
  return (
    <div className="range-picker" title="Applies to every monitoring view: Monitor, Home and Uptime">
      <div className="segmented">
        {PRESETS.map((p) => (
          <button key={p.id} className={"preset" in range && range.preset === p.id ? "on" : ""} onClick={() => { set({ preset: p.id }); setCustom(false); }}>{p.label}</button>
        ))}
        <button className={isCustom ? "on" : ""} onClick={() => { const bb = bounds(range); setFrom(toLocalInput(bb.from)); setTo(toLocalInput(bb.to)); setCustom((c) => !c); }}>
          <CalendarRange size={12} /> {isCustom ? "Dates" : "Dates…"}
        </button>
      </div>
      {custom && (
        <div className="range-custom">
          <input type="datetime-local" className="input" value={from} max={to} onChange={(e) => setFrom(e.target.value)} aria-label="From" />
          <span className="faint">to</span>
          <input type="datetime-local" className="input" value={to} min={from} onChange={(e) => setTo(e.target.value)} aria-label="To" />
          <button className="btn primary" onClick={apply}>Show</button>
        </div>
      )}
    </div>
  );
}
