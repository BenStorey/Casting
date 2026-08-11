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
| `/home/ben/casting-workspace/state` | Casting state-dir (always separate) |

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

- Unit files: `/etc/systemd/system/cast-backend.service`,
  `/etc/systemd/system/cast-frontend.service`.
- `cast-frontend` depends on (`Wants` + `After`) `cast-backend`.
- The frontend unit sets `Environment=PATH=/home/ben/.local/bin:...` because
  node/npm are installed under `~/.local/bin` (not in systemd's default PATH).
- Both run as user `ben`, `Restart=on-failure`.

Manual run (equivalent, if not using systemd):

```bash
cast add dev /home/ben/casting-workspace/proj   # once: register the project
CAST_ADDR=127.0.0.1:8080 ./target/debug/cast run dev
cd /home/ben/casting/frontend && npm run dev -- --host 127.0.0.1   # separately
```

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