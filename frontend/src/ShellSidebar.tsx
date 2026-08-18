import { NavLink } from "react-router-dom";
import { NAV_GROUPS } from "./nav";
import { Badge } from "@/components/ui/badge";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { PanelLeftClose, PanelLeftOpen } from "lucide-react";
import { cn } from "@/lib/utils";

interface ShellSidebarProps {
  active: string;
  collapsed: boolean;
  onToggle: () => void;
  unread: number;
  /** Mobile: open/close state of the off-canvas drawer. */
  mobileOpen: boolean;
  onCloseMobile: () => void;
}

export function ShellSidebar({
  active,
  collapsed,
  onToggle,
  unread,
  mobileOpen,
  onCloseMobile,
}: ShellSidebarProps) {
  const items = NAV_GROUPS.flatMap((g) => g.items);

  return (
    <>
      {/* Mobile backdrop */}
      <div
        className={cn(
          "fixed inset-0 z-40 bg-foreground/25 backdrop-blur-sm transition-opacity lg:hidden",
          mobileOpen ? "opacity-100" : "pointer-events-none opacity-0"
        )}
        onClick={onCloseMobile}
        aria-hidden="true"
      />

      <aside
        className={cn(
          "app-sidebar",
          collapsed ? "collapsed" : "",
          "mobile-fixed",
          mobileOpen && "mobile-open"
        )}
      >
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
                const label = (
                  <>
                    <Icon className="nav-icon" />
                    {!collapsed && (
                      <>
                        <span className="flex-1 text-left">{item.label}</span>
                        {item.key === "inbox" && unread > 0 && (
                          <Badge className="px-1.5">{unread}</Badge>
                        )}
                      </>
                    )}
                  </>
                );
                return (
                  <NavLink
                    key={item.key}
                    to={item.path}
                    onClick={onCloseMobile}
                    end={item.path === "/"}
                    className={cn("nav-item", isActive && "active")}
                  >
                    {collapsed ? (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <span className="contents">{label}</span>
                        </TooltipTrigger>
                        <TooltipContent side="right">
                          {item.label}
                          {item.key === "inbox" && unread > 0 ? ` (${unread})` : ""}
                        </TooltipContent>
                      </Tooltip>
                    ) : (
                      label
                    )}
                  </NavLink>
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

      <span data-active-tab={active} data-items={items.length} hidden />
    </>
  );
}
