# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked \
    && cp /src/target/release/change_flare /change_flare

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /change_flare /change_flare
USER nonroot
ENTRYPOINT ["/change_flare"]
