import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ADVISOR_IDENTITY } from "./identities";
import { Message, handoffAdvisor, sendToAdvisor, summarizeAdvisor } from "./api";
import { useCastStore } from "./store";

/// The Direction Advisor — a special second role the owner talks to directly.
/// This chat is ISOLATED from the PM's context by design: you think freely, and
/// only when you choose to "hand off to the PM" does the conversation become a
/// Briefing the PM reads (advisory, provenanced "advisor"). The advisor replies
/// are D2 (LLM) when the model layer is configured; otherwise the seam records
/// your side and you hand off manually.

export default function Advisor({ thread, onChanged }: { thread: Message[]; onChanged: () => void }) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  // The advisor's LLM spend (from the operating picture's per-agent breakdown).
  // 0 / absent = the LLM layer isn't configured, so replies are deterministic.
  const advisorSpend = useCastStore(
    (s) => s.model?.spend.by_agent["advisor"] ?? 0
  );
  const advisorReplies = thread.filter((m) => m.from === "advisor").length;
  const llmActive = advisorSpend > 0 || advisorReplies > 0;

  const send = async () => {
    if (!draft.trim()) return;
    setBusy(true);
    setMsg(null);
    try {
      await sendToAdvisor(draft.trim());
      setDraft("");
      onChanged();
    } catch (e) {
      setMsg(`⚠️ ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const handoff = async () => {
    setBusy(true);
    setMsg(null);
    try {
      // Ask the LLM to distill the conversation into a briefing (falls back to a
      // deterministic summarizer server-side when no LLM is configured).
      let summary = "";
      try {
        summary = (await summarizeAdvisor()).summary;
      } catch {
        summary = summarizer(thread); // local deterministic fallback
      }
      await handoffAdvisor(summary || "Advisor conversation handed off to PM", summarizer(thread));
      setDraft("");
      setMsg("✅ Handed off to the PM as an advisor briefing");
      onChanged();
    } catch (e) {
      setMsg(`⚠️ ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-3">
          <img src={ADVISOR_IDENTITY.avatar ?? ""} alt={ADVISOR_IDENTITY.name} className="h-12 w-12 rounded-xl" />
          <div>
            <CardTitle className="text-base">{ADVISOR_IDENTITY.name}</CardTitle>
            <CardDescription>
              {ADVISOR_IDENTITY.role} — {ADVISOR_IDENTITY.persona}
            </CardDescription>
          </div>
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-xs text-muted-foreground leading-relaxed">
          This chat is private — it doesn't reach the PM until you hand it off. Think freely about
          product direction; when you're ready for the company to act, summarize and hand off.
        </p>
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          {llmActive ? (
            <Badge variant="secondary">🟢 LLM advisor active</Badge>
          ) : (
            <Badge variant="outline">⚪ Deterministic (no LLM configured)</Badge>
          )}
          <span>{advisorReplies} reply{advisorReplies === 1 ? "" : "s"}</span>
          {advisorSpend > 0 && <span>· ${advisorSpend.toFixed(4)} spend</span>}
        </div>
        <div className="thread max-h-[40vh] overflow-y-auto space-y-2">
          {thread.length === 0 && (
            <div className="text-sm text-muted-foreground">A fresh page — what are you thinking about?</div>
          )}
          {thread.map((m) => (
            <div key={m.id} className={"row flex gap-2 text-sm " + (m.from === "owner" ? "justify-end" : "")}>
              <span className="seq text-xs text-muted-foreground whitespace-nowrap">{m.from}</span>
              <span className={"bubble rounded-lg px-3 py-1.5 " + (m.from === "owner" ? "bg-primary/15" : "bg-muted")}>
                {m.to === "advisor" && m.from === "owner" ? "💬 " : ""}{m.body}
              </span>
            </div>
          ))}
        </div>
        <div className="flex gap-2">
          <Input
            value={draft}
            placeholder='e.g. "Should the product be open-core or closed?"'
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && !busy && void send()}
          />
          <Button size="sm" onClick={() => void send()} disabled={busy}>
            Send
          </Button>
        </div>
        <Button variant="secondary" size="sm" className="w-full" onClick={() => void handoff()} disabled={busy}>
          ➡️ Summarize this and hand off to the PM
        </Button>
        {msg && <div className="text-xs text-muted-foreground">{msg}</div>}
      </CardContent>
    </Card>
  );
}

/// A quick deterministic "summary": the session as headings. (Full summarization is D2.)
function summarizer(thread: Message[]): string {
  if (thread.length === 0) return "Advisor conversation handed off to PM.";
  const owners = thread.map((m) => m.body).join("; ");
  return `Advisor session — owner's thinking: ${owners}`;
}
