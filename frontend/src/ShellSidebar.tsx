import { NAV_GROUPS, type Tab } from "./nav";
import { Badge } from "@/components/ui/badge";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { PanelLeftClose, PanelLeftOpen } from "lucide-react";

interface ShellSidebarProps {
  active: Tab;
  onNavigate: (t: Tab) => void;
  collapsed: boolean;
  onToggle: () => void;
  unread: number;
}

export function ShellSidebar({
  active,
  onNavigate,
  collapsed,
  onToggle,
  unread,
}: ShellSidebarProps) {
  return (
    <aside className={`app-sidebar ${collapsed ? "collapsed" : ""}`}>
      <div className="sidebar-brand">
        <span className="logo">🎬</span>
        {!collapsed && <span className="name">Casting</span>}
      </div>

      {NAV_GROUPS.map((group) => (
        <div key={group.label}>
          {!collapsed && <div className="sidebar-group-label">{group.label}</div>}
          <div className="flex flex-col gap-0.5">
            {group.items.map((item) => {
              const Icon = item.icon;
              const isActive = active === item.key;
              const content = (
                <button
                  className={`nav-item ${isActive ? "active" : ""}`}
                  onClick={() => onNavigate(item.key)}
                  aria-current={isActive ? "page" : undefined}
                >
                  <Icon className="nav-icon" />
                  {!collapsed && (
                    <>
                      <span className="flex-1 text-left">{item.label}</span>
                      {item.key === "inbox" && unread > 0 && (
                        <Badge className="px-1.5">{unread}</Badge>
                      )}
                    </>
                  )}
                </button>
              );
              return collapsed ? (
                <Tooltip key={item.key}>
                  <TooltipTrigger asChild>{content}</TooltipTrigger>
                  <TooltipContent side="right">
                    {item.label}
                    {item.key === "inbox" && unread > 0 ? ` (${unread})` : ""}
                  </TooltipContent>
                </Tooltip>
              ) : (
                <div key={item.key}>{content}</div>
              );
            })}
          </div>
        </div>
      ))}

      <div className="sidebar-footer">
        <button className="nav-item mx-0" onClick={onToggle} title="Toggle sidebar">
          {collapsed ? (
            <PanelLeftOpen className="nav-icon" />
          ) : (
            <PanelLeftClose className="nav-icon" />
          )}
          {!collapsed && <span className="flex-1 text-left">Collapse</span>}
        </button>
      </div>
    </aside>
  );
}
