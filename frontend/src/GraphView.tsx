// Graph — the derived graph/transition view (/api/graph).
//
// Renders the explicit task lifecycle: parallel-work groups (join points),
// per-node state + assignee + currently-available transitions, and the causal
// chain ("why in this order"). Everything here is a DERIVED read of the event
// log served by the backend — this view never re-derives projection state.
import { GraphGroup, GraphNode, GraphTaskState } from "./api";
import type { GraphView as GraphViewData } from "./api";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

const STATE_LABEL: Record<GraphTaskState, string> = {
  queued: "queued",
  working: "working",
  in_review: "in review",
  awaiting_human: "awaiting human",
  rejected: "rejected — rework",
  done: "done",
};

const STATE_VARIANT: Record<
  GraphTaskState,
  "default" | "secondary" | "destructive" | "outline"
> = {
  queued: "outline",
  working: "default",
  in_review: "secondary",
  awaiting_human: "destructive",
  rejected: "destructive",
  done: "outline",
};

const TRANSITION_LABEL: Record<string, string> = {
  assign: "assign",
  start: "start",
  submit: "submit",
  approve: "approve",
  reject: "request changes",
  block: "escalate",
  decompose: "decompose",
  resume: "resume",
  rework: "rework",
};

function NodeCard({ node, highlightChildrenOf }: { node: GraphNode; highlightChildrenOf?: string | null }) {
  return (
    <Card className={node.awaiting_human ? "border-destructive/60" : "border-border/60"}>
      <CardContent className="p-3">
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0">
            <div className="truncate text-sm font-medium leading-snug">{node.title}</div>
            <div className="text-xs text-muted-foreground truncate">
              {node.task_id}
              {node.assignee ? ` · ${node.assignee}` : ""}
            </div>
          </div>
          <Badge variant={STATE_VARIANT[node.state]} className="shrink-0">
            {STATE_LABEL[node.state]}
          </Badge>
        </div>

        {highlightChildrenOf === node.task_id && (
          <div className="mt-2 text-xs text-primary">⊢ join point · {node.children.length} parallel subtask(s)</div>
        )}

        {node.blocked_by.length > 0 && (
          <div className="mt-1 text-[11px] font-medium text-amber-600">
            ⏳ waiting on: {node.blocked_by.join(", ")}
          </div>
        )}

        {node.transitions.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-1">
            {node.transitions.map((t) => (
              <span key={t} className="rounded border border-border px-1.5 py-0.5 text-[11px] text-muted-foreground">
                {TRANSITION_LABEL[t] ?? t}
              </span>
            ))}
          </div>
        )}

        {node.chain.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-x-1 gap-y-0.5 text-[11px] text-muted-foreground">
            {node.chain.map((step, i) => (
              <span key={i} className="flex items-center gap-1">
                {i > 0 && <span className="text-primary">→</span>}
                <span>{step}</span>
              </span>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function GroupCard({ group, nodesById }: { group: GraphGroup; nodesById: Map<string, GraphNode> }) {
  return (
    <Card>
      <CardContent className="pt-4">
        <div className="mb-2 flex items-center justify-between gap-2">
          <div className="font-semibold text-sm">{group.title}</div>
          <Badge variant={group.resolved ? "outline" : "default"}>
            {group.resolved ? "joined ✓" : `${group.done.length}/${group.children.length} done`}
          </Badge>
        </div>
        {!group.resolved && group.remaining.length > 0 && (
          <div className="mb-2 text-xs text-muted-foreground">
            blocked by: {group.remaining.join(", ")}
          </div>
        )}
        <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
          {group.children.map((id) => {
            const node = nodesById.get(id);
            return node ? <NodeCard key={id} node={node} /> : null;
          })}
        </div>
      </CardContent>
    </Card>
  );
}

export default function GraphView({ graph }: { graph: GraphViewData | null }) {
  if (!graph) return null;
  const nodesById = new Map(graph.nodes.map((n) => [n.task_id, n]));

  // Root tasks = no parent. Those that are join points render as Groups; the
  // rest render as standalone root cards.
  const roots = graph.nodes.filter((n) => n.parent_id === null);
  const groupIds = new Set(graph.groups.map((g) => g.parent_id));
  const standaloneRoots = roots.filter((n) => !groupIds.has(n.task_id));

  return (
    <div className="grid gap-4">
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">Flow graph</CardTitle>
          <CardDescription>
            Derived from the event log — states, parallel-work joins, and what's waiting on you.
          </CardDescription>
        </CardHeader>
        <CardContent className="pt-2 flex flex-wrap gap-2">
          <Badge variant="outline">{graph.total} total</Badge>
          <Badge variant="outline">{graph.done} done</Badge>
          <Badge variant="default">{graph.active.length} active</Badge>
          <Badge variant={graph.blocked.length > 0 ? "destructive" : "secondary"}>
            {graph.blocked.length} awaiting human
          </Badge>
        </CardContent>
      </Card>

      {graph.blocked.length > 0 && (
        <Card className="border-destructive/50">
          <CardHeader className="pb-2">
            <CardTitle className="text-base text-destructive">Waiting on you</CardTitle>
          </CardHeader>
          <CardContent className="pt-2 grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
            {graph.blocked.map((id) => {
              const node = nodesById.get(id);
              return node ? <NodeCard key={id} node={node} /> : null;
            })}
          </CardContent>
        </Card>
      )}

      {graph.groups.map((g) => (
        <GroupCard key={g.parent_id} group={g} nodesById={nodesById} />
      ))}

      {standaloneRoots.length > 0 && (
        <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
          {standaloneRoots.map((n) => (
            <NodeCard key={n.task_id} node={n} />
          ))}
        </div>
      )}

      {graph.groups.length === 0 && standaloneRoots.length === 0 && (
        <Card className="muted">
          <CardContent className="py-6">No tasks on the board yet.</CardContent>
        </Card>
      )}
    </div>
  );
}
