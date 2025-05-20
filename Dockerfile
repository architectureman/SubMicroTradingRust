# Giai đoạn 1: Builder
# Sử dụng một image Rust chính thức với phiên bản ổn định gần đây
FROM rust:1.70-slim AS builder

WORKDIR /usr/src/app

# Sao chép file manifest của workspace trước để tận dụng Docker layer caching cho dependencies
COPY SubMicroTradingRust_workspace/Cargo.toml SubMicroTradingRust_workspace/Cargo.toml
COPY SubMicroTradingRust_workspace/Cargo.lock SubMicroTradingRust_workspace/Cargo.lock

# Sao chép manifest của các crates con
COPY SubMicroTradingRust_workspace/smt_core/Cargo.toml SubMicroTradingRust_workspace/smt_core/Cargo.toml
COPY SubMicroTradingRust_workspace/smt_io_adapters/Cargo.toml SubMicroTradingRust_workspace/smt_io_adapters/Cargo.toml
COPY SubMicroTradingRust_workspace/smt_oms_simulation/Cargo.toml SubMicroTradingRust_workspace/smt_oms_simulation/Cargo.toml

# Build dependencies trước để cache layer này
# Tạo thư mục src rỗng cho các crates để cargo có thể build dependencies mà không cần toàn bộ source code
RUN mkdir -p SubMicroTradingRust_workspace/smt_core/src && \
    mkdir -p SubMicroTradingRust_workspace/smt_io_adapters/src && \
    mkdir -p SubMicroTradingRust_workspace/smt_oms_simulation/src && \
    mkdir -p SubMicroTradingRust_workspace/tests && \
    echo "// Dummy lib.rs for smt_core" > SubMicroTradingRust_workspace/smt_core/src/lib.rs && \
    echo "// Dummy lib.rs for smt_io_adapters" > SubMicroTradingRust_workspace/smt_io_adapters/src/lib.rs && \
    echo "fn main() {println!(\"Dummy main for smt_oms_simulation\");}" > SubMicroTradingRust_workspace/smt_oms_simulation/src/main.rs && \
    cd SubMicroTradingRust_workspace && cargo build --release --workspace && rm -rf target

# Sao chép toàn bộ mã nguồn của workspace
COPY SubMicroTradingRust_workspace/ SubMicroTradingRust_workspace/

# Build toàn bộ workspace cho release, bao gồm binary oms_simulator
RUN cd SubMicroTradingRust_workspace && cargo build --release --bin smt_oms_simulation

# Giai đoạn 2: Runtime
FROM debian:bullseye-slim

WORKDIR /usr/local/bin

# Sao chép binary oms_simulator đã được build từ giai đoạn builder.
COPY --from=builder /usr/src/app/SubMicroTradingRust_workspace/target/release/smt_oms_simulation /usr/local/bin/oms_simulator

# Thiết lập entrypoint cho container
# Mặc định chạy server mode, có thể override bằng docker run command
CMD ["./oms_simulator", "server"]

