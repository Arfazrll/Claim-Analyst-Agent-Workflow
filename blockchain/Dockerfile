# One-shot deployer for ClaimLedger (used by docker-compose and as a Kubernetes Job).
FROM ghcr.io/foundry-rs/foundry:stable

USER root
RUN mkdir -p /contracts /shared && chown -R 1000:1000 /contracts /shared
WORKDIR /contracts
COPY --chown=1000:1000 foundry.toml deploy.sh ./
COPY --chown=1000:1000 src ./src
USER 1000
# Compile at build time so the job only signs and sends the transaction.
RUN forge build && chmod +x deploy.sh

ENTRYPOINT ["/bin/sh", "/contracts/deploy.sh"]
