#!/bin/sh
# Deploy ClaimLedger once and write the address to $DEPLOYMENT_FILE.
# Idempotent: if the file points at a contract that still has code, nothing is deployed
# (a fresh Anvil node has no code at the old address, so it redeploys automatically).
#
# Env: RPC_URL, DEPLOYER_PRIVATE_KEY, WRITER_ADDRESS, DEPLOYMENT_FILE, CHAIN_ID
set -eu

RPC_URL="${RPC_URL:-http://anvil:8545}"
DEPLOYMENT_FILE="${DEPLOYMENT_FILE:-/shared/deployment.json}"
WRITER_ADDRESS="${WRITER_ADDRESS:-0x0000000000000000000000000000000000000000}"
: "${DEPLOYER_PRIVATE_KEY:?DEPLOYER_PRIVATE_KEY is required}"

echo "waiting for RPC at $RPC_URL ..."
i=0
until cast block-number --rpc-url "$RPC_URL" >/dev/null 2>&1; do
  i=$((i + 1))
  [ "$i" -gt 60 ] && { echo "RPC not reachable"; exit 1; }
  sleep 1
done

if [ -f "$DEPLOYMENT_FILE" ]; then
  existing=$(sed -n 's/.*"address": *"\(0x[0-9a-fA-F]*\)".*/\1/p' "$DEPLOYMENT_FILE")
  if [ -n "$existing" ] && [ "$(cast code "$existing" --rpc-url "$RPC_URL")" != "0x" ]; then
    echo "ClaimLedger already deployed at $existing"
    exit 0
  fi
fi

cd "$(dirname "$0")"
out=$(forge create src/ClaimLedger.sol:ClaimLedger \
  --rpc-url "$RPC_URL" --private-key "$DEPLOYER_PRIVATE_KEY" --broadcast \
  --constructor-args "$WRITER_ADDRESS" --json)
address=$(echo "$out" | sed -n 's/.*"deployedTo": *"\(0x[0-9a-fA-F]*\)".*/\1/p')
tx=$(echo "$out" | sed -n 's/.*"transactionHash": *"\(0x[0-9a-fA-F]*\)".*/\1/p')
[ -n "$address" ] || { echo "deployment failed: $out"; exit 1; }

chain_id=$(cast chain-id --rpc-url "$RPC_URL")
mkdir -p "$(dirname "$DEPLOYMENT_FILE")"
cat > "$DEPLOYMENT_FILE" <<EOF
{"address": "$address", "chain_id": $chain_id, "tx_hash": "$tx", "writer": "$WRITER_ADDRESS"}
EOF
echo "ClaimLedger deployed at $address (chain $chain_id)"
