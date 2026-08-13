# Development Deployment & Remote Access

This document captures how the Casting dev environment is run, supervised, and
exposed for remote/phone access on the production host (`vps-53b30dc5`,
`15.235.211.194`). Written 2026-08-09 during the initial server migration.

---

## Architecture

- **`cast run`** — Rust web/API backend (axum), binds `127.0.0.1:8080`
  (`CAST_ADDR` overrides). Serves the API + the embedded SPA.
- **Vite dev server** — React SPA dev server, binds `127.0.0.1:5173`, proxies
  `/api` → `127.0.0.1:8080` (SSE-unbuffered for `/api/_events`).
- **Caddy** — reverse-proxies the public host `dev.benstorey.com` to Vite
  (:5173). Rarely: `cast run` alone serves the whole app on :8080.

Remote access while traveling: open **https://dev.benstorey.com** on the phone →
(browser prompts for Basic Auth) → Caddy → Vite (:5173) → (SPA + `/api` →
`cast run` :8080).

> 🔐 **Auth:** The `dev.benstorey.com` vhost is protected by **HTTP Basic Auth**
> (Caddy `basicauth`, bcrypt). This covers both the SPA **and** the `/api`
> backend — nothing is reachable unauthenticated. Credentials are remembered by
> your phone's browser after the first login. To change the password, generate a
> new bcrypt hash and update the block:
> `caddy hash-password --algorithm bcrypt --plaintext 'NEWPASS'`.

---

## Workspace location

The Casting dev workspace lives **outside** the source tree
(`/home/ben/casting-workspace/`), NOT in `.dev/` — the D5 ownership-boundary
guard refuses any repo inside the embedded source root.

| Path | Purpose |
|------|---------|
| `/home/ben/casting` | Casting source repo (product code) |
| `/home/ben/casting-workspace/proj` | artifact repo `/api` live target |

Casting **state lives collocated** in `<repo>/.casting/` (gitignored): events.db,
cursors.db, snapshots (optional), `config.json` (owner token), `secrets.json`.
There is no separate state directory — the workspace and its state are the same
repo.

Both are gitignored via `.dev/` in repo `.gitignore` (the external workspace
is outside the repo entirely).

---

## Managing the dev stack (systemd)

Two services supervise the dev stack (both `enabled` on boot):

```bash
sudo systemctl status  cast-backend   # cast run on :8080
sudo systemctl status  cast-frontend  # vite on :5173
sudo systemctl restart cast-backend
sudo systemctl restart cast-frontend
sudo systemctl stop    cast-frontend cast-backend
```

> **⚠️ Stale-serve pitfall (learned 2026-08-10):** these are LONG-LIVED
> processes, and they do NOT survive a dependency or binary change under them.
> A running Vite dev server (`cast-frontend`) holds its module graph in memory —
> if `node_modules` is swapped underneath it (e.g. a frontend major upgrade),
> it serves a **blank/broken page** (`Cannot find module …/vite/…/dep-*.js`)
> until restarted. Likewise `cast-backend` serves the *embedded* build, so it
> needs a rebuild + restart to pick up Rust or embedded-frontend changes.
>
> **Always use the one-command deploy instead of editing in place:**
> ```bash
> make deploy-dev    # = make build (re-embed SPA) + make restart (both services)
> ```
> So a deploy can't silently go stale. (For pure frontend *code* edits on the
> live host, Vite HMR already reflects them without a restart — the restart is
> the safety net for dep/binary changes.)

- Unit files: `/etc/systemd/system/cast-backend.service`,
  `/etc/systemd/system/cast-frontend.service`.
