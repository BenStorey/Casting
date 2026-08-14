import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import {
  fetchTelegramStatus,
  configureTelegram,
  TelegramStatus,
  TelegramConfigureResult,
} from "./api";

/// The "connect your messaging" panel (2026-08-14): guides a Casting user
/// through creating their own Telegram bot via @BotFather, pasting the token,
/// DM-ing their bot, and lets the server brand it as the PM + auto-learn the
/// chat_id. Reusable so it can appear in the setup wizard AND later in a
/// Settings panel — every Casting install configures its OWN bot (never shared).
export default function TelegramConnect() {
  const [status, setStatus] = useState<TelegramStatus | null>(null);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [result, setResult] = useState<TelegramConfigureResult | null>(null);

  const refresh = () =>
    fetchTelegramStatus().then(setStatus).catch((e: unknown) => setErr(String(e)));
  useEffect(() => {
    refresh();
  }, []);

  const connect = async () => {
    if (!token.trim()) return;
    setBusy(true);
    setErr(null);
    setMsg(null);
    try {
      const r = await configureTelegram(token.trim());
      setResult(r);
      setMsg(
        r.chat_linked
          ? `Connected as @${r.bot_username} — chat linked. You can now message your PM from Telegram.`
          : `Validated @${r.bot_username} and branded it as the PM — but no chat is linked yet. Open @${r.bot_username} in Telegram and send it a message (e.g. /start), then hit "I've messaged it".`
      );
      refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const linked = result?.chat_linked || status?.configured || false;

  return (
    <div className="space-y-3">
      {linked ? (
        <div className="rounded-lg border border-green-500/40 bg-green-500/10 p-3 text-sm">
          ✅ Connected
          {status?.bot_username && <strong> @{status.bot_username}</strong>} — you can
          message your PM from Telegram. (Each Casting install uses its own bot.)
        </div>
      ) : (
        <ol className="list-decimal list-inside space-y-1 text-sm text-muted-foreground">
          <li>
            In Telegram, open <strong>@BotFather</strong> and send{" "}
            <code>/newbot</code>. Pick any name + username.
          </li>
          <li>Copy the token it gives you and paste it below.</li>
          <li>
            Press <strong>Connect</strong>, then send your bot a message (e.g.{" "}
            <code>/start</code>) so it learns who you are.
          </li>
        </ol>
      )}

      <div className="flex gap-2">
        <Input
          value={token}
          onChange={(e) => setToken(e.target.value)}
          placeholder="paste your BotFather token (1234:AA…)"
          className="font-mono text-sm"
        />
        <Button onClick={() => void connect()} disabled={busy || !token.trim()}>
          {busy ? "Connecting…" : linked ? "Reconnect" : "Connect"}
        </Button>
        {!linked && (
          <Button
            variant="outline"
            onClick={() => {
              setResult(null);
              void connect();
            }}
            disabled={busy || !result?.bot_username}
          >
            I've messaged it
          </Button>
        )}
      </div>

      {msg && <div className="text-sm text-green-600">{msg}</div>}
      {err && <div className="text-sm text-red-600">{err}</div>}
      {result?.chat_id != null && (
        <div className="text-xs text-muted-foreground">
          <Badge variant="secondary">bot {result.bot_id}</Badge>
          <Badge variant="secondary" className="ml-1">
            chat {result.chat_id}
          </Badge>
        </div>
      )}
      {!linked && (
        <p className="text-xs text-muted-foreground">
          You can skip this — it's optional. You can always connect messaging
          later from settings.
        </p>
      )}
    </div>
  );
}
