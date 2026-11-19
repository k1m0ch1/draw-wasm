// Edge host for coret.app.
// Static: served from Worker assets (same edge as Pages). /api/* and /ws/*: proxied to the
// draw backend through its /coret-api/ and /coret-ws/ origin paths (WebSocket included).
// The origin host's /coret-api/ and /coret-ws/ paths (see deploy/nginx.conf.template).
const ORIGIN = 'https://@@ORIGIN_HOST@@';

export default {
  async fetch(request, env) {
    const u = new URL(request.url);
    let target;
    if (u.pathname.startsWith('/api/')) {
      target = ORIGIN + '/coret-api/' + u.pathname.slice(5) + u.search;
    } else if (u.pathname.startsWith('/ws/')) {
      target = ORIGIN + '/coret-ws/' + u.pathname.slice(4) + u.search;
    } else {
      const res = await env.ASSETS.fetch(request);
      if ((res.status === 404 || (res.status >= 300 && res.status < 400)) && !u.pathname.startsWith('/assets/')) {
        const page = await env.ASSETS.fetch(new Request(u.origin + '/index.html'));
        return new Response(page.body, { status: 200, headers: page.headers }); // SPA fallback for /room/doc routes
      }
      return res;
    }
    const headers = new Headers(request.headers);
    return fetch(new Request(target, { method: request.method, headers, body: request.body, redirect: 'manual' }));
  },
};
