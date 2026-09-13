#!/bin/sh
# Run this on any machine that needs to `docker build`/`docker push` against
# the in-cluster registry (infra/registry/) — it's plain HTTP, so Docker
# needs to be told to trust it explicitly rather than expecting TLS.
#
# Usage: sudo ./configure-docker-client.sh
set -eu

REGISTRY="10.0.0.11:30500"
DAEMON_JSON="/etc/docker/daemon.json"

if [ "$(id -u)" -ne 0 ]; then
    echo "must run as root (sudo)" >&2
    exit 1
fi

if [ -f "$DAEMON_JSON" ]; then
    if grep -q "$REGISTRY" "$DAEMON_JSON" 2>/dev/null; then
        echo "$DAEMON_JSON already trusts $REGISTRY, nothing to do"
        exit 0
    fi
    echo "$DAEMON_JSON already exists and doesn't mention $REGISTRY — edit it by hand:"
    echo "  add \"$REGISTRY\" to the \"insecure-registries\" array"
    cat "$DAEMON_JSON"
    exit 1
fi

cat > "$DAEMON_JSON" <<EOF
{
  "insecure-registries": ["$REGISTRY"]
}
EOF

systemctl restart docker
echo "docker restarted, now trusts $REGISTRY as an insecure (HTTP) registry"
