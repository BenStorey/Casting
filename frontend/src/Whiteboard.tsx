import { useRef, useState } from "react";
import { Tldraw, serializeTldrawJson } from "tldraw";
import "tldraw/tldraw.css";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card } from "@/components/ui/card";
import { saveDiagram } from "./api";

/// A freeform drawing surface (tldraw) for architecture diagrams + UI sketches.
/// onMount gives us the Editor; we hold a ref so the Save button can serialize
/// the store DIRECTLY (serializeTldrawJson -> POST /api/diagram) — no
/// download/re-upload. Saved diagrams are durable, reloadable visual artifacts.

export default function Whiteboard({ onSaved }: { onSaved: () => void }) {
  const editorRef = useRef<any>(null);
  const [title, setTitle] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const save = async () => {
    const editor = editorRef.current;
    if (!editor) return;
    setBusy(true);
    setMsg(null);
    try {
      // Direct capture: serialize the whole tldraw document into its JSON string.
      const data = await serializeTldrawJson(editor);
      if (!data.trim()) throw new Error("nothing to save yet — draw something first");
      await saveDiagram(title.trim() || "Untitled diagram", data);
      setMsg("Saved ✓");
      onSaved();
    } catch (e) {
      setMsg(`⚠️ ${String(e)}`);
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
        <Tldraw onMount={(editor) => { editorRef.current = editor; }} />
      </div>
    </Card>
  );
}
