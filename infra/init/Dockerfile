# syntax=docker/dockerfile:1.7
# Postgres bootstrap — provisions roles, DBs, and extensions for
# starstats / spicedb / glitchtip in the existing shared cluster.
# Idempotent; safe to run on every `docker compose up`.
#
# Base must match the libpq major of the server (pg17). Using the
# official postgres image gets us psql + libpq + a known-good cert
# bundle in ~80MB. Alpine variant trades 60MB of disk for a slightly
# different libc — not worth it for a one-shot init container.
FROM postgres:17-bookworm@sha256:0042a9d3d3365f837bd561981a9098518ed529b801fb2f655338d49108a464d5

COPY init-databases.sh /init/init-databases.sh
RUN chmod +x /init/init-databases.sh

ENTRYPOINT ["/init/init-databases.sh"]
