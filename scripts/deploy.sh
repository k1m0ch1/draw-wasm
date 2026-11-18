#!/usr/bin/env bash
# Deploy helper. Reads environment-specific values from the environment or an untracked .deploy.env.
# This script only prints the steps' commands when DRY_RUN=1; the real access wrapper is site specific.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .deploy.env ] && set -a && . ./.deploy.env && set +a
: "${DEPLOY_HOST:?set DEPLOY_HOST}" "${DEPLOY_SSH:?set DEPLOY_SSH}" "${DEPLOY_DIR:=/opt/draw}"
run() { if [ -n "${DRY_RUN:-}" ]; then echo "+ $*"; else "$@"; fi; }
run ssh "$DEPLOY_SSH" "mkdir -p $DEPLOY_DIR/src"
run rsync -az --delete --exclude .git --exclude target --exclude dist --exclude node_modules ./ "$DEPLOY_SSH:$DEPLOY_DIR/src/"
run ssh "$DEPLOY_SSH" "cd $DEPLOY_DIR/src && PUBLIC_BASE_URL=${PUBLIC_BASE_URL:-} nice -n 19 docker compose -p draw up -d --build"
sed "s/@@HOST@@/$DEPLOY_HOST/g" deploy/nginx.conf.template > /tmp/draw-vhost.rendered
echo "Rendered vhost at /tmp/draw-vhost.rendered; install it as /etc/nginx/sites-available/$DEPLOY_HOST, run nginx -t, then reload."
