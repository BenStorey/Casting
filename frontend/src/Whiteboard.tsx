import { useRef, useState } from "react";
import { Excalidraw, serializeAsJSON } from "@excalidraw/excalidraw";
import "@excalidraw/excalidraw/index.css";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card } from "@/components/ui/card";
import { saveDiagram } from "./api";

/// A freeform drawing surface (Excalidraw, MIT) for architecture diagrams + UI
/// sketches. We hold the live elements/appState in a ref (updated on every
/// change) so the Save button serializes the document DIRECTLY
/// (serializeAsJSON -> POST /api/diagram) — no export/download/re-upload.
/// Saved diagrams are durable, reloadable visual artifacts.
///
/// Licensing: chosen over tldraw because tldraw requires a PAID production
/// license (commercial $6k/yr; even OSS downstream users each need one).
/// Excalidraw is MIT — free forever, no keys/watermarks/obligations.

export default function Whiteboard({ onSaved }: { onSaved: () => void }) {
  // Excalidraw is controlled; we stash the latest doc so Save can serialize it.
  const docRef = useRef<{ elements: readonly any[]; appState: any }>({ elements: [], appState: null });
  const excalidrawAPI = useRef<any>(null);
  const [title, setTitle] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const handleChange = (elements: readonly any[], appState: any) => {
    docRef.current = { elements, appState };
  };

  const save = async () => {
    const { elements, appState } = docRef.current;
    if (!appState) return;
    setBusy(true);
    setMsg(null);
    try {
      // Direct capture: serialize the current scene into Excalidraw's JSON.
      const data = serializeAsJSON(elements, appState, {}, "local");
      if (JSON.parse(data).elements.length === 0) {
        throw new Error("nothing to save yet — draw something first");
      }
      await saveDiagram(title.trim() || "Untitled diagram", data);
      setMsg("Saved ✓");
      onSaved();
    } catch (e) {
      setMsg(`⚠️ ${String(e instanceof Error ? e.message : e)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Card className="p-0 overflow-hidden">
      <div className="flex flex-wrap items-center gap-2 border-b p-2">
        <Input
          className="max-w-xs h-8"
          placeholder="Diagram title (optional)"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
        />
        <Button size="sm" onClick={() => void save()} disabled={busy}>
          {busy ? "Saving…" : "💾 Save diagram"}
        </Button>
        {msg && <span className="text-xs text-muted-foreground">{msg}</span>}
      </div>
      <div className="h-[60vh]">
        <Excalidraw
          excalidrawAPI={(api) => (excalidrawAPI.current = api)}
          onChange={handleChange}
          initialData={{ appState: { viewBackgroundColor: "#0e1116" } }}
        />
      </div>
    </Card>
  );
}