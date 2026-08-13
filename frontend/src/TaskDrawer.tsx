// TaskDrawer — per-task drill-down (G7).
//
// One click on a board/task card opens a detail drawer pulling together every
// diagnostic slice about that task: its provenance ("why does this code
// exist"), its graph node (state, transitions, blockers), its isolated
// worktree, and its spend entries. For "why is task-7 stuck?" it's all in one
// place instead of a manual provenance lookup.
import { useEffect, useState } from "react";
import { useCastStore } from "./store";
import { fetchTaskProvenance, Task } from "./api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

interface Props {
  task: Task;
  onClose: () => void;
}

export default function TaskDrawer({ task, onClose }: Props) {
  const graph = useCastStore((s) => s.graph);
  const model = useCastStore((s) => s.model);
  const [prov, setProv] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    fetchTaskProvenance(task.id)
      .then((p) => alive && setProv(p as Record<string, unknown>))
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [task.id]);

  const node = graph?.nodes.find((n) => n.task_id === task.id) ?? null;
  const worktree = model?.worktrees.find((w) => w.task_id === task.id) ?? null;
  const blockedBy = node?.blocked_by ?? [];
  const chain = node?.chain ?? [];
  const transitions = node?.transitions ?? [];
  const assigneeSpend =
    task.assignee && model?.spend.by_agent[task.assignee] !== undefined
      ? model.spend.by_agent[task.assignee]
      : null;

  return (
    <div className="fixed inset-0 z-50 bg-black/40" onClick={onClose}>
      <div
        className="absolute inset-y-0 right-0 w-full max-w-lg overflow-y-auto bg-background shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="sticky top-0 flex items-start justify-between gap-2 border-b bg-background p-4">
          <div className="min-w-0">
            <div className="text-xs text-muted-foreground">{task.id}</div>
            <CardTitle className="break-words leading-snug">{task.title}</CardTitle>
            <div className="mt-1 flex flex-wrap gap-1.5">
              <Badge variant="outline">{task.kind}</Badge>
              <Badge variant={task.status === "blocked" ? "destructive" : "secondary"}>
                {task.status}
              </Badge>
              {node && <Badge variant="outline">state: {node.state}</Badge>}
            </div>
          </div>
          <Button size="sm" variant="ghost" onClick={onClose}>✕</Button>
        </div>

        <div className="space-y-3 p-4">
          {blockedBy.length > 0 && (
            <Card className="border-amber-500/50">
              <CardContent className="pt-4 text-sm">
                <span className="font-medium text-amber-600">⏳ waiting on: </span>
                {blockedBy.join(", ")}
              </CardContent>
            </Card>
          )}
          {chain.length > 0 && (
            <Card>
              <CardHeader className="pb-1"><CardTitle className="text-sm">Causal chain</CardTitle></CardHeader>
              <CardContent className="pt-0 text-xs text-muted-foreground">
                {chain.join(" → ")}
              </CardContent>
            </Card>
          )}
          {transitions.length > 0 && (
            <Card>
              <CardHeader className="pb-1"><CardTitle className="text-sm">Available transitions</CardTitle></CardHeader>
              <CardContent className="pt-0 flex flex-wrap gap-1">
                {transitions.map((t) => (
                  <span key={t} className="rounded border border-border px-1.5 py-0.5 text-[11px]">{t}</span>
                ))}
              </CardContent>
            </Card>
          )}
          {worktree && (
            <Card>
              <CardHeader className="pb-1"><CardTitle className="text-sm">Isolated worktree</CardTitle></CardHeader>
              <CardContent className="pt-0 text-xs">
                <div>branch: <code className="text-primary">{worktree.branch}</code></div>
                <div>port: :{worktree.port}</div>
                <div className="text-muted-foreground break-all">path: {worktree.path}</div>
              </CardContent>
            </Card>
          )}
          {assigneeSpend !== null && task.status === "done" && (
            <Card>
              <CardHeader className="pb-1"><CardTitle className="text-sm">Spend</CardTitle></CardHeader>
              <CardContent className="pt-0 text-xs text-muted-foreground">
                task spend attributed to {task.assignee}: ${assigneeSpend.toFixed(4)}
              </CardContent>
            </Card>
          )}

          <Card>
            <CardHeader className="pb-1">
              <CardTitle className="text-sm">Provenance</CardTitle>
              <CardDescription>why does this code exist?</CardDescription>
            </CardHeader>
            <CardContent className="pt-0">
              {error && <div className="text-sm text-destructive">{error}</div>}
              {!prov && !error && <div className="text-sm text-muted-foreground">Loading…</div>}
              {prov && (
                <pre className="overflow-auto rounded bg-muted p-3 text-xs">{JSON.stringify(prov, null, 2)}</pre>
              )}
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
