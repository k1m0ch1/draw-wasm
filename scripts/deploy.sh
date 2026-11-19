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
# Build the Rust build stage under a draw-* tag first, so no untagged 2 GB intermediate image is left behind;
# the previous tagged build image is removed afterwards. Build logs stay under $DEPLOY_DIR (never /tmp).
run ssh "$DEPLOY_SSH" "cd $DEPLOY_DIR/src && OLD=\$(docker images -q draw-app:latest) && OLDB=\$(docker images -q draw-build:latest) && nice -n 19 docker build --target build -t draw-build:latest . > $DEPLOY_DIR/build.log 2>&1 && PUBLIC_BASE_URL=${PUBLIC_BASE_URL:-} nice -n 19 docker compose -p draw up -d --build >> $DEPLOY_DIR/build.log 2>&1 && NEW=\$(docker images -q draw-app:latest) && NEWB=\$(docker images -q draw-build:latest) && { [ -z \"\$OLD\" ] || [ \"\$OLD\" = \"\$NEW\" ] || docker rmi \"\$OLD\" || true; } && { [ -z \"\$OLDB\" ] || [ \"\$OLDB\" = \"\$NEWB\" ] || docker rmi \"\$OLDB\" || true; }"
sed "s/@@HOST@@/$DEPLOY_HOST/g" deploy/nginx.conf.template > "${TMPDIR:-.}/draw-vhost.rendered"
echo "Rendered vhost at ${TMPDIR:-.}/draw-vhost.rendered; install it as /etc/nginx/sites-available/$DEPLOY_HOST, run nginx -t, then reload."
