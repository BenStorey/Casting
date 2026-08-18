// Application navigation — the grouped IA for the Casting UI.
// Retires the old 12-flat-tab bar in favour of a hierarchy grouped by what
// the owner does: daily surfaces first, internals tucked under ADVANCED.
// Each tab maps to a real URL (react-router), so surfaces are deep-linkable.
import {
  Activity,
  BookOpen,
  Brain,
  CreditCard,
  GitBranch,
  Home,
  Inbox,
  MessageSquare,
  PenTool,
  Scale,
  Settings,
  SlidersHorizontal,
  Sparkles,
  SquareKanban,
  TerminalSquare,
  Users,
  type LucideIcon,
} from "lucide-react";

export type Tab =
  | "home"
  | "inbox"
  | "chat"
  | "board"
  | "graph"
  | "team"
  | "activity"
  | "decisions"
  | "knowledge"
  | "spend"
  | "advisor"
  | "sketch"
  | "settings"
  | "routing"
  | "debug";

export interface NavItem {
  key: Tab;
  label: string;
  icon: LucideIcon;
  /** URL path for this surface. */
  path: string;
}

export interface NavGroup {
  label: string;
  items: NavItem[];
}

export const NAV_GROUPS: NavGroup[] = [
  {
    label: "Home",
    items: [
      { key: "home", label: "Home", icon: Home, path: "/" },
      { key: "inbox", label: "Inbox", icon: Inbox, path: "/inbox" },
      { key: "chat", label: "Chat", icon: MessageSquare, path: "/chat" },
    ],
  },
  {
    label: "Work",
    items: [
      { key: "board", label: "Board", icon: SquareKanban, path: "/board" },
      { key: "graph", label: "Graph", icon: GitBranch, path: "/graph" },
      { key: "team", label: "Cast", icon: Users, path: "/cast" },
      { key: "activity", label: "History", icon: Activity, path: "/history" },
    ],
  },
  {
    label: "Tools",
    items: [
      { key: "advisor", label: "Advisor", icon: Brain, path: "/advisor" },
      { key: "sketch", label: "Sketch", icon: PenTool, path: "/sketch" },
    ],
  },
  {
    label: "Review",
    items: [
      { key: "decisions", label: "Decisions", icon: Scale, path: "/decisions" },
      { key: "knowledge", label: "Knowledge", icon: BookOpen, path: "/knowledge" },
      { key: "spend", label: "Spend", icon: CreditCard, path: "/spend" },
    ],
  },
  {
    label: "Settings",
    items: [
      { key: "settings", label: "Setup & Connect", icon: Settings, path: "/settings" },
      { key: "routing", label: "Model routing", icon: SlidersHorizontal, path: "/routing" },
    ],
  },
  {
    label: "Advanced",
    items: [{ key: "debug", label: "Debug", icon: TerminalSquare, path: "/debug" }],
  },
];

/** Flatten all nav keys for quick lookups. */
export const ALL_TABS: Tab[] = NAV_GROUPS.flatMap((g) => g.items.map((i) => i.key));

/** The default landing surface after setup. */
export const DEFAULT_TAB: Tab = "home";

/** Map a tab to its URL path (used when navigating). */
export function pathForTab(tab: Tab): string {
  for (const g of NAV_GROUPS) {
    const it = g.items.find((i) => i.key === tab);
    if (it) return it.path;
  }
  return "/";
}

/** Resolve a URL path to a tab; unknown paths fall back to the default. */
export function tabForPath(path: string): Tab {
  const clean = path.startsWith("/") ? path : `/${path}`;
  for (const g of NAV_GROUPS) {
    for (const it of g.items) {
      if (it.path === clean) return it.key;
    }
  }
  return DEFAULT_TAB;
}

/** Human label for a tab (for the header breadcrumb). */
export function tabLabel(tab: Tab): string {
  for (const g of NAV_GROUPS) {
    const it = g.items.find((i) => i.key === tab);
    if (it) return it.label;
  }
  return tab;
}

/** Icon for a tab (header / misc). */
export function tabIcon(tab: Tab): LucideIcon {
  for (const g of NAV_GROUPS) {
    const it = g.items.find((i) => i.key === tab);
    if (it) return it.icon;
  }
  return Sparkles;
}
