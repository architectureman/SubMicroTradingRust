use clap::Parser;
use smt_core::common_types::{OrderType, Side, Symbol, TimeInForce, OrderStatus};
use smt_core::protocol_defs::fix_messages::{FixMessageBody, FixNewOrderSingle, FixLogon, FixExecutionReport};
use smt_core::codec_engine::fix_codec::{encode_fix_message, decode_fix_message, get_codec_metrics};
use smt_io_adapters::network_tcp::{TcpConnectionManager, read_message, write_message, get_network_metrics};
use smt_io_adapters::file_logger;
use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt}; // Thêm AsyncWriteExt
use bytes::BytesMut;
use rust_decimal_macros::dec;
use tracing::{info, error, warn, debug};
use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, Duration};
use tokio::runtime::Runtime;
use tokio::sync::{mpsc, Semaphore};
use std::thread;
use core_affinity::{get_core_ids, set_for_current};

// Performance metrics
static ORDERS_PROCESSED: AtomicU64 = AtomicU64::new(0);
static TOTAL_LATENCY_MICROS: AtomicU64 = AtomicU64::new(0);
static MIN_LATENCY_MICROS: AtomicU64 = AtomicU64::new(u64::MAX);
static MAX_LATENCY_MICROS: AtomicU64 = AtomicU64::new(0);

// Message types for inter-thread communication
enum NetworkMessage {
    Data { buffer_index: usize, data: BytesMut },
    Disconnect,
}

enum ProcessingMessage {
    Response { buffer_index: usize, data: BytesMut },
    Disconnect,
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Parser, Debug)]
enum Commands {
    /// Runs the OMS simulator in server mode
    Server {
        #[clap(short, long, default_value = "0.0.0.0:3000")]
        listen_addr: String,
        
        #[clap(short, long, default_value = "10")]
        worker_threads: usize,
        
        #[clap(long, default_value = "false")]
        pin_cores: bool,
    },
    /// Runs the OMS simulator in client mode to send a test order
    Client {
        #[clap(short, long, default_value = "0.0.0.0:3000")]
        server_addr: String,
        
        #[clap(long, default_value = "TESTCLIENT")]
        sender_comp_id: String,
        
        #[clap(long, default_value = "TESTSERVER")]
        target_comp_id: String,
        
        #[clap(short, long, default_value = "1")]
        num_orders: usize,
        
        #[clap(short, long, default_value = "false")]
        benchmark: bool,
    },
    /// Runs a performance benchmark 
    Benchmark {
        #[clap(short, long, default_value = "127.0.0.1:3000")]
        server_addr: String,
        
        #[clap(short, long, default_value = "1000")]
        num_orders: usize,
        
        #[clap(short, long, default_value = "10")]
        concurrency: usize,
        
        #[clap(long, default_value = "BENCHMARKCLIENT")]
        sender_comp_id: String,
        
        #[clap(long, default_value = "TESTSERVER")]
        target_comp_id: String,
        
        #[clap(long, default_value = "0")]
        rate_limit: u64,
    }
}