- `cast-frontend` depends on (`Wants` + `After`) `cast-backend`.
- The frontend unit sets `Environment=PATH=/home/ben/.local/bin:...` because
  node/npm are installed under `~/.local/bin` (not in systemd's default PATH).
- Both run as user `ben`, `Restart=on-failure`.

Manual run (equivalent, if not using systemd):
> **Casting is SINGLE-PROJECT (2026-08-12).** There is no registry or project
> name — the binary runs exactly one project, the dir you pass. The home-dir
> `~/.casting/projects.json` registry was removed.

```bash
CAST_ADDR=127.0.0.1:8080 ./target/debug/cast run /home/ben/casting-workspace/proj
cd /home/ben/casting/frontend && npm run dev -- --host 127.0.0.1   # separately
```

---

## Docker (optional — for users who prefer a container)

A container is just an alternative way to run the same single binary. It is
**optional**; the binary + local `cast run` remain the primary path.

Build and run (from the repo root):

```bash
docker build -t casting .                             # multi-stage: node SPA → rust binary → slim runtime
docker run --rm -p 8080:8080 \                        # serve on :8080
  -v "/path/to/project:/home/casting/projects/demo" \ # your project repo (single project)
  casting run /home/casting/projects/demo
docker run --rm casting --help                        # explore the CLI
```

- The project repo must be **mounted** so the container sees the same project as
  a host run; per-project state stays collocated in `<repo>/.casting/` and
  persists in the mounted repo. (No registry mount — Casting is single-project.)
- The image runs as a non-root user (`casting`, uid 1000) and ships
  `ca-certificates` + `git` (needed by the workspace git runner). Because the
  container user is non-root, **mounted project dirs must be writable
  by uid 1000** — e.g. `sudo chown -R 1000:1000 /path/to/project`, or
  your `cast run` will hit "Permission denied" creating `<repo>/.casting/`.
- Internally `CAST_ADDR=0.0.0.0:8080` so the container binds the exposed port.

The image has been verified end-to-end: it builds (`docker build -t casting .`),
`casting --help` runs, and `casting run <dir>` boots a mounted project and
serves the API (state 200) on the published port.

---

## Caddy

Config: `/etc/caddy/Caddyfile`. The `dev.benstorey.com` block:

```caddy
dev.benstorey.com {
	basicauth {
		ben <bcrypt-hash>
	}
	reverse_proxy 127.0.0.1:5173 {
		flush_interval -1   # unbuffered for Vite HMR + SSE
	}
}
```

Caddy auto-issues + auto-renews a Let's Encrypt cert for `dev.benstorey.com`
(global `acme_ca` pins Let's Encrypt). Production vhosts
(`www.benstorey.com`, `multistorey.com`) are unchanged and served alongside.
The committed block in `deploy/Caddyfile.dev` is the source of truth.

> **Vite host allow-list:** Vite only accepts requests whose `Host` header is
> `localhost`/`127.0.0.1`. Because Caddy forwards the public host, `server
> .allowedHosts` must include `dev.benstorey.com` (set in
> `frontend/vite.config.ts`) or Vite returns `403 Blocked request`.

---

## DNS (NameCheap)

`dev.benstorey.com` needs an **A record → `15.235.211.194`**. There is no
wildcard record, so it must be added explicitly. Once added, Caddy obtains the
cert automatically on the next request — no action needed.

---

## Reproducing on a fresh host (checklist)

1. Install Caddy (official repo) + copy `Caddyfile`; see `deploy/Caddyfile`-style
   block above. Reload.
2. Create workspace dirs: `mkdir -p /home/ben/casting-workspace/proj /home/ben/casting-workspace/state`.
3. Install the two systemd units (copy to `/etc/systemd/system/`), `daemon-reload`,
   `enable --now`.
4. Add DNS `dev.benstorey.com` → host IP.
5. Firewall: ensure 80/443 open (ufw) — 5173/8080 stay bound to 127.0.0.1 only.

---

## Notes / pitfalls (learned)

- **Ownership guard:** `.dev/proj` inside the source tree is refused ("refusing
  to operate on the Casting source repo"). Use the external workspace path.
- **systemd PATH:** node/npm are under `~/.local/bin`; without
  `Environment=PATH=...` or an absolute `ExecStart`, npm fails with
  `env: 'node': No such file or directory` / exit 203.
- The binary must be rebuilt (`cargo build`) after backend changes; the SPA must
  be rebuilt (`npm run build` in `frontend/`) before a re-embed for `cast run` to
  serve the real UI without the Vite dev server.