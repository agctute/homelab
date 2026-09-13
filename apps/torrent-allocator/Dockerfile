FROM rust:slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev libsqlite3-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# cache deps
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release 2>/dev/null || true

# real build
COPY src ./src
COPY migrations ./migrations
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim

# zip/unrar handle CBZ packaging (see src/format/convert.rs — a cbz is just
# a zip archive of images); calibre provides `ebook-convert` for EPUB/PDF.
# `unrar` here is the free `unrar-free` clone from Debian's main repo, not
# RARLAB's proprietary unrar — it doesn't fully support RAR5. If real-world
# .cbr/.rar releases fail to extract, switch to the non-free `unrar`
# package (requires enabling the `non-free` component in sources.list).
# calibre is a large install (~1GB) since it pulls in Qt; that's the
# tradeoff for using the real, well-documented conversion tool rather than
# a lighter but less capable alternative.
RUN apt-get update && apt-get install -y ca-certificates zip unrar-free calibre && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/torrent-allocator /usr/local/bin/torrent-allocator

CMD ["torrent-allocator"]