// HFT-optimized server implementation with separate threads for networking and processing
async fn run_server(listen_addr: String, worker_threads: usize, pin_cores: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
    info!("Starting OMS Simulator in HFT SERVER mode, listening on {}", listen_addr);
    
    // Create a multi-threaded runtime for processing
    let processing_rt = Runtime::new()?;
    
    // Create a listener
    let listener = TcpListener::bind(&listen_addr).await?;
    info!("Server listening on {}", listener.local_addr()?);
    
    // Create a semaphore to limit concurrent connections
    let connection_semaphore = Arc::new(Semaphore::new(worker_threads * 10)); // Allow 10x concurrent connections per worker
    
    // Start a performance monitoring thread
    let _monitor_handle = thread::spawn(|| {
        let mut last_time = Instant::now();
        let mut last_orders = 0;
        
        loop {
            thread::sleep(Duration::from_secs(1));
            
            let current_orders = ORDERS_PROCESSED.load(Ordering::Relaxed);
            let orders_per_second = current_orders - last_orders;
            last_orders = current_orders;
            
            let now = Instant::now();
            let elapsed = now.duration_since(last_time);
            last_time = now;
            
            let avg_latency = if current_orders > 0 {
                TOTAL_LATENCY_MICROS.load(Ordering::Relaxed) as f64 / current_orders as f64
            } else {
                0.0
            };
            
            let min_latency = MIN_LATENCY_MICROS.load(Ordering::Relaxed);
            let max_latency = MAX_LATENCY_MICROS.load(Ordering::Relaxed);
            
            let (avg_encode_micros, avg_decode_micros, _encode_count, _decode_count) = get_codec_metrics();
            let (avg_read_micros, avg_write_micros, _read_count, _write_count) = get_network_metrics();
            
            info!(
                "Performance: {:.2} orders/sec, Latency avg: {:.2}µs min: {}µs max: {}µs, Codec encode: {:.2}µs decode: {:.2}µs, Network read: {:.2}µs write: {:.2}µs",
                orders_per_second as f64 / elapsed.as_secs_f64(),
                avg_latency,
                min_latency,
                max_latency,
                avg_encode_micros,
                avg_decode_micros,
                avg_read_micros,
                avg_write_micros
            );
        }
    });
    
    // Pin the main thread to a specific core if requested
    if pin_cores {
        let core_ids = get_core_ids().unwrap_or_default();
        if !core_ids.is_empty() {
            if set_for_current(core_ids[0]) {
                info!("Main thread pinned to core ID: {}", core_ids[0].id);
            }
        }
    }
    
    // Accept loop
    loop {
        // Wait for a connection
        let permit = connection_semaphore.clone().acquire_owned().await?;
        
        match listener.accept().await {
            Ok((socket, addr)) => {
                info!("Accepted connection from: {}", addr);
                
                // Configure socket for HFT performance
                socket.set_nodelay(true)?;
                
                // Clone runtime handle for the connection handler
                let processing_rt = processing_rt.handle().clone();
                
                // Spawn a task to handle the connection
                tokio::spawn(async move {
                    // The permit will be dropped when the task completes
                    let _permit = permit;
                    
                    // Create channels for network<->processing communication
                    let (net_tx, mut proc_rx) = mpsc::channel::<NetworkMessage>(1000); // Thêm mut ở đây
                    let (proc_tx, mut net_rx) = mpsc::channel::<ProcessingMessage>(1000);
                    
                    // Sử dụng into_split để tách socket thành hai phần
                    let (mut reader_socket, mut writer_socket) = socket.into_split();
                    
                    // Spawn network reader thread
                    let reader_net_tx = net_tx.clone();
                    
                    let reader_handle = tokio::spawn(async move {
                        let mut buffer = BytesMut::with_capacity(8192);
                        
                        loop {
                            match reader_socket.read_buf(&mut buffer).await {
                                Ok(0) => {
                                    // Connection closed
                                    let _ = reader_net_tx.send(NetworkMessage::Disconnect).await;
                                    break;
                                }
                                Ok(n) => {
                                    debug!("Read {} bytes from network", n);
                                    
                                    // Clone the buffer to send to the processing thread
                                    let data = buffer.split().freeze();
                                    
                                    // Send the data to the processing thread
                                    let _ = reader_net_tx.send(NetworkMessage::Data {
                                        buffer_index: 0, // Not using buffer pool here for simplicity
                                        data: BytesMut::from(&data[..]),
                                    }).await;
                                }
                                Err(e) => {
                                    error!("Error reading from socket: {}", e);
                                    break;
                                }
                            }
                        }
                        
                        debug!("Network reader thread exiting");
                    });
                    
                    // Spawn network writer thread
                    let writer_handle = tokio::spawn(async move {
                        while let Some(msg) = net_rx.recv().await {
                            match msg {
                                ProcessingMessage::Response { data, .. } => {
                                    if let Err(e) = writer_socket.write_all(&data).await {
                                        error!("Error writing to socket: {}", e);
                                        break;
                                    }
                                }
                                ProcessingMessage::Disconnect => {
                                    break;
                                }
                            }
                        }
                        
                        debug!("Network writer thread exiting");
                    });
                    
                    // Spawn processing thread for this connection
                    let proc_handle = processing_rt.spawn(async move {
                        let mut msg_seq_num_counter = 1;
                        
                        while let Some(msg) = proc_rx.recv().await {
                            match msg {
                                NetworkMessage::Data { buffer_index, mut data } => {
                                    let start = Instant::now();
                                    
                                    // Decode the FIX message
                                    match decode_fix_message(&mut data) {
                                        Ok(Some(fix_message)) => {
                                            debug!("Decoded FIX message: {}", fix_message.header.msg_type);
                                            
                                            // Process the message based on its type
                                            match fix_message.body {
                                                FixMessageBody::NewOrderSingle(nos) => {
                                                    // Create execution report
                                                    let exec_report = FixExecutionReport {
                                                        order_id: chrono::Utc::now().timestamp_millis() as u64,
                                                        cl_ord_id: Some(nos.cl_ord_id.clone()),
                                                        exec_id: format!("EID_{}", chrono::Utc::now().timestamp_millis()),
                                                        ord_status: OrderStatus::New,
                                                        symbol: nos.symbol.clone(),
                                                        side: nos.side,
                                                        leaves_qty: nos.order_qty,
                                                        cum_qty: dec!(0),
                                                        avg_px: dec!(0),
                                                        last_px: None,
                                                        last_qty: None,
                                                        transact_time: chrono::Utc::now().timestamp_millis().try_into().unwrap(),
                                                        text: Some("Order Accepted".to_string()),
                                                    };
                                                    
                                                    // Encode response
                                                    let response_bytes = encode_fix_message(
                                                        &exec_report,
                                                        &fix_message.header.target_comp_id,
                                                        &fix_message.header.sender_comp_id,
                                                        msg_seq_num_counter,
                                                        chrono::Utc::now().timestamp_millis().try_into().unwrap()
                                                    ).unwrap();
                                                    
                                                    msg_seq_num_counter += 1;
                                                    
                                                    // Send response
                                                    let _ = proc_tx.send(ProcessingMessage::Response {
                                                        buffer_index,
                                                        data: BytesMut::from(&response_bytes[..]),
                                                    }).await;
                                                    
                                                    let elapsed = start.elapsed();
                                                    let latency_micros = elapsed.as_micros() as u64;
                                                    
                                                    // Update performance metrics
                                                    ORDERS_PROCESSED.fetch_add(1, Ordering::Relaxed);
                                                    TOTAL_LATENCY_MICROS.fetch_add(latency_micros, Ordering::Relaxed);
                                                    
                                                    // Update min/max latency
                                                    let mut current_min = MIN_LATENCY_MICROS.load(Ordering::Relaxed);
                                                    while latency_micros < current_min {
                                                        match MIN_LATENCY_MICROS.compare_exchange(
                                                            current_min,
                                                            latency_micros,
                                                            Ordering::SeqCst,
                                                            Ordering::SeqCst,
                                                        ) {
                                                            Ok(_) => break,
                                                            Err(latest) => current_min = latest,
                                                        }
                                                    }
                                                    
                                                    let mut current_max = MAX_LATENCY_MICROS.load(Ordering::Relaxed);
                                                    while latency_micros > current_max {
                                                        match MAX_LATENCY_MICROS.compare_exchange(
                                                            current_max,
                                                            latency_micros,
                                                            Ordering::SeqCst,
                                                            Ordering::SeqCst,
                                                        ) {
                                                            Ok(_) => break,
                                                            Err(latest) => current_max = latest,
                                                        }
                                                    }
                                                    
                                                    debug!("Order processed in {}µs", latency_micros);
                                                }
                                                FixMessageBody::Logon(_logon_msg) => { // Thêm dấu gạch dưới
                                                    // Create logon response
                                                    let logon_response = FixLogon {
                                                        encrypt_method: 0,
                                                        heart_bt_int: 30,
                                                        reset_seq_num_flag: Some(false),
                                                    };
                                                    
                                                    // Encode response
                                                    let response_bytes = encode_fix_message(
                                                        &logon_response,
                                                        &fix_message.header.target_comp_id,
                                                        &fix_message.header.sender_comp_id,
                                                        msg_seq_num_counter,
                                                        chrono::Utc::now().timestamp_millis().try_into().unwrap()
                                                    ).unwrap();
                                                    
                                                    msg_seq_num_counter += 1;
                                                    
                                                    // Send response
                                                    let _ = proc_tx.send(ProcessingMessage::Response {
                                                        buffer_index,
                                                        data: BytesMut::from(&response_bytes[..]),
                                                    }).await;
                                                }
                                                _ => {
                                                    warn!("Unhandled message type: {}", fix_message.header.msg_type);
                                                }
                                            }
                                        }
                                        Ok(None) => {
                                            debug!("Incomplete FIX message");
                                        }
                                        Err(e) => {
                                            error!("Error decoding FIX message: {}", e);
                                        }
                                    }
                                }
                                NetworkMessage::Disconnect => {
                                    let _ = proc_tx.send(ProcessingMessage::Disconnect).await;
                                    break;
                                }
                            }
                        }
                        
                        debug!("Processing thread exiting");
                    });
                    
                    // Wait for all threads to complete
                    let _ = tokio::try_join!(reader_handle, writer_handle);
                    
                    // Cancel the processing thread if it's still running
                    proc_handle.abort();
                    
                    debug!("Connection handler exiting");
                });
            }
            Err(e) => {
                error!("Error accepting connection: {}", e);
            }
        }
    }
}

