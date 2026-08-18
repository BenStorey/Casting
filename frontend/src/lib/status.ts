// Shared status → style helpers + formatters for the Casting UI.
// Centralises the mapping from data values (priority, severity, task status,
// etc.) onto the design-system tokens, so every surface stays consistent.
import { type BadgeProps } from "@/components/ui/badge";

/** Priority → badge variant (low→muted, medium→amber, high→indigo, critical→red). */
export const PRIORITY_VARIANT: Record<string, BadgeProps["variant"]> = {
  critical: "destructive",
  high: "default",
  medium: "warning",
  low: "outline",
};

export function priorityVariant(p: string | undefined): BadgeProps["variant"] {
  return (p && PRIORITY_VARIANT[p]) || "secondary";
}

/** Observation / risk severity → badge variant. */
export const SEVERITY_VARIANT: Record<string, BadgeProps["variant"]> = {
  critical: "destructive",
  high: "destructive",
  medium: "warning",
  low: "secondary",
};

export function severityVariant(s: string | undefined): BadgeProps["variant"] {
  return (s && SEVERITY_VARIANT[s]) || "secondary";
}

/** Task status → badge variant. */
export const TASK_STATUS_VARIANT: Record<string, BadgeProps["variant"]> = {
  done: "success",
  in_review: "default",
  working: "soft",
  blocked: "destructive",
  backlog: "outline",
};

export function taskStatusVariant(s: string | undefined): BadgeProps["variant"] {
  return (s && TASK_STATUS_VARIANT[s]) || "secondary";
}

/** Decision status → badge variant. */
export const DECISION_STATUS_VARIANT: Record<string, BadgeProps["variant"]> = {
  approved: "success",
  proposed: "warning",
  rejected: "destructive",
  superseded: "outline",
};

export function decisionStatusVariant(s: string | undefined): BadgeProps["variant"] {
  return (s && DECISION_STATUS_VARIANT[s]) || "secondary";
}

/** Format USD to 4 decimals below $1, 2 above — enough precision for metering. */
export function usd(n: number): string {
  if (n > 0 && n < 1) return `$${n.toFixed(4)}`;
  return `$${n.toFixed(2)}`;
}

/** Compact money: $12.40k / $3.1m. */
export function usdCompact(n: number): string {
  if (Math.abs(n) >= 1_000_000) return `$${(n / 1_000_000).toFixed(2)}m`;
  if (Math.abs(n) >= 1_000) return `$${(n / 1_000).toFixed(1)}k`;
  return usd(n);
}

/** Token count with thousands separators. */
export function tokens(n: number): string {
  return n.toLocaleString("en-US");
}

/** Friendly "5m ago" / "3h ago" style relative time from an epoch ms. */
export function ago(ms: number): string {
  const s = Math.floor((Date.now() - ms) / 1000);
  if (s < 5) return "just now";
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${m % 60}m ago`;
  return `${Math.floor(h / 24)}d ${h % 24}h ago`;
}

/** Format an ISO timestamp to a short local date-time. */
export function fmtTime(iso: string): string {
  const d = new Date(iso);
  return isNaN(d.getTime()) ? "" : d.toLocaleString();
}
