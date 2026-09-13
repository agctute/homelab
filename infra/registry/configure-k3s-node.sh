#!/bin/sh
# Run this on EACH k3s node (bravostation, charliestation, deltastation) so
# containerd will pull from the in-cluster registry (infra/registry/) over
# plain HTTP instead of expecting TLS. Chosen deliberately for a LAN-only
# homelab registry over the cost of managing a cert.
#
# Usage: sudo ./configure-k3s-node.sh
set -eu

REGISTRY="10.0.0.11:30500"
REGISTRIES_YAML="/etc/rancher/k3s/registries.yaml"

if [ "$(id -u)" -ne 0 ]; then
    echo "must run as root (sudo)" >&2
    exit 1
fi

if [ -f "$REGISTRIES_YAML" ] && grep -q "$REGISTRY" "$REGISTRIES_YAML" 2>/dev/null; then
    echo "$REGISTRIES_YAML already configures $REGISTRY, nothing to do"
    exit 0
fi

mkdir -p /etc/rancher/k3s
cat > "$REGISTRIES_YAML" <<EOF
mirrors:
  "$REGISTRY":
    endpoint:
      - "http://$REGISTRY"
configs:
  "$REGISTRY":
    tls:
      insecure_skip_verify: true
EOF

if systemctl is-active --quiet k3s; then
    systemctl restart k3s
    echo "k3s (server) restarted, containerd now trusts $REGISTRY"
elif systemctl is-active --quiet k3s-agent; then
    systemctl restart k3s-agent
    echo "k3s-agent restarted, containerd now trusts $REGISTRY"
else
    echo "neither k3s nor k3s-agent is active on this host — is this a k3s node?" >&2
    exit 1
fi
