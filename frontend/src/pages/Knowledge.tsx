import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import type { OperatingModel } from "@/api";

interface KnowledgeProps {
  model: OperatingModel | null;
}

/** The "company brain" — facts, opinions, risks, and briefings the cast operates on. */
export default function Knowledge({ model }: KnowledgeProps) {
  const k = model?.knowledge;
  const risks = model?.context?.open_risks ?? [];

  return (
    <div className="grid gap-4 md:grid-cols-2">
      <Card>
        <CardHeader className="pb-2">
          <CardTitle>Facts</CardTitle>
          <CardDescription>Established, agreed facts about the company and project.</CardDescription>
        </CardHeader>
        <CardContent className="pt-2">
          {!k || k.facts.length === 0 ? (
            <div className="empty">
              <div className="icon">📌</div>
              <div className="title">No facts recorded yet</div>
            </div>
          ) : (
            <ul className="space-y-1 text-sm">
              {k.facts.map((f, i) => (
                <li key={i} className="flex gap-2">
                  <span className="text-primary shrink-0">•</span>
                  <span>{f}</span>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-2">
          <CardTitle>Opinions</CardTitle>
          <CardDescription>Active positions the cast holds. Superseded ones are tracked too.</CardDescription>
        </CardHeader>
        <CardContent className="pt-2">
          {!k || (k.opinions.length === 0 && k.superseded_opinions.length === 0) ? (
            <div className="empty">
              <div className="icon">💭</div>
              <div className="title">No opinions yet</div>
            </div>
          ) : (
            <>
              <div className="space-y-1 text-sm">
                {k?.opinions.map((o, i) => (
                  <div key={i} className="flex gap-2">
                    <Badge variant="soft">active</Badge>
                    <span>{o}</span>
                  </div>
                ))}
              </div>
              {k && k.superseded_opinions.length > 0 && (
                <>
                  <Separator className="my-3" />
                  <div className="space-y-1 text-sm text-muted-foreground">
                    <div className="overline-label">Superseded</div>
                    {k.superseded_opinions.map((o, i) => (
                      <div key={i} className="line-through opacity-70">{o}</div>
                    ))}
                  </div>
                </>
              )}
            </>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-2">
          <CardTitle>Risks</CardTitle>
          <CardDescription>Open risks the cast is tracking.</CardDescription>
        </CardHeader>
        <CardContent className="pt-2">
          {risks.length === 0 ? (
            <div className="empty">
              <div className="icon">⚠️</div>
              <div className="title">No open risks</div>
            </div>
          ) : (
            <ul className="space-y-1 text-sm">
              {risks.map((r, i) => (
                <li key={i} className="flex gap-2">
                  <span className="text-destructive shrink-0">⚠</span>
                  <span>{r}</span>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-2">
          <CardTitle>Briefings</CardTitle>
          <CardDescription>Advisor briefings brought into the company.</CardDescription>
        </CardHeader>
        <CardContent className="pt-2">
          {!k || k.briefings.active.length === 0 ? (
            <div className="empty">
              <div className="icon">📋</div>
              <div className="title">No active briefings</div>
            </div>
          ) : (
            <ul className="space-y-1 text-sm">
              {k.briefings.active.map((b, i) => (
                <li key={i} className="flex gap-2">
                  <span className="text-primary shrink-0">•</span>
                  <span>{b}</span>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
