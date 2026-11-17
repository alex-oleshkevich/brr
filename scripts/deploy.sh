#!/usr/bin/env bash
# Deploy deploy/compose.yaml with uncloud, then delete the brr images no container uses:
# every deploy builds a new ~1.4 GB brr:<date> image on the machine.
set -euo pipefail
cd "$(dirname "$0")/../deploy"
host=${BRR_DEPLOY_SSH:-ssh.aresa.me}

uc -c "${BRR_DEPLOY_CONTEXT:-default}" deploy -f compose.yaml -y "$@"

ssh -o BatchMode=yes "$host" '
  used=$(docker ps -a --format "{{.Image}}" | sort -u)
  for image in $(docker images --format "{{.Repository}}:{{.Tag}}" | grep "^brr:"); do
    echo "$used" | grep -qx "$image" || docker rmi "$image"
  done
'