// Client to send test orders
async fn run_client(
    server_addr: String,
    sender_comp_id: String,
    target_comp_id: String,
    num_orders: usize,
    benchmark: bool
) -> Result<(), Box<dyn Error + Send + Sync>> {
    info!("Starting OMS Simulator in CLIENT mode, connecting to {}", server_addr);
    
    let total_start = Instant::now();
    let mut total_latency_micros = 0;
    let mut min_latency_micros = u64::MAX;
    let mut max_latency_micros = 0;
    
    for i in 0..num_orders {
        let mut stream = TcpConnectionManager::connect(&server_addr).await?;
        let order_start = Instant::now();
        
        // 1. Send Logon
        let logon_msg = FixLogon {
            encrypt_method: 0,
            heart_bt_int: 30,
            reset_seq_num_flag: Some(true),
        };
        
        let logon_bytes = encode_fix_message(
            &logon_msg,
            &sender_comp_id,
            &target_comp_id,
            1,
            chrono::Utc::now().timestamp_millis().try_into().unwrap()
        )?;
        
        write_message(&mut stream, &logon_bytes).await?;
        
        if !benchmark {
            info!("Sent Logon message");
        }
        
        // Wait for Logon response
        let mut response_buffer = BytesMut::with_capacity(1024);
        let logon_response = read_message(&mut stream, &mut response_buffer).await?;
        
        if logon_response.is_none() {
            error!("No logon response received");
            continue;
        }
        
        // 2. Send NewOrderSingle
        let new_order = FixNewOrderSingle {
            cl_ord_id: format!("TestOrd_{}", chrono::Utc::now().timestamp_micros()),
            symbol: Symbol::new("BTC/USD"),
            side: Side::Buy,
            transact_time: chrono::Utc::now().timestamp_millis().try_into().unwrap(),
            order_qty: dec!(1.5),
            ord_type: OrderType::Limit,
            price: Some(dec!(50000.0)),
            tif: Some(TimeInForce::GTC),
        };
        
        let order_bytes = encode_fix_message(
            &new_order,
            &sender_comp_id,
            &target_comp_id,
            2,
            chrono::Utc::now().timestamp_millis().try_into().unwrap()
        )?;
        
        write_message(&mut stream, &order_bytes).await?;
        
        if !benchmark {
            info!("Sent NewOrderSingle: {:?}", new_order);
        }
        
        // Wait for ExecutionReport
        response_buffer.clear();
        let exec_response = read_message(&mut stream, &mut response_buffer).await?;
        
        let order_latency = order_start.elapsed();
        let latency_micros = order_latency.as_micros() as u64;
        
        total_latency_micros += latency_micros;
        min_latency_micros = min_latency_micros.min(latency_micros);
        max_latency_micros = max_latency_micros.max(latency_micros);
        
        if !benchmark && exec_response.is_some() {
            info!("Received ExecutionReport in {}µs", latency_micros);
        } else if exec_response.is_none() {
            error!("No ExecutionReport received");
        }
        
        if benchmark && i % 100 == 0 {
            info!("Processed {} orders", i + 1);
        }
    }
    
    let total_elapsed = total_start.elapsed();
    let avg_latency = total_latency_micros as f64 / num_orders as f64;
    let throughput = num_orders as f64 / total_elapsed.as_secs_f64();
    
    info!("Client operations finished.");
    info!("Sent {} orders in {:.2?}", num_orders, total_elapsed);
    info!("Throughput: {:.2} orders/second", throughput);
    info!("Latency: avg={:.2}µs, min={}µs, max={}µs", avg_latency, min_latency_micros, max_latency_micros);
    
    Ok(())
}

