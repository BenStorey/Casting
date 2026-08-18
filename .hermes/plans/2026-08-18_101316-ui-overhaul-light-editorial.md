# Casting UI Overhaul — Light Editorial Studio

> **For Hermes:** read this entire plan before implementing. Treat the "Design Tokens" section as the source of truth for the new theme. Treat the Information Architecture section as the new page map.

**Goal:** Replace the current "dirty hacky" 12-flat-tab, dark-cockpit, wall-of-text UI with a polished, light, editorial-studio design that is ready for real (including non-technical) end users. Ship a redesigned setup wizard and a proper dashboard with sensible, grouped navigation.

**Architecture:** Keep React 19 + Vite 8 + Tailwind 4 (CSS-first `@theme`) + shadcn/ui. Restructure the app shell from a single crowded top tab-bar into a **grouped left sidebar + content area** with a sticky header. Introduce a real design-token layer (color, type, spacing, radius, shadow) so the whole app re-themes consistently. No new heavy frontend deps.

**Tech Stack:** React 19, TypeScript 7, Vite 8, Tailwind 4 (`@theme` oklch tokens), shadcn/ui, lucide-react, zustand. (Already present — no installs beyond adding a few shadcn primitives like `separator`, `dropdown-menu`, `avatar`, `scroll-area`, `switch`, `progress`, `tooltip`.)

**Design personae (person is the "Director/Owner"):**
- Non-technical owner → needs a calm, unambiguous "what needs me + is it healthy" surface, not raw internals.
- Technical owner → wants spending, routing, governance, and the event history reachable, but *reachable*, not the landing view.
- Everyone → the 🎬 theatre/studio metaphor: "direct your cast."

---

## Part 1 — Design System (the tokens that make it pretty)

### 1.1 Colour palette (light editorial)
Warm, paper-like neutrals — never sterile pure-white or harsh graphite. One distinctive **indigo** primary (Casting blue, refined warmer/deeper), a **warm amber** accent used sparingly (spotlights, highlights), and clear status hues.

| Token | Value (oklch) | Rendered feel |
|---|---|---|
| `--background` | `oklch(0.985 0.004 90)` | warm paper |
| `--foreground` | `oklch(0.26 0.02 260)` | soft ink |
| `--card` | `oklch(1 0 0)` | crisp white surfaces |
| `--card-foreground` | = foreground | |
| `--popover` | `oklch(1 0 0)` | |
| `--popover-foreground` | = foreground | |
| `--primary` | `oklch(0.52 0.17 262)` | cinematic deep indigo |
| `--primary-foreground` | `oklch(0.985 0.004 262)` | white on indigo |
| `--primary-soft` (new) | `oklch(0.94 0.03 262)` | tinted indigo wash for selected/active rows |
| `--secondary` | `oklch(0.95 0.005 90)` | warm grey surface |
| `--secondary-foreground` | = foreground | |
| `--muted` | `oklch(0.955 0.005 90)` | subtler wash |
| `--muted-foreground` | `oklch(0.54 0.02 260)` | readable secondary text |
| `--accent` | `oklch(0.93 0.02 90)` | hover surface |
| `--accent-foreground` | = foreground | |
| `--destructive` | `oklch(0.58 0.21 25)` | refined red (not neon) |
| `--destructive-foreground` | `oklch(0.99 0.005 25)` | |
| `--warning` (new) | `oklch(0.79 0.15 76)` | amber "spotlight"/attention |
| `--warning-foreground` | `oklch(0.28 0.07 60)` | dark amber text |
| `--success` (new) | `oklch(0.62 0.15 150)` | calm green |
| `--success-foreground` | `oklch(0.99 0.005 150)` | |
| `--border` | `oklch(0.90 0.01 90)` | hairline warm-grey |
| `--input` | `oklch(0.90 0.01 90)` | |
| `--ring` | `oklch(0.52 0.17 262)` | focus = primary |

Semantic status → colour mapping (shared util): task/decision severity reuse `critical=destructive`, `high=primary`, `medium=warning`, `low=success` (or muted), drawn from the same tokens.

### 1.2 Typography (the editorial signature)
Two-axis system: a **distinctive display serif** for the brand and page titles (this is what makes it read "editorial studio" instead of "generic SaaS"), paired with **Inter** for body/UI/data.

