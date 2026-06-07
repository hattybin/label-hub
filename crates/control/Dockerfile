# Build the control plane binary, then ship a slim runtime image.
# Built from the workspace root context: `docker build -f crates/control/Dockerfile .`

FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p label-control

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /src/target/release/label-control /usr/local/bin/label-control
COPY --from=build /src/crates/control/web /app/web
COPY --from=build /src/crates/control/migrations /app/migrations
ENV DASH_WEB_DIR=/app/web
EXPOSE 9090 9091
CMD ["label-control"]
