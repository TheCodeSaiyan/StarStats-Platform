#!/bin/sh
# Idempotently provision starstats / spicedb / glitchtip / synapse databases and roles
# in the existing voyager Postgres. Reads passwords from /run/secrets.
# Safe to re-run on every `docker compose up`.

set -eu

export PGHOST=postgres
export PGUSER=postgres
export PGPASSWORD="$(cat /run/secrets/postgres_default)"

ensure_role() {
  role="$1"
  pw_file="$2"
  pw="$(cat "$pw_file")"
  exists="$(psql -At -c "SELECT 1 FROM pg_roles WHERE rolname='${role}'")"
  if [ "${exists}" = "1" ]; then
    echo "role ${role}: present, syncing password"
    psql -c "ALTER ROLE ${role} WITH LOGIN PASSWORD '${pw}'"
  else
    echo "role ${role}: creating"
    psql -c "CREATE ROLE ${role} WITH LOGIN PASSWORD '${pw}'"
  fi
}

ensure_db() {
  db="$1"
  owner="$2"
  exists="$(psql -At -c "SELECT 1 FROM pg_database WHERE datname='${db}'")"
  if [ "${exists}" = "1" ]; then
    echo "database ${db}: present"
  else
    echo "database ${db}: creating, owner=${owner}"
    psql -c "CREATE DATABASE ${db} OWNER ${owner}"
  fi
}

# Synapse refuses to start on a database whose collation is not C, so its
# database is created from template0 with C collation and ctype, and an
# existing one with anything else stops the init loudly rather than leaving
# Synapse to crash-loop later.
ensure_db_c() {
  db="$1"
  owner="$2"
  exists="$(psql -At -c "SELECT 1 FROM pg_database WHERE datname='${db}'")"
  if [ "${exists}" = "1" ]; then
    collation="$(psql -At -c "SELECT datcollate || '/' || datctype FROM pg_database WHERE datname='${db}'")"
    if [ "${collation}" != "C/C" ]; then
      echo "database ${db}: collation is ${collation}, Synapse needs C/C" >&2
      exit 1
    fi
    echo "database ${db}: present (C collation)"
  else
    echo "database ${db}: creating with C collation, owner=${owner}"
    psql -c "CREATE DATABASE ${db} OWNER ${owner} ENCODING 'UTF8' LC_COLLATE 'C' LC_CTYPE 'C' TEMPLATE template0"
  fi
}

ensure_extensions() {
  db="$1"
  shift
  for ext in "$@"; do
    psql -d "${db}" -c "CREATE EXTENSION IF NOT EXISTS \"${ext}\""
  done
}

ensure_role starstats_app /run/secrets/starstats_db_password
ensure_role spicedb_app   /run/secrets/spicedb_db_password
ensure_role glitchtip_app /run/secrets/glitchtip_db_password

ensure_db starstats starstats_app
ensure_db spicedb   spicedb_app
ensure_db glitchtip glitchtip_app

ensure_extensions starstats "uuid-ossp" pgcrypto pg_stat_statements

# Chat (social phase 6). Only once the stack mounts the secret: this image
# rolls out on every push to main, before the Compose change that adds
# Synapse, and the API waits for this init to succeed.
if [ -f /run/secrets/synapse_db_password ]; then
  ensure_role synapse_app /run/secrets/synapse_db_password
  ensure_db_c synapse synapse_app
  # The API's application-service account erases deleted players' Matrix
  # accounts, which needs Synapse admin. It has no config switch, so it is
  # set here, every deploy. The row appears once the API has registered
  # the account (on its first start); until then this matches nothing.
  # Synapse caches the flag, so the first time it takes effect after
  # Synapse restarts.
  if [ "$(psql -d synapse -At -c "SELECT to_regclass('public.users') IS NOT NULL")" = "t" ]; then
    psql -d synapse -c "UPDATE users SET admin = 1 WHERE name = '@starstats:starstats.app' AND admin = 0"
  fi
else
  echo "synapse: no synapse_db_password secret mounted, skipping"
fi

echo "starstats-db-init: complete"
