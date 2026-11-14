import { Bar, Sparkline } from "../components/ui";
import { bytes, duration, percent, rate } from "../lib/format";
import { SessionState } from "../store";

export function MonitorView({ s }: { s: SessionState }) {
  const m = s.metrics[s.metrics.length - 1];
  if (!m) return <div className="empty"><p>Collecting the first sample…</p></div>;
  const hist = (f: (x: typeof m) => number) => s.metrics.map(f);
  const memPct = m.memTotal ? (m.memUsed / m.memTotal) * 100 : 0;
  const cachePct = m.memTotal ? (Math.min(m.memCached, m.memAvailable) / m.memTotal) * 100 : 0;
  const swapPct = m.swapTotal ? (m.swapUsed / m.swapTotal) * 100 : 0;

  return (
    <div className="monitor">
      <section className="panel span-4">
        <h4>CPU <span className="faint">{m.cores} cores</span></h4>
        <div className="big">{percent(m.cpuPercent)}</div>
        <Sparkline values={hist((x) => x.cpuPercent ?? 0)} width={300} height={46} fluid />
        <div className="cores" aria-label="Per-core usage">
          {m.perCorePercent.map((v, i) => <div key={i} className="core" title={`Core ${i}: ${v.toFixed(0)}%`}><i style={{ height: `${v}%` }} /></div>)}
        </div>
        <p className="faint" style={{ margin: "10px 0 0", fontSize: 12 }}>{m.cpuModel || "Unknown CPU"}</p>
      </section>

      <section className="panel span-4">
        <h4>Memory <span className="faint">{bytes(m.memTotal)}</span></h4>
        <div className="big">{bytes(m.memUsed)}</div>
        <Sparkline values={hist((x) => (x.memTotal ? (x.memUsed / x.memTotal) * 100 : 0))} width={300} height={46} fluid />
        <div style={{ marginTop: 12 }}><Bar value={memPct} cache={cachePct} hot={memPct > 90} /></div>
        <p className="faint" style={{ margin: "8px 0 0", fontSize: 12 }}>{bytes(m.memAvailable)} available · {bytes(m.memCached)} cache</p>
        <h4 style={{ marginTop: 14 }}>Swap <span className="faint">{m.swapTotal ? `${bytes(m.swapUsed)} of ${bytes(m.swapTotal)}` : "none"}</span></h4>
        {m.swapTotal > 0 && <Bar value={swapPct} hot={swapPct > 50} />}
      </section>

      <section className="panel span-4">
        <h4>Load average</h4>
        <div className="big">{m.load[0].toFixed(2)}</div>
        <Sparkline values={hist((x) => x.load[0])} max={m.cores} width={300} height={46} fluid />
        <p className="faint" style={{ margin: "10px 0 0", fontSize: 12 }}>5 min {m.load[1].toFixed(2)} · 15 min {m.load[2].toFixed(2)} · {m.load[0] > m.cores ? "busier than its core count" : "within capacity"}</p>
        <h4 style={{ marginTop: 14 }}>Network</h4>
        <div style={{ display: "flex", gap: 24 }}>
          <div><div className="faint" style={{ fontSize: 12 }}>In</div><b>{rate(m.netRxPerSec)}</b></div>
          <div><div className="faint" style={{ fontSize: 12 }}>Out</div><b>{rate(m.netTxPerSec)}</b></div>
        </div>
      </section>

      <section className="panel span-7">
        <h4>Disks</h4>
        <table className="table">
          <thead><tr><th>Mount</th><th>Device</th><th style={{ width: "34%" }}>Used</th><th className="num">Free</th></tr></thead>
          <tbody>
            {m.disks.map((d) => (
              <tr key={d.mount + d.filesystem}>
                <td className="mono">{d.mount}</td>
                <td className="faint mono" style={{ fontSize: 11.5 }}>{d.filesystem} · {d.fsType}</td>
                <td><div style={{ display: "flex", alignItems: "center", gap: 8 }}><Bar value={d.usedPercent} hot={d.usedPercent > 90} /><span style={{ fontSize: 12, minWidth: 34 }}>{d.usedPercent.toFixed(0)}%</span></div></td>
                <td className="num">{bytes(d.availableBytes)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className="panel span-5">
        <h4>System</h4>
        <dl className="kv" style={{ padding: 0 }}>
          <dt>Hostname</dt><dd>{m.hostname}</dd>
          <dt>OS</dt><dd>{m.osName}</dd>
          <dt>Kernel</dt><dd>{m.kernel}</dd>
          <dt>Uptime</dt><dd>{duration(m.uptimeSecs)}</dd>
          <dt>Docker</dt><dd>{s.info.dockerVersion ?? "not installed"}</dd>
          <dt>Init</dt><dd>{s.info.hasSystemd ? "systemd" : "other"}</dd>
        </dl>
      </section>

      <section className="panel span-12">
        <h4>Top processes <span className="faint">by CPU</span></h4>
        <table className="table">
          <thead><tr><th className="num">PID</th><th>User</th><th>Command</th><th className="num">CPU</th><th className="num">Memory</th><th className="num">RSS</th></tr></thead>
          <tbody>
            {m.processes.map((p) => (
              <tr key={p.pid}>
                <td className="num mono">{p.pid}</td>
                <td className="muted">{p.user}</td>
                <td className="mono">{p.command}</td>
                <td className="num">{p.cpuPercent.toFixed(1)}%</td>
                <td className="num">{p.memPercent.toFixed(1)}%</td>
                <td className="num">{bytes(p.rssKb * 1024)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </div>
  );
}
