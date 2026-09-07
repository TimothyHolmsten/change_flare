# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /src/target/release/change_flare /change_flare
USER nonroot
ENTRYPOINT ["/change_flare"]
