#!/bin/sh
# Idempotently provision starstats / spicedb / glitchtip databases and roles
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

echo "starstats-db-init: complete"
