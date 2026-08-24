# syntax=docker/dockerfile:1

# Swift is the least portable server in the matrix. Its official image also
# gives us sourcekit-lsp, so the other toolchains are installed around it.
FROM swift:6.1-jammy

ARG TARGETARCH
ARG GO_VERSION=1.24.4
ARG GOPLS_VERSION=v0.19.1
ARG MARKSMAN_VERSION=2024-12-18
ARG NODE_VERSION=22.16.0
ARG PYRIGHT_VERSION=1.1.402
ARG RUST_VERSION=1.88.0
ARG TYPESCRIPT_LANGUAGE_SERVER_VERSION=4.3.4
ARG TYPESCRIPT_VERSION=5.8.3

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
        build-essential \
        ca-certificates \
        clangd \
        curl \
        git \
        pkg-config \
        xz-utils \
    && rm -rf /var/lib/apt/lists/*

RUN case "${TARGETARCH}" in \
        amd64) tool_arch=amd64; node_arch=x64; marksman_arch=x64 ;; \
        arm64) tool_arch=arm64; node_arch=arm64; marksman_arch=arm64 ;; \
        *) echo "unsupported Docker architecture: ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
    && curl --fail --location --silent --show-error \
        "https://go.dev/dl/go${GO_VERSION}.linux-${tool_arch}.tar.gz" \
        | tar --extract --gzip --directory /usr/local \
    && curl --fail --location --silent --show-error \
        "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-${node_arch}.tar.xz" \
        | tar --extract --xz --directory /usr/local --strip-components=1 \
    && curl --fail --location --silent --show-error \
        "https://github.com/artempyanykh/marksman/releases/download/${MARKSMAN_VERSION}/marksman-linux-${marksman_arch}" \
        --output /usr/local/bin/marksman \
    && chmod +x /usr/local/bin/marksman

ENV PATH="/usr/local/go/bin:/root/.cargo/bin:${PATH}"

RUN curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
        https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --default-toolchain "${RUST_VERSION}" \
    && rustup component add rust-analyzer rustfmt --toolchain "${RUST_VERSION}" \
    && npm install --global \
        "pyright@${PYRIGHT_VERSION}" \
        "typescript@${TYPESCRIPT_VERSION}" \
        "typescript-language-server@${TYPESCRIPT_LANGUAGE_SERVER_VERSION}" \
    && GOBIN=/usr/local/bin go install "golang.org/x/tools/gopls@${GOPLS_VERSION}"

WORKDIR /runyte
COPY . .

# Compile during the image build so `run` is only the compatibility check.
RUN cargo test --test lsp_real_servers --no-run

ENV RUNYTE_SMOKE_LSP_ALL=1

CMD ["cargo", "test", "--test", "lsp_real_servers", "--", "--ignored", "--nocapture", "--test-threads=1"]
