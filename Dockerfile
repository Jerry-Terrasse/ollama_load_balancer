ARG DEBIAN_VERSION="bookworm"
ARG RUST_VERSION="1.84.0"

FROM docker.io/rust:${RUST_VERSION}-${DEBIAN_VERSION} AS builder

COPY . /app
WORKDIR /app

# Dependencies
#RUN apt update && apt install -y musl musl-dev musl-tools libssl-dev openssl pkg-config

RUN cargo build --release &&\
    find /app/target -name ollama_load_balancer | xargs -I {} mv {} /ollama_load_balancer

# Final stage
FROM docker.io/debian:${DEBIAN_VERSION}

RUN apt update && apt install -y libssl-dev

COPY --from=builder /ollama_load_balancer /usr/local/bin/ollama_load_balancer

EXPOSE 11434

ENTRYPOINT ["/usr/local/bin/ollama_load_balancer"]
