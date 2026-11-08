#!/usr/bin/env sh
# Create .env from .env.example with freshly generated secrets (idempotent: an
# existing .env is left untouched).
set -eu
cd "$(dirname "$0")/.."
if [ -f .env ]; then
  echo ".env already exists; not overwriting."
  exit 0
fi
rand() { openssl rand -base64 "$1" | tr -d '\n/+=' | cut -c1-"$2"; }
cp .env.example .env
set_var() { sed -i.bak "s|^$1=.*|$1=$2|" .env && rm -f .env.bak; }
set_var POSTGRES_PASSWORD "$(rand 32 32)"
set_var REDIS_PASSWORD "$(rand 32 32)"
set_var JWT_SECRET "$(rand 64 64)"
set_var DATA_ENCRYPTION_KEY "$(openssl rand -base64 32 | tr -d '\n')"
set_var METRICS_TOKEN "$(rand 32 32)"
set_var ADMIN_PASSWORD "Desk-$(rand 24 16)-9a"
echo "Wrote .env. Admin sign-in: $(grep ^ADMIN_EMAIL= .env | cut -d= -f2) / $(grep ^ADMIN_PASSWORD= .env | cut -d= -f2)"