- **Display / headings:** `Fraunces` (opsz axis, ~`600-700` weight) — theatrical, warm, characterful. Loaded at ~45px and 600/700.
- **UI / body / code:** `Inter` (400/500/600/700) — calm and highly legible for dense surfaces.
- Weights ladder: 400 body, 500 emphasis, 600 section titles, 700 display titles.
- Type scale (rem, tuned ~1.25 ratio):
  - Display/Hero: `1.75rem` (28) — page titles
  - Section: `1.125rem` (18) — card titles
  - Subtitle: `0.9375rem` (15) — card descriptions
  - Body: `0.875rem` (14) — default
  - Small: `0.8125rem` (13) — metadata
  - Micro/caption: `0.75rem` (12) — labels, tabular numbers, overlines
- Overline labels (small-caps-ish): `0.75rem`, `500`, `letter-spacing: 0.08em`, uppercase — used for tab group names and "micro" stat labels. Gives an editorial, organised feel.
- Tabular numbers (`font-variant-numeric: tabular-nums`) for all money/counts.
- Load fonts via bundled `@font-face` (self-host woff2) — Caddy static, no Google CDN, offline-friendly, matches the self-host ethos. Keep fallbacks (`ui-serif`, `-apple-system`, etc.).

### 1.3 Spacing, layout, radius, shadow
- **Base unit 4px.** Spacing scale 2/4/8/12/16/20/24/32/40/48/64.
- **App container:** `max-width: 1280px`, centered, `padding: 0 24px`; generous but not sprawling.
- **Radius:** buttons/inputs `0.5rem` (8); cards `0.75rem` (12); modals/drawers `1rem` (16); avatars `rounded-xl`. Raise these as the "soft, premium" feel (vs. current 0.625rem).
- **Shadow language:** layered & quiet for light theme —
  - `--shadow-sm`: `0 1px 2px oklch(0.26 0.02 260 / 0.05)`
  - `--shadow-md`: `0 2px 8px -1px oklch(0.26 0.02 260 / 0.08)`
  - `--shadow-lg`: `0 8px 24px -4px oklch(0.26 0.02 260 / 0.12)`
  - Cards default to `sm`; hover/raised to `md`.
- **Borders:** `1px` hairline (`--border`), never heavy.
- **Header:** sticky, `height: 60px`, `backdrop-blur`, `bg-background/80`, bottom hairline. Contains: logo + app name (left), page title/breadcrumb, right cluster = health dot + owner identity + auth control.
- **Sidebar:** `width: 240px` (collapsible to 64px icons), `bg-background`, right hairline; grouped nav (see Part 3).

### 1.4 Component restyle (within shadcn)
- **Cards** → `rounded-xl`, `border`, `shadow-sm`, `bg-card`, `p-5/6`; hover states raise shadow & tint border. Card headers get the overline style for section labels.
- **Buttons** → primary = indigo filled; secondary/outline = warm-grey; ghost for nav. Rounded 8px, clear focus ring.
- **Tabs** → **no more global top tab-bar.** Tabs only *inside* a page where genuinely needed (e.g. Dashboard sub-views). Sidebar replaces the global tabs.
- **Badges** → softer pills (rounded-full, tinted bg + matching fg); status tones mapped from semantic tokens.
- **Empty states** → every surface gets a polished empty state (icon, faint title, one-line guidance) instead of bare "None"/"-".
- **Progress bars** (new, for budget/engagement/quality) → slim `h-1.5 rounded-full`, tone-aware fill.
- **Avatars** (new shadcn primitive) → replace raw `<img>/text` circles everywhere; gradient fallbacks.

---

## Part 2 — Information Architecture (pages, not tabs)

The core change: **retire the 12-flat-tab bar** → a **grouped sidebar + header** where surfaces are organised by *what the owner does*, daily things first, internals tucked away.

### Sidebar structure (grouped, with overline group labels)
```
[ 🎬 Casting ]                      ← brand row + project name
──────────── HOME
  Home / Dashboard            (Home)
  Inbox  (badge = unread)     (Inbox)
  Chat                        (Chat)
──────────── WORK
  Board                       (Board)
  Graph                       (Graph)
  Cast  (Team)                (Team)
  History  (Activity)         (Activity)
──────────── REVIEW
  Decisions                   (Decisions)
  Knowledge                   (Knowledge) ← NEW: facts/opinions/risks/briefings
  Spend                       (Spend)     ← NEW: cost breakout (promoted from Overview)
──────────── SETTINGS
  Setup & Connect             (Settings)
  Model routing               (Routing)   ← NEW: promoted from Overview
  [ Advanced ]  Debug         (Debug, collapsed/secondary)
Footer: Health indicator · owner avatar · ⚙ root settings
```

