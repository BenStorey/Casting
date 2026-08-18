import { useEffect, useState } from "react";
import { getAuthToken, setAuthToken, clearAuthToken, hasAuthToken } from "./api";
import { useCastStore } from "./store";
import { tabLabel, type Tab } from "./nav";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";

function ago(ms: number): string {
  const s = Math.floor((Date.now() - ms) / 1000);
  if (s < 5) return "just now";
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${m % 60}m ago`;
  return `${Math.floor(h / 24)}d ago`;
}

/** The owner cluster — connection health + owner identity + auth control. */
export function HeaderRight() {
  const streamConnected = useCastStore((s) => s.streamConnected);
  const reconnects = useCastStore((s) => s.reconnects);
  const lastEventAt = useCastStore((s) => s.lastEventAt);
  const lastRefreshAt = useCastStore((s) => s.lastRefreshAt);
  const refresh = useCastStore((s) => s.refresh);
  const [tokenInput, setTokenInput] = useState("");
  const [, setTick] = useState(0);

  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 5000);
    return () => clearInterval(id);
  }, []);

  const hasEvent = lastEventAt > 0;
  const eventStale = hasEvent && Date.now() - lastEventAt > 60000;
  const dotClass = !streamConnected ? "red" : eventStale ? "amber" : "green";
  const statusTitle = !streamConnected
    ? "Stream disconnected"
    : eventStale
    ? `Stream up but idle · last event ${ago(lastEventAt)}`
    : `Live · last event ${ago(lastEventAt)}` +
      (reconnects > 0 ? ` · reconnected ${reconnects}x` : "");

  return (
    <div className="ml-auto flex items-center gap-3">
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="flex cursor-default items-center gap-1.5 text-xs text-muted-foreground">
            <span className={`status-dot ${dotClass}`} />
            {streamConnected ? (eventStale ? "idle" : "live") : "down"}
          </span>
        </TooltipTrigger>
        <TooltipContent>
          {statusTitle}
          {lastRefreshAt > 0 ? ` · refreshed ${ago(lastRefreshAt)}` : ""}
        </TooltipContent>
      </Tooltip>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button className="flex items-center gap-2 rounded-lg p-1 transition-colors hover:bg-accent">
            <Avatar className="h-8 w-8">
              <AvatarImage src="" alt="Owner" />
              <AvatarFallback>👤</AvatarFallback>
            </Avatar>
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-56">
          <DropdownMenuLabel>Owner</DropdownMenuLabel>
          <DropdownMenuSeparator />
          {hasAuthToken() ? (
            <DropdownMenuItem
              className="text-destructive focus:text-destructive"
              onClick={() => {
                clearAuthToken();
                refresh();
              }}
            >
              Clear owner token
            </DropdownMenuItem>
          ) : (
            <div className="flex items-center gap-1 p-1.5">
              <Input
                type="password"
                placeholder="owner token…"
                className="h-8 text-xs"
                value={tokenInput}
                onChange={(e) => setTokenInput(e.target.value)}
              />
              <Button
                size="sm"
                className="h-8 shrink-0 px-2 text-xs"
                onClick={() => {
                  if (tokenInput.trim()) {
                    setAuthToken(tokenInput.trim());
                    setTokenInput("");
                    refresh();
                  }
                }}
              >
                Set
              </Button>
            </div>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

interface ShellHeaderProps {
  tab: Tab;
}

export function ShellHeader({ tab }: ShellHeaderProps) {
  return (
    <header className="app-header">
      <h1>{tabLabel(tab)}</h1>
      <HeaderRight />
    </header>
  );
}
