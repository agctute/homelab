FROM rust:1-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev libsqlite3-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# cache deps
COPY Cargo.toml Cargo.lock* ./
RUN mkdir -p src/bin && echo "fn main() {}" > src/main.rs && echo "" > src/lib.rs && echo "fn main() {}" > src/bin/migrate_filenames.rs && echo "fn main() {}" > src/bin/fix_scene_names.rs && echo "fn main() {}" > src/bin/audit_library.rs && echo "fn main() {}" > src/bin/purge_manga.rs && echo "fn main() {}" > src/bin/rename_dual_title.rs
RUN cargo build --release 2>/dev/null || true

# real build
COPY src ./src
COPY migrations ./migrations
RUN find src -name '*.rs' -exec touch {} + && cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/comic-retrieval /usr/local/bin/comic-retrieval
COPY --from=builder /app/target/release/migrate-filenames /usr/local/bin/migrate-filenames
COPY --from=builder /app/target/release/fix-scene-names /usr/local/bin/fix-scene-names
COPY --from=builder /app/target/release/audit-library /usr/local/bin/audit-library
COPY --from=builder /app/target/release/purge-manga /usr/local/bin/purge-manga
COPY --from=builder /app/target/release/rename-dual-title /usr/local/bin/rename-dual-title

CMD ["comic-retrieval"]
