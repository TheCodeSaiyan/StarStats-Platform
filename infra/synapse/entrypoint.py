"""Render the Synapse config from its template, then start Synapse.

Values `secret:NAME` become the contents of /run/secrets/NAME and values
`env:NAME` become that environment variable; a missing one stops the
container with its name, never its value. The rendered files go to
/tmp/synapse-config in the container's own layer, not the /data volume, so
secrets are never persisted with the data. The homeserver signing key is
generated into /data on first start and reused after.
"""

import os
import sys

import yaml

TEMPLATES = "/etc/starstats-synapse"
OUT = "/tmp/synapse-config"
SECRETS = os.environ.get("SYNAPSE_SECRETS_DIR", "/run/secrets")


def resolve(value):
    if isinstance(value, dict):
        return {k: resolve(v) for k, v in value.items()}
    if isinstance(value, list):
        return [resolve(v) for v in value]
    if isinstance(value, str) and value.startswith("secret:"):
        name = value[len("secret:"):]
        path = os.path.join(SECRETS, name)
        try:
            with open(path, encoding="utf-8") as f:
                text = f.read()
        except OSError:
            sys.exit(f"synapse entrypoint: secret {name} is missing at {path}")
        # PEM keys keep their line breaks; one-line secrets lose the newline
        # the renderer adds.
        return text if "-----BEGIN" in text else text.strip()
    if isinstance(value, str) and value.startswith("env:"):
        name = value[len("env:"):]
        if not os.environ.get(name):
            sys.exit(f"synapse entrypoint: environment variable {name} is not set")
        return os.environ[name]
    return value


def render(name):
    with open(os.path.join(TEMPLATES, name), encoding="utf-8") as f:
        config = resolve(yaml.safe_load(f))
    path = os.path.join(OUT, name)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        yaml.safe_dump(config, f, sort_keys=False)
    return path


def main():
    os.makedirs(OUT, mode=0o700, exist_ok=True)
    render("appservice-starstats.yaml")
    config = render("homeserver.yaml")
    os.makedirs("/data/keys", exist_ok=True)
    if not os.path.exists("/data/keys/starstats.app.signing.key"):
        print("synapse entrypoint: generating the homeserver signing key", flush=True)
        rc = os.spawnlp(
            os.P_WAIT, "python", "python", "-m", "synapse.app.homeserver",
            "--config-path", config, "--generate-keys",
        )
        if rc != 0:
            sys.exit(f"synapse entrypoint: key generation failed ({rc})")
    os.execlp("python", "python", "-m", "synapse.app.homeserver", "--config-path", config)


if __name__ == "__main__":
    main()
