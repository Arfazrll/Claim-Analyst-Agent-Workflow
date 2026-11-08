#!/usr/bin/env sh
# End-to-end smoke test against a running stack (used by CI after `docker compose up`).
# Registers a user, analyses a sample claim, waits for on-chain anchoring and checks the
# integrity proof. Requires curl and jq.
set -eu
BASE="${1:-http://localhost:8080}"
JAR="$(mktemp)"
trap 'rm -f "$JAR"' EXIT
EMAIL="smoke-$(date +%s)@example.com"

echo "waiting for $BASE ..."
i=0
until curl -fsS "$BASE/edge-health" >/dev/null 2>&1 && curl -fsS -o /dev/null -w '%{http_code}' "$BASE/api/auth/me" 2>/dev/null | grep -q 401; do
  i=$((i + 1)); [ "$i" -gt 90 ] && { echo "stack not ready"; exit 1; }; sleep 2
done

csrf() { awk '$6 == "caw_csrf" {print $7}' "$JAR"; }
post() { curl -fsS -b "$JAR" -c "$JAR" -H 'content-type: application/json' -H "x-csrf-token: $(csrf)" -H "origin: $BASE" -d "$2" "$BASE$1"; }

curl -fsS -c "$JAR" -H 'content-type: application/json' -H "origin: $BASE" \
  -d "{\"email\":\"$EMAIL\",\"name\":\"Smoke Test\",\"password\":\"Correct-Horse-Battery-9\"}" "$BASE/api/auth/register" >/dev/null
echo "registered $EMAIL"

CASE=$(curl -fsS -b "$JAR" "$BASE/api/samples" | jq -c '.items[] | select(.case_id == "PUB-001") | .case')
RES=$(post /api/claims "{\"case\": $CASE}")
ID=$(echo "$RES" | jq -r .claim.id)
DEC=$(echo "$RES" | jq -r .analysis.decision)
DED=$(echo "$RES" | jq -r .analysis.amounts.total_deductions_inr)
echo "claim $ID -> $DEC (deductions $DED)"
[ "$DEC" = "ADMISSIBLE_WITH_LIMITS" ] || { echo "unexpected decision"; exit 1; }
[ "$DED" = "10200" ] || [ "$DED" = "10200.0" ] || { echo "unexpected deduction"; exit 1; }

for _ in $(seq 1 30); do
  STATUS=$(curl -fsS -b "$JAR" "$BASE/api/claims/$ID/proof" | jq -r '.entries[0].chain_status')
  [ "$STATUS" = "anchored" ] && break
  sleep 2
done
PROOF=$(curl -fsS -b "$JAR" "$BASE/api/claims/$ID/proof")
echo "$PROOF" | jq -r '.checks[] | "  \(if .passed then "PASS" else "FAIL" end) \(.name)"'
[ "$(echo "$PROOF" | jq -r .ok)" = "true" ] || { echo "integrity proof failed"; exit 1; }
[ "$(echo "$PROOF" | jq -r '.entries[0].chain_status')" = "anchored" ] || { echo "entry not anchored on-chain"; exit 1; }
[ -n "$(echo "$PROOF" | jq -r '.entries[0].cid // empty')" ] || { echo "entry not pinned to IPFS"; exit 1; }

# Rate limiter answers 429 under a burst of sign-in attempts.
CODES=$(for _ in $(seq 1 20); do curl -s -o /dev/null -w '%{http_code}\n' -H 'content-type: application/json' -H "origin: $BASE" -d '{"email":"x@example.com","password":"y"}' "$BASE/api/auth/login"; done | sort | uniq -c)
echo "$CODES" | grep -q 429 || { echo "rate limiting did not engage"; exit 1; }

echo "smoke test passed"
