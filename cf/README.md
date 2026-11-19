# coret.app edge

Static frontend + API proxy on Cloudflare for the production instance (coret.app),
with the backend running as the second container (port 8191) behind the origin
host's `/coret-api/` and `/coret-ws/` paths (see `deploy/nginx.conf.template`).

Deploy:
1. Build/pull a fresh `dist/` (same as the container's static bundle).
2. Copy it to `cf/public/` and add `_headers` (security headers + immutable asset cache).
3. Fill in `@@ORIGIN_HOST@@` in `coret-edge.js` and `@@ACCOUNT_ID@@` in `wrangler.toml.example`.
4. `wrangler deploy`, then attach the zone apex as a Worker Custom Domain
   (creates the DNS record and the certificate automatically).

Notes: the SPA fallback serves `/index.html` for non-asset paths so deep links
(`/room/doc`) work; `/api/*` and `/ws/*` (WebSocket) are proxied to the origin.
