# syntax=docker/dockerfile:1.7
# OpenTelemetry Collector with the StarStats config baked in.
# Pinned by digest — tag is mutable, digest is byte-for-byte immutable.
# Bump deliberately by re-resolving the manifest with `docker buildx
# imagetools inspect <repo>:<tag> --format '{{.Manifest.Digest}}'`.
FROM otel/opentelemetry-collector-contrib:0.115.1@sha256:d2da12c4336a79758826700be9e21ecf4a9f7d945b7f8a58ba55ee3fa45427c8
COPY config.yaml /etc/otelcol/config.yaml
