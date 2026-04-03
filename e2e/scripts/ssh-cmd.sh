#!/usr/bin/env bash
# Helper: run a command in the Tart VM via SSH.
# Usage: ./e2e/scripts/ssh-cmd.sh <vm-name> <command> [args...]

set -euo pipefail

VM_NAME="${1:?Usage: ssh-cmd.sh <vm-name> <command> [args...]}"
shift

VM_IP=$(tart ip "$VM_NAME")
sshpass -p admin ssh \
  -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=/dev/null \
  -o LogLevel=ERROR \
  -o ConnectTimeout=10 \
  "admin@${VM_IP}" \
  "$@"
