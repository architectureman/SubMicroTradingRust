# Giai đoạn 1: Builder
# Sử dụng một image Rust chính thức với phiên bản ổn định gần đây
FROM rust:1.87.0-slim-bullseye AS builder

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
    # Create a valid workspace Cargo.toml
    echo '[workspace]\nmembers = ["smt_core", "smt_io_adapters", "smt_oms_simulation"]' > SubMicroTradingRust_workspace/Cargo.toml && \
    # Create a valid smt_core Cargo.toml
    echo '[package]\nname = "smt_core"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\nbytes = "1"\nrust_decimal = { version = "1.33", features = ["macros"] }\nrust_decimal_macros = "1.33"\nthiserror = "1.0"' > SubMicroTradingRust_workspace/smt_core/Cargo.toml && \
    # Create a valid smt_io_adapters Cargo.toml
    echo '[package]\nname = "smt_io_adapters"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\ntokio = { version = "1", features = ["full"] }\ntracing = "0.1"\ntracing-subscriber = { version = "0.3", features = ["env-filter"] }\nbytes = "1"\nthiserror = "1.0"\nlibc = "0.2"\ncore_affinity = "0.8"' > SubMicroTradingRust_workspace/smt_io_adapters/Cargo.toml && \
    # Create a valid smt_oms_simulation Cargo.toml
    echo '[package]\nname = "smt_oms_simulation"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\nsmt_core = { path = "../smt_core" }\nsmt_io_adapters = { path = "../smt_io_adapters" }\ntokio = { version = "1", features = ["full"] }\ntracing = "0.1"\nrust_decimal = "1"\nrust_decimal_macros = "1"\nclap = { version = "4", features = ["derive"] }\nbytes = "1"\ncore_affinity = "0.8"' > SubMicroTradingRust_workspace/smt_oms_simulation/Cargo.toml && \
    # Create dummy source files
    echo "// Dummy lib.rs for smt_core" > SubMicroTradingRust_workspace/smt_core/src/lib.rs && \
    echo "// Dummy lib.rs for smt_io_adapters" > SubMicroTradingRust_workspace/smt_io_adapters/src/lib.rs && \
    echo 'fn main() {println!("Dummy main for smt_oms_simulation");}' > SubMicroTradingRust_workspace/smt_oms_simulation/src/main.rs && \
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
CMD ["./oms_simulator", "server", "--listen-addr", "0.0.0.0:3000"]