// Benchmark with multiple concurrent orders
async fn run_benchmark(
    server_addr: String,
    num_orders: usize,
    concurrency: usize,
    sender_comp_id: String,
    target_comp_id: String,
    rate_limit: u64
) -> Result<(), Box<dyn Error + Send + Sync>> {
    info!("Starting benchmark with {} orders, {} concurrent connections", num_orders, concurrency);
    
    // Set up progress tracking
    let orders_per_connection = num_orders / concurrency;
    let remainder = num_orders % concurrency;
    
    // Set up rate limiting if requested
    let mut rate_limiter = if rate_limit > 0 {
        Some(tokio::time::interval(Duration::from_millis(1000 / rate_limit)))
    } else {
        None
    };
    
    // Create a semaphore to limit concurrency
    let semaphore = Arc::new(Semaphore::new(concurrency));
    
    // Create a vector to hold the join handles
    let mut handles = Vec::with_capacity(concurrency);
    
    // Start time for throughput calculation
    let benchmark_start = Instant::now();
    
    // Create tasks for each connection
    for i in 0..concurrency {
        // Determine how many orders this connection will send
        let orders_to_send = if i < remainder {
            orders_per_connection + 1
        } else {
            orders_per_connection
        };
        
        // Skip connections with no orders
        if orders_to_send == 0 {
            continue;
        }
        
        // Clone data for the task
        let server_addr = server_addr.clone();
        let sender_comp_id = sender_comp_id.clone();
        let target_comp_id = target_comp_id.clone();
        let semaphore = semaphore.clone();
        
        // Rate limiting
        if let Some(limiter) = &mut rate_limiter {
            limiter.tick().await;
        }
        
        // Spawn task
        let handle = tokio::spawn(async move {
            // Acquire permit
            let _permit = semaphore.acquire().await.unwrap();
            
            // Connect to server
            let mut stream = match TcpConnectionManager::connect(&server_addr).await {
                Ok(stream) => stream,
                Err(e) => {
                    error!("Failed to connect to server: {}", e);
                    return;
                }
            };
            
            // Send logon
            let logon_msg = FixLogon {
                encrypt_method: 0,
                heart_bt_int: 30,
                reset_seq_num_flag: Some(true),
            };
            
            let logon_bytes = match encode_fix_message(
                &logon_msg,
                &sender_comp_id,
                &target_comp_id,
                1,
                chrono::Utc::now().timestamp_millis().try_into().unwrap()
            ) {
                Ok(bytes) => bytes,
                Err(e) => {
                    error!("Failed to encode logon message: {}", e);
                    return;
                }
            };
            
            if let Err(e) = write_message(&mut stream, &logon_bytes).await {
                error!("Failed to send logon message: {}", e);
                return;
            }
            
            // Wait for logon response
            let mut response_buffer = BytesMut::with_capacity(1024);
            match read_message(&mut stream, &mut response_buffer).await {
                Ok(Some(_)) => {},
                Ok(None) => {
                    error!("No logon response received");
                    return;
                }
                Err(e) => {
                    error!("Failed to read logon response: {}", e);
                    return;
                }
            }
            
            // Send orders
            for j in 0..orders_to_send {
                let start = Instant::now();
                
                // Create order
                let new_order = FixNewOrderSingle {
                    cl_ord_id: format!("BenchOrd_{}", chrono::Utc::now().timestamp_micros()),
                    symbol: Symbol::new("BTC/USD"),
                    side: if j % 2 == 0 { Side::Buy } else { Side::Sell },
                    transact_time: chrono::Utc::now().timestamp_millis().try_into().unwrap(),
                    order_qty: dec!(1.5),
                    ord_type: OrderType::Limit,
                    price: Some(dec!(50000.0)),
                    tif: Some(TimeInForce::GTC),
                };
                
                // Encode order
                let order_bytes = match encode_fix_message(
                    &new_order,
                    &sender_comp_id,
                    &target_comp_id,
                    j as u32 + 2, // Starting from 2 (after logon)
                    chrono::Utc::now().timestamp_millis().try_into().unwrap()
                ) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        error!("Failed to encode order message: {}", e);
                        continue;
                    }
                };
                
                // Send order
                if let Err(e) = write_message(&mut stream, &order_bytes).await {
                    error!("Failed to send order message: {}", e);
                    continue;
                }
                
                // Wait for response
                response_buffer.clear();
                match read_message(&mut stream, &mut response_buffer).await {
                    Ok(Some(_)) => {
                        let elapsed = start.elapsed();
                        let latency_micros = elapsed.as_micros() as u64;
                        
                        // Update metrics
                        ORDERS_PROCESSED.fetch_add(1, Ordering::Relaxed);
                        TOTAL_LATENCY_MICROS.fetch_add(latency_micros, Ordering::Relaxed);
                        
                        // Update min latency
                        let mut current_min = MIN_LATENCY_MICROS.load(Ordering::Relaxed);
                        while latency_micros < current_min {
                            match MIN_LATENCY_MICROS.compare_exchange(
                                current_min,
                                latency_micros,
                                Ordering::SeqCst,
                                Ordering::SeqCst,
                            ) {
                                Ok(_) => break,
                                Err(latest) => current_min = latest,
                            }
                        }
                        
                        // Update max latency
                        let mut current_max = MAX_LATENCY_MICROS.load(Ordering::Relaxed);
                        while latency_micros > current_max {
                            match MAX_LATENCY_MICROS.compare_exchange(
                                current_max,
                                latency_micros,
                                Ordering::SeqCst,
                                Ordering::SeqCst,
                            ) {
                                Ok(_) => break,
                                Err(latest) => current_max = latest,
                            }
                        }
                    }
                    Ok(None) => {
                        error!("No response received for order");
                    }
                    Err(e) => {
                        error!("Failed to read response: {}", e);
                    }
                }
            }
        });
        
        handles.push(handle);
    }
    
    // Wait for all tasks to complete
    for handle in handles {
        let _ = handle.await;
    }
    
    // Calculate benchmark results
    let benchmark_duration = benchmark_start.elapsed();
    let throughput = num_orders as f64 / benchmark_duration.as_secs_f64();
    
    let orders_processed = ORDERS_PROCESSED.load(Ordering::Relaxed);
    let avg_latency = if orders_processed > 0 {
        TOTAL_LATENCY_MICROS.load(Ordering::Relaxed) as f64 / orders_processed as f64
    } else {
        0.0
    };
    
    let min_latency = MIN_LATENCY_MICROS.load(Ordering::Relaxed);
    let max_latency = MAX_LATENCY_MICROS.load(Ordering::Relaxed);
    
    // Reset metrics for next benchmark
    ORDERS_PROCESSED.store(0, Ordering::Relaxed);
    TOTAL_LATENCY_MICROS.store(0, Ordering::Relaxed);
    MIN_LATENCY_MICROS.store(u64::MAX, Ordering::Relaxed);
    MAX_LATENCY_MICROS.store(0, Ordering::Relaxed);
    
    // Print benchmark results
    info!("\n--- Benchmark Summary ---");
    info!("Total Orders: {}", num_orders);
    info!("Successful Orders: {}", orders_processed);
    info!("Test Duration: {:.2} seconds", benchmark_duration.as_secs_f64());
    info!("Throughput: {:.2} orders/second", throughput);
    info!("\n--- Latency Statistics (µs) ---");
    info!("Average: {:.2}", avg_latency);
    info!("Min: {}", min_latency);
    info!("Max: {}", max_latency);
    
    let (avg_encode_micros, avg_decode_micros, _encode_count, _decode_count) = get_codec_metrics();
    let (avg_read_micros, avg_write_micros, _read_count, _write_count) = get_network_metrics();
    
    info!("\n--- Component Performance ---");
    info!("Codec: encode={:.2}µs ({} calls), decode={:.2}µs ({} calls)", 
        avg_encode_micros, _encode_count, avg_decode_micros, _decode_count);
    info!("Network: read={:.2}µs ({} calls), write={:.2}µs ({} calls)",
        avg_read_micros, _read_count, avg_write_micros, _write_count);
    
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Initialize logger
    if let Err(e) = file_logger::init_logger() {
        eprintln!("Failed to initialize logger: {}", e);
    }
    
    let args = Args::parse();
    
    match args.command {
        Commands::Server { listen_addr, worker_threads, pin_cores } => {
            if let Err(e) = run_server(listen_addr, worker_threads, pin_cores).await {
                error!("Server error: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Client { server_addr, sender_comp_id, target_comp_id, num_orders, benchmark } => {
            if let Err(e) = run_client(server_addr, sender_comp_id, target_comp_id, num_orders, benchmark).await {
                error!("Client error: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Benchmark { server_addr, num_orders, concurrency, sender_comp_id, target_comp_id, rate_limit } => {
            if let Err(e) = run_benchmark(server_addr, num_orders, concurrency, sender_comp_id, target_comp_id, rate_limit).await {
                error!("Benchmark error: {}", e);
                std::process::exit(1);
            }
        }
    }
    
    Ok(())
}