# syntax=docker/dockerfile:1.7
# StarStats Matrix homeserver: upstream Synapse, pinned, plus the StarStats
# chat guard module and a config template. Secrets are read from
# /run/secrets at start by entrypoint.py; none is in the image.
#
# Synapse is AGPL-3.0; the chat guard module is published under the same
# licence (see its header).
FROM matrixdotorg/synapse:v1.161.0@sha256:6b95dd129e35e97e8032cb759e5db4a516a5f7ef6c98d34d716fea1ae239b62a

COPY starstats_chat_guard.py test_chat_guard.py /usr/local/lib/starstats/
COPY homeserver.yaml appservice-starstats.yaml log.config entrypoint.py /etc/starstats-synapse/
ENV PYTHONPATH=/usr/local/lib/starstats
# The guard's rules are tested against this exact Synapse at build time, so
# an image whose rules fail (or whose module API moved) is never pushed.
RUN cd /usr/local/lib/starstats && python -m unittest -q test_chat_guard

# UID 1000 owns the rendered secret files on the host, so Synapse runs as
# it; /data is created with that owner so a fresh named volume inherits it.
RUN mkdir -p /data && chown 1000:1000 /data
USER 1000:1000
WORKDIR /usr/local/lib/starstats

EXPOSE 8008
HEALTHCHECK --interval=15s --timeout=5s --start-period=60s --retries=5 \
  CMD curl -fsS http://localhost:8008/health || exit 1

ENTRYPOINT ["python", "/etc/starstats-synapse/entrypoint.py"]
