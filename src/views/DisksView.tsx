import { AlertTriangle, CheckCircle2, HardDrive, RefreshCw, XCircle } from "lucide-react";
import { BlockDevice, api, DiskReport } from "../lib/api";
import { bytes } from "../lib/format";
import { useRemote } from "../lib/remote";
import { Bar } from "../components/ui";
import { SessionState, useStore } from "../store";

const pct = (a: number, b: number) => (b > 0 ? Math.round((a / b) * 100) : 0);

/** Disks: devices and partitions, filesystems with space and inodes, fstab, SMART health. */
export function DisksView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const { openFile } = useStore();
  const { data: r, error, loading, reload } = useRemote<DiskReport>(() => api.diskReport(sid), [sid]);
  const disks = (r?.devices ?? []).filter((d) => !d.parent && d.kind !== "loop");
  const children = (name: string): BlockDevice[] => (r?.devices ?? []).filter((d) => d.parent === name);

  const deviceRow = (d: BlockDevice, depth: number): JSX.Element[] => [
    <tr key={d.name}>
      <td><div style={{ paddingLeft: depth * 18 }} className="disk-name">{depth === 0 && <HardDrive size={13} className="faint" />}<span className="mono">/dev/{d.name}</span></div></td>
      <td className="muted">{d.kind}{depth === 0 && d.kind === "disk" ? (d.rotational ? " · spinning" : " · SSD") : ""}</td>
      <td className="num">{bytes(d.size)}</td>
      <td className="mono muted">{d.fstype ?? ""}</td>
      <td className="mono">{d.mountpoint ?? ""}</td>
      <td className="muted">{d.model ?? ""}</td>
      <td>{d.healthy === true ? <span className="badge ok"><CheckCircle2 size={11} /> healthy</span> : d.healthy === false ? <span className="badge bad"><XCircle size={11} /> failing</span> : null}</td>
    </tr>,
    ...children(d.name).flatMap((c) => deviceRow(c, depth + 1)),
  ];

  return (
    <div className="view">
      <div className="toolbar view-toolbar">
        <span className="faint" style={{ fontSize: 12.5 }}>Disks, partitions and filesystems on {s.profile.name}.</span>
        <span className="spacer" />
        <button className="btn" onClick={() => openFile(sid, "/etc/fstab")}>Edit /etc/fstab</button>
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      <div className="scroll">
        {error && <div className="alert" style={{ margin: 16 }}>{error.message}</div>}
        {r && r.warnings.length > 0 && (
          <div className="notice disk-warnings">{r.warnings.map((w) => <div key={w}><AlertTriangle size={13} className="bad-text" /> {w}</div>)}</div>
        )}
        {r && (
          <>
            <div className="section-title">Filesystems</div>
            <table className="table">
              <thead><tr><th>Mounted at</th><th>Device</th><th>Type</th><th style={{ width: 220 }}>Space</th><th style={{ width: 160 }}>Inodes (files)</th></tr></thead>
              <tbody>
                {r.filesystems.map((f) => (
                  <tr key={f.mountpoint}>
                    <td className="mono"><b>{f.mountpoint}</b></td>
                    <td className="mono muted">{f.source}</td>
                    <td className="muted">{f.fstype}</td>
                    <td><div className="disk-bar"><Bar value={pct(f.used, f.size)} hot={pct(f.used, f.size) >= 90} /><span className="faint">{bytes(f.used)} of {bytes(f.size)}</span></div></td>
                    <td>{f.inodes ? <div className="disk-bar"><Bar value={pct(f.inodesUsed ?? 0, f.inodes)} hot={pct(f.inodesUsed ?? 0, f.inodes) >= 90} /><span className="faint">{pct(f.inodesUsed ?? 0, f.inodes)}%</span></div> : <span className="faint">not limited</span>}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <div className="section-title">Devices</div>
            <table className="table">
              <thead><tr><th>Device</th><th>Kind</th><th className="num">Size</th><th>Filesystem</th><th>Mounted at</th><th>Model</th><th>SMART</th></tr></thead>
              <tbody>{disks.flatMap((d) => deviceRow(d, 0))}</tbody>
            </table>
            {r.smartMissing && <p className="footnote">Disk health (SMART) needs smartmontools: install <span className="mono">smartmontools</span> from the Packages tab. Virtual disks in cloud servers usually don't report SMART.</p>}
            <div className="section-title">Mounted at boot (/etc/fstab)</div>
            <table className="table">
              <thead><tr><th>Device</th><th>Mount point</th><th>Type</th><th>Options</th><th /></tr></thead>
              <tbody>
                {r.fstab.map((e) => (
                  <tr key={`${e.source} ${e.mountpoint}`}>
                    <td className="mono"><div className="cell-clip" style={{ maxWidth: 260 }} title={e.source}>{e.source}</div></td>
                    <td className="mono">{e.mountpoint}</td>
                    <td className="muted">{e.fstype}</td>
                    <td className="mono faint">{e.options}</td>
                    <td>{e.mounted ? <span className="faint">mounted</span> : e.options.split(",").includes("noauto") ? <span className="faint">on demand</span> : <span className="badge warn">not mounted</span>}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <p className="footnote">Read-only. A mistake in /etc/fstab can stop the server from booting: test a change with <span className="mono">sudo mount -a</span> before rebooting, and add <span className="mono">nofail</span> for disks that may be missing.</p>
          </>
        )}
      </div>
    </div>
  );
}
