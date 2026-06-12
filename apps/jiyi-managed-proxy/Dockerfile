FROM rust:1-bookworm AS builder

WORKDIR /workspace
COPY . .
RUN cargo build --release -p jiyi-managed-proxy

FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/* \
  && useradd --system --home-dir /var/lib/jiyi-codex --create-home --shell /usr/sbin/nologin jiyi-codex

COPY --from=builder /workspace/target/release/jiyi-managed-proxy /usr/local/bin/jiyi-managed-proxy

ENV JIYI_MANAGED_PROXY_LISTEN=0.0.0.0:8080
ENV JIYI_MANAGED_PROXY_UPSTREAM_BASE_URL=https://dashscope.aliyuncs.com/compatible-mode/v1
ENV JIYI_MANAGED_PROXY_UPSTREAM_API_KEY=""
ENV JIYI_MANAGED_PROXY_SYNC_API_KEY=""
ENV JIYI_MANAGED_PROXY_ADMIN_API_KEY=""
ENV JIYI_MANAGED_PROXY_USER_READ_API_KEY=""
ENV JIYI_MANAGED_PROXY_BILLING_API_KEY=""
ENV JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY=""
ENV JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET=""
ENV JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY=""
ENV JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY_PATH=""
ENV JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY=""
ENV JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY_PATH=""
ENV JIYI_MANAGED_PROXY_ACCESS_API_KEY=""
ENV JIYI_MANAGED_PROXY_AUDIT_API_KEY=""
ENV JIYI_MANAGED_PROXY_DB_PATH=/var/lib/jiyi-codex/jiyi-codex-backend.sqlite

USER jiyi-codex
WORKDIR /var/lib/jiyi-codex
VOLUME ["/var/lib/jiyi-codex"]
EXPOSE 8080

CMD ["/usr/local/bin/jiyi-managed-proxy"]
