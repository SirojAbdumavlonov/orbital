import { Box } from "lucide-react";
import { ReactNode, useEffect, useState } from "react";
import { SessionState } from "../store";

/**
 * Where a database or web server tab looks: the server itself, or one of the
 * running containers whose image is that program (from tool detection).
 * Returns the container name (null = the server) and a picker for the toolbar.
 */
export function usePlace(s: SessionState, toolId: string): [string | null, ReactNode] {
  const tool = s.tools?.find((t) => t.id === toolId);
  const containers = tool?.containers ?? [];
  const onHost = tool?.onHost ?? true;
  const first = !onHost && containers.length ? containers[0] : null;
  const [place, setPlace] = useState<string | null>(first);
  // Detection can arrive after the tab opened; a vanished container falls back.
  useEffect(() => {
    if (place && !containers.includes(place)) setPlace(first);
    else if (!place && !onHost && first) setPlace(first);
  }, [onHost, containers.join(",")]);

  let picker: ReactNode = null;
  if (containers.length > 1 || (containers.length === 1 && onHost)) {
    picker = (
      <label className="place-pick" title="Where to look: this server, or a Docker container running it">
        <Box size={13} />
        <select className="input" value={place ?? ""} onChange={(e) => setPlace(e.target.value || null)}>
          {onHost && <option value="">This server</option>}
          {containers.map((c) => <option key={c} value={c}>Container {c}</option>)}
        </select>
      </label>
    );
  } else if (place) {
    picker = <span className="badge" title="Runs in Docker; Orbital works inside the container"><Box size={12} /> container {place}</span>;
  }
  return [place, picker];
}
