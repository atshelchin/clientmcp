# ClientMCP bridge: a static binary on an empty image.
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN CARGO_PROFILE_RELEASE_STRIP=true cargo build --release --locked -p clientmcp-server

FROM scratch
COPY --from=build /src/target/release/clientmcp /clientmcp
ENV CLIENTMCP_LISTEN=0.0.0.0:8080
EXPOSE 8080
USER 65534:65534
ENTRYPOINT ["/clientmcp"]
