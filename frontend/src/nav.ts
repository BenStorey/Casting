// Application navigation — the grouped IA for the Casting UI.
// Retires the old 12-flat-tab bar in favour of a hierarchy grouped by what
// the owner does: daily surfaces first, internals tucked under ADVANCED.
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
}

export interface NavGroup {
  label: string;
  items: NavItem[];
}

export const NAV_GROUPS: NavGroup[] = [
  {
    label: "Home",
    items: [
      { key: "home", label: "Home", icon: Home },
      { key: "inbox", label: "Inbox", icon: Inbox },
      { key: "chat", label: "Chat", icon: MessageSquare },
    ],
  },
  {
    label: "Work",
    items: [
      { key: "board", label: "Board", icon: SquareKanban },
      { key: "graph", label: "Graph", icon: GitBranch },
      { key: "team", label: "Cast", icon: Users },
      { key: "activity", label: "History", icon: Activity },
    ],
  },
  {
    label: "Tools",
    items: [
      { key: "advisor", label: "Advisor", icon: Brain },
      { key: "sketch", label: "Sketch", icon: PenTool },
    ],
  },
  {
    label: "Review",
    items: [
      { key: "decisions", label: "Decisions", icon: Scale },
      { key: "knowledge", label: "Knowledge", icon: BookOpen },
      { key: "spend", label: "Spend", icon: CreditCard },
    ],
  },
  {
    label: "Settings",
    items: [
      { key: "settings", label: "Setup & Connect", icon: Settings },
      { key: "routing", label: "Model routing", icon: SlidersHorizontal },
    ],
  },
  {
    label: "Advanced",
    items: [{ key: "debug", label: "Debug", icon: TerminalSquare }],
  },
];

/** Flatten all nav keys for quick lookups. */
export const ALL_TABS: Tab[] = NAV_GROUPS.flatMap((g) => g.items.map((i) => i.key));

/** The default landing surface after setup. */
export const DEFAULT_TAB: Tab = "home";

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