**Why:** Home/Inbox/Chat are the "daily" surfaces; Work holds the execution artifacts; Review holds what the director audits; Settings groups config. Debug is deliberately de-emphasised (under an "Advanced" affordance) — it is *not* a daily surface and shouldn't pretend to be.

### Page map & responsibilities
| Sidebar item | Data source | What it now shows |
|---|---|---|
| **Home / Dashboard** | `/api/model` + projection | Greeting + objective; "Needs your attention" cluster (pending decisions, unread inbox, flagged observations, budget warnings, paused/drift); top 5 priorities with progress bars; compact health strip (spend, engagement%, diff quality, repo snapshot). Visual hierarchy: headline status card → attention cards → detail strip. **No raw object dumps.** |
| **Inbox** | `/api/inbox` + observations | The **action center**: pending decisions awaiting approve/reject (the existing InboxView, restyled) + flagged observations needing action. One clear "resolve" flow per item. |
| **Chat** | `messages` | Restyled team-thread (bubbles with real avatars, timestamps, sender colour key). Reads as a real messenger, not a stack of cards. |
| **Board** | `tasks` | Kanban (5 cols) with denser cards: title, priority badge, assignee avatar, kind. Drawer unchanged conceptually (restyled). |
| **Graph** | `/api/graph` | Existing GraphView, rethemed + empty state. |
| **Cast / Team** | `agents` + consultants | Agent cards → polished roster: avatar, name/role, CV bullets, routing chip (provider/model). |
| **History / Activity** | `ActivityView` | Event timeline, rethemed, with clear action/actor/result framing. |
| **Decisions** | `decisions` | Decision log: status pills, options, recommendation, owner verdict; propose/approve/reject surfaced cleanly. |
| **Knowledge** *(new)* | `opinions/facts/risks` + briefings | Organised into sub-tabs: Facts · Opinions (active/superseded) · Risks · Briefings. Reuses the "oversee the company brain" mental model. |
| **Spend** *(new)* | `spend` + `budget` + `routing` costs | Cost dashboard: total + budget progress bar (warn/halt states), breakdown by agent & cost class (from `CostEntry[] owned by agent_id/cost_class`), tokens + cache hit, latency. Turns Overview's dense Spend card into a first-class page. |
| **Settings / Setup & Connect** | setup status + `/api/policy` | Telegram connect; policies/autonomy (reuses the wizard's preset + per-class controls); project info. |
| **Model routing** *(new)* | `/api/routing` | Read-mostly table of actor→model/cost; surface `spend.by_agent` alongside. |
| **Debug** | `/api/events`, `/api/model` diagnostics | Behind "Advanced": event stream, orchestration runs, rejections, provenance lookup, actor contexts. Unchanged volume, de-prioritised in IA. |

**Notes**
- "Overview" (the current first tab) is **disbanded into Home + Spend + Routing + Knowledge**. Its best ingredients (health, attention, spend) are distributed; no single page is a wall of bullets.
- Graph/Board both live under Work — they were both "the board" semantically before; keep them distinct but grouped.

---

## Part 3 — App shell (layout blueprint)

```
┌──────────────────────────────────────────────────────────────┐
│ Header (60px, sticky):  [🎬 Casting ◆ proj]  ...  ● Health  👤|
├──────────┬───────────────────────────────────────────────────┤
│ Sidebar  │  Content (max-width 1280, px-6, py-6)             │
│  240px   │                                                   │
│ grouped  │   [Page title (Fraunces)]                         │
│ nav with │   [overline subtitle]                             │
│ overline │   ┌──────────────┐  ┌──────────────┐              │
│ labels   │   │  attention   │  │  attention   │  ← dash grid │
│          │   │  card        │  │  card        │              │
│          │   └──────────────┘  └──────────────┘              │
│          │   ┌─────────────────────────────────────┐         │
│          │   │  main panel / page-specific layout  │         │
│          │   └─────────────────────────────────────┘         │
│ footer   │                                                   │
└──────────┴───────────────────────────────────────────────────┘
```
- **Header right cluster:** subtle health dot (green/amber/red, click → tooltip with details), owner avatar + name (dropdown → auth control, switch project).
- **Sidebar:** nav items = icon (lucide) + label; active item = `--primary-soft` tinted pill with `--primary` text; group overline labels uppercase/12/500/spaced. Unread badge on Inbox. Collapsible on mobile (`hamburger` → overlay drawer).
- **Routing:** keep simple — a `useState<Tab>` selected key (no react-router needed yet unless we want deep-linking; see Open Questions). Content area swaps per selection; keep lazy `Suspense` for Whiteboard/Advisor.

---

## Part 4 — Setup wizard redesign (must look different & be clean)

**Goal:** a warm, encouraging, *fast* onboarding — not the current centred single-card 9-step flow. Uses the same light editorial system. Structure:

```
┌───────────────────────────┬──────────────────────────────────┐
│  Progress rail (left,     │  Content card                      │
│  ~300px):                 │  [Step title (Fraunces)]           │
│  🎬 logo + "Casting"      │  [one-line subtitle]               │
│  • Tell us about you ✓    │  ______ step body ______           │
│  • Meet your cast ✓       │                                    │
│  • Your project ●         │   [ Back ]        [ Continue → ]   │
│  • Autonomy               │                                    │
│  • AI provider            │                                    │
│  • Launch                 │                                    │
└───────────────────────────┴──────────────────────────────────┘
```

- **Left progress rail** (`md+`) replaces the tiny dots — always visible, labelled, checkable. Clears on mobile → slim stepper.
- Group the current 9 micro-steps into **5-6 meaningful ones** (fewer clicks = friendlier):
  1. **About you** (name + experience) — combine current `name` + `experience`.
  2. **Meet your cast** (carousel of the roster cards; from `consultants` registry) — unchanged data, redesigned presentation.
  3. **Your project** (existing vs new + details + repo path) — combine `existing-project` + `project-details`.
  4. **Autonomy** (policy presets + optional per-class tweak `<details>`) — unchanged.
  5. **AI provider** (provider picker + model + key) — unchanged, restyled.
  6. **Launch** (review card + "Launch my company") — unchanged.
- Welcome screen folded into step 1 ("About you" shows the PM intro card + name + experience), removing a click.
- The `created` success state becomes a proper final screen (confetti-free but celebratory — big 🎉, what happens next, restart hint) on brand background.

**Backend contract unchanged** — `submitSetup(...)` signature & `/api/setup`, `/api/policy` POSTs stay identical; only presentation changes. Existing `SetupWizard.test.tsx` will need updates for the new DOM/steps (see Validation).

---

## Part 5 — Files to change

**Theme / design tokens**
- `frontend/src/index.css` — replace palette, add `--primary-soft`, `--warning`, `--success`, shadows, type vars, `@font-face` for Fraunces + Inter, refine radius. *(core)*
- `frontend/src/components/ui/{card,button,badge,tabs,input,textarea}.tsx` — token-driven restyle + radius/shadow.
- Add shadcn: `avatar.tsx`, `separator.tsx`, `dropdown-menu.tsx`, `switch.tsx`, `progress.tsx`, `scroll-area.tsx`, `tooltip.tsx` (via `npx shadcn@latest add …`).

**App shell & navigation**
- `frontend/src/App.tsx` — new shell: `Sidebar` + `Header` + content router; replace the `TabsList` (Part 3). Grouped nav definitions array.
- `frontend/src/Shell.tsx` / `Sidebar.tsx` / `Header.tsx` *(new)* — extract shell components.
- `frontend/src/lib/status.ts` *(new)* — semantic token → component mapping helpers (priority/severity → badge/progress variats), tabular-money formatters.

**Pages**
- `Overview.tsx` → **Home/Dashboard** rewrite + split: extract **Spend** & **Routing** & **Knowledge** page components.
- `frontend/src/pages/Spend.tsx` *(new)*, `frontend/src/pages/Routing.tsx` *(new)*, `frontend/src/pages/Knowledge.tsx` *(new)*.
- `frontend/src/pages/Home.tsx` *(new, replaces Overview top-level)*.
- Restyle: `Chat` (bubbles/avatars), `Board`, `Team`→`Cast`, `Decisions`, `Inbox`, `ActivityView`, `DebugView`, `GraphView`, `Whiteboard`/`Advisor` containers, `SettingsView`.
- `frontend/src/SetupWizard.tsx` — Part 4 rewrite (progress rail, step regrouping, success screen).
- `frontend/src/store.ts` — unchanged logic; possibly add selected-tab default + unread plumbing if needed.
- `frontend/src/main.tsx` — unchanged (or fonts preload).

**Tests**
- `frontend/src/__tests__/SetupWizard.test.tsx` — update for new step grouping/DOM.
- `frontend/src/__tests__/boardColumns.test.ts` — verify still green (no board-rule change).
- Add light `App` shell test if desired (nav renders, tab switching) — TDD where cheap.

---

## Part 6 — Validation

- `cd frontend && npx tsc --noEmit` → 0 errors.
- `npx vitest run` → all existing + new frontend tests pass.
- `cargo build` (or `cargo test`) still green where SPA is embedded (no backend changes expected; confirm build embeds new `dist`).
- Manual visual pass in browser: wizard 6-step flow, dashboard attention states (also with sample paused/budget-warn data), each sidebar destination renders, responsive at 1280/768/375 widths, unread badge, health dot.
- Confirm lazy chunks still split (Whiteboard ~1MB stays lazy).

---

## Part 7 — Phasing / execution order

1. **Theme foundation** — `index.css` tokens + font loading + shadcn restyle + new primitives. (Enables everything else.)
2. **App shell** — extract Sidebar/Header, replace global tabs with grouped nav, health/auth in header. App still navigates to existing page components.
3. **Page splitting** — carve Spend, Routing, Knowledge out of Overview; write Home dashboard as the new landing view.
4. **Per-surface restyle** — Chat, Board, Cast, Decisions, Inbox, History, Graph across the new system.
5. **Setup wizard redesign** — progress rail + regrouping + success screen; update tests.
6. **Polish pass** — empty states everywhere, responsive, micro-interactions, final visual QA; rebuild + commit.

Each phase commits independently (`feat(ui): …`), keeping the tree green. Prefer a **static HTML design mockup** (see Open Questions) before Phase 1 to lock the look with Ben before building.

---

## Risks, tradeoffs, open questions

**Risks**
- Serializer: frontend is embedded in the Rust binary; a broken `dist` breaks `cast run`. Mitigate: build + smoke-test the SPA before each commit; keep `cargo build` green.
- Test churn: `SetupWizard.test.tsx` asserts old steps; update tests alongside the rewrite (per Ben: fix the behavior, then tests match reality — but here behavior is intentionally reorganised, so tests are updated to the new IA, not contorted).
- Font self-hosting adds a couple hundred KB to the bundle. Acceptable for the visual win; load display font only (woff2, `font-display: swap`), body Inter subset.

**Tradeoffs**
- Serif display (Fraunces) is a deliberate character choice; if it doesn't land, swapping to a sans display is a one-token change. Confirmed "editorial" direction, but flag as the most opinionated call.
- No react-router: tab-key state only. Deep-linking/bookmarking a specific page isn't possible yet. Cheap to add later if wanted.

**Open questions for Ben**
1. ~~Static visual mockup first?~~ → **Skipped (Ben chose go straight to implementation).**
2. React-router (real URLs `/board`, `/settings`…) now, or keep tab-state for this pass? → **Tab-state (no react-router) this pass.** Deep-linking/bookmarking is a cheap later add.
3. Sidebar collapse-to-icons on desktop, or full-width always? → **Collapsible to 64px icons on desktop; mobile overlays.**

**Status (2026-08-18): DONE.** Theme, shell, page split, per-surface restyle, wizard redesign, and polish all implemented, tested, built, and pushed (`b4d64a0`). Verified: `tsc --noEmit` clean, 7/7 vitest, `vite build` + `cargo build` embed the new SPA, all 15 nav destinations render.

---

## References
- Current UI: `frontend/src/{App,Overview,SetupWizard,index.css}.tsx`; data model in `frontend/src/api.ts`.
- Design context: `react-vite-tailwind-shadcn` skill (Tailwind 4 CSS-first theming, shadcn add flow).
- Backend contract: `src/web/routes/{setup,intake}.rs` unchanged.
