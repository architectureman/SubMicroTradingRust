use clap::Parser;
use smt_core::common_types::{OrderType, Side, Symbol, TimeInForce};
use smt_core::protocol_defs::fix_messages::{FixMessageBody, FixNewOrderSingle, FixLogon, FixExecutionReport};
use smt_core::common_types::OrderStatus;
use smt_core::codec_engine::fix_codec::{encode_fix_message, decode_fix_message};
use smt_io_adapters::network_tcp::{TcpConnectionManager, read_message, write_message, NetworkError};
use smt_io_adapters::file_logger;
use tokio::net::{TcpListener, TcpStream};
// Remove these unused imports
// Xóa các dòng sau
// use tokio::io::{AsyncReadExt, AsyncWriteExt};
// use tracing::{debug};
use bytes::BytesMut;
use rust_decimal_macros::dec;
use tracing::{info, error, warn};
use std::error::Error;

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
    },
    /// Runs the OMS simulator in client mode to send a test order
    Client {
        #[clap(short, long, default_value = "0.0.0.0:3000")]
        server_addr: String,
        #[clap(long, default_value = "TESTCLIENT")]
        sender_comp_id: String,
        #[clap(long, default_value = "TESTSERVER")]
        target_comp_id: String,
    },
}

async fn run_server(listen_addr: String) -> Result<(), Box<dyn Error + Send + Sync>> {
    info!("Starting OMS Simulator in SERVER mode, listening on {}", listen_addr);
    let listener = TcpListener::bind(&listen_addr).await?;
    info!("Server listening on {}", listener.local_addr()?);

    loop {
        match listener.accept().await {
            Ok((socket, addr)) => {
                info!("Accepted connection from: {}", addr);
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(socket).await {
                        error!("Error handling connection from {}: {}", addr, e);
                    }
                });
            }
            Err(e) => {
                error!("Failed to accept connection: {}", e);
            }
        }
    }
}

async fn handle_connection(mut stream: TcpStream) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut buffer = BytesMut::with_capacity(4096);
    let mut msg_seq_num_counter = 1;
    
    // Thêm các biến này
    let sender_comp_id = "TESTSERVER".to_string();
    let target_comp_id = "TESTCLIENT".to_string();
    
    loop {
        match read_message(&mut stream, &mut buffer).await {
            Ok(Some(received_data)) => {
                // Append received data to a temporary buffer for decoding
                let mut decode_buffer = BytesMut::from(received_data.as_ref());
                match decode_fix_message(&mut decode_buffer) {
                    Ok(Some(fix_message)) => {
                        info!("Server Received FIX Message: {:?}", fix_message);
                        // Process the message and potentially send a response
                        match fix_message.body {
                            FixMessageBody::NewOrderSingle(nos) => {
                                info!("Received NewOrderSingle: {:?}", nos);
                                let exec_report = FixExecutionReport {
                                    // Thay thế format!() bằng một giá trị u64
                                    order_id: chrono::Utc::now().timestamp_millis() as u64, // Sử dụng timestamp làm ID
                                    cl_ord_id: Some(nos.cl_ord_id.clone()),
                                    exec_id: format!("EID_{}", chrono::Utc::now().timestamp_millis()),
                                    // Sửa lỗi: Xóa trường exec_type không tồn tại
                                    ord_status: OrderStatus::New,
                                    symbol: nos.symbol.clone(),
                                    side: nos.side,
                                    leaves_qty: nos.order_qty,
                                    cum_qty: dec!(0),
                                    avg_px: dec!(0),
                                    last_px: None,
                                    last_qty: None,
                                    // Sửa lỗi: Không bọc trong Some()
                                    transact_time: chrono::Utc::now().timestamp_millis().try_into().unwrap(),
                                    text: Some("Order Accepted".to_string()),
                                };
                                let response_bytes = encode_fix_message(
                                    &exec_report,
                                    &fix_message.header.target_comp_id,
                                    &fix_message.header.sender_comp_id,
                                    msg_seq_num_counter,
                                    chrono::Utc::now().timestamp_millis().try_into().unwrap()
                                )?;
                                msg_seq_num_counter += 1;
                                write_message(&mut stream, &response_bytes).await?;
                                info!("Sent ExecutionReport for ClOrdID: {}", nos.cl_ord_id);
                            }
                            FixMessageBody::Logon(logon_msg) => {
                                info!("Received Logon: {:?}", logon_msg);
                                // Respond with a Logon message
                                let logon_response = FixLogon {
                                    encrypt_method: 0,
                                    heart_bt_int: 30,
                                    reset_seq_num_flag: Some(false),
                                };
                                let response_bytes = encode_fix_message(
                                    &logon_response,
                                    &fix_message.header.target_comp_id,
                                    &fix_message.header.sender_comp_id,
                                    msg_seq_num_counter,
                                    chrono::Utc::now().timestamp_millis().try_into().unwrap()
                                )?;
                                msg_seq_num_counter += 1;
                                write_message(&mut stream, &response_bytes).await?;
                                info!("Sent Logon response");
                            }
                            _ => {
                                warn!("Received unhandled FIX message type: {}", fix_message.header.msg_type);
                            }
                        }
                    }
                    Ok(None) => {
                        info!("No complete FIX message decoded from received data or buffer is empty.");
                    }
                    Err(e) => {
                        error!("Failed to decode FIX message: {}", e);
                        // Potentially send a Reject message here
                    }
                }
            }
            Ok(None) => {
                info!("Connection closed by peer or no data.");
                break; // Connection closed or no data
            }
            Err(NetworkError::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Spurious wakeup, continue
                continue;
            }
            Err(e) => {
                error!("Error reading from stream: {}", e);
                break;
            }
        }
    }
    Ok(())
}

async fn run_client(server_addr: String, sender_comp_id: String, target_comp_id: String) -> Result<(), Box<dyn Error + Send + Sync>> {
    info!("Starting OMS Simulator in CLIENT mode, connecting to {}", server_addr);
    let mut stream = TcpConnectionManager::connect(&server_addr).await?;
    info!("Client connected to server: {}", server_addr);

    // 1. Send Logon
    let logon_msg = FixLogon {
        encrypt_method: 0,
        heart_bt_int: 30,
        reset_seq_num_flag: Some(true),
    };
    let logon_bytes = encode_fix_message(&logon_msg, &sender_comp_id, &target_comp_id, 1, chrono::Utc::now().timestamp_millis().try_into().unwrap())?;
    write_message(&mut stream, &logon_bytes).await?;
    info!("Sent Logon message");

    // Wait for and decode Logon response
    let mut response_buffer = BytesMut::with_capacity(1024);
    match read_message(&mut stream, &mut response_buffer).await {
        Ok(Some(data)) => {
            let mut decode_buffer = BytesMut::from(data.as_ref());
            match decode_fix_message(&mut decode_buffer) {
                Ok(Some(fix_resp)) if fix_resp.header.msg_type == "A" => {
                    info!("Received Logon response: {:?}", fix_resp.body);
                }
                Ok(Some(other_msg)) => {
                    warn!("Received unexpected message after Logon: {:?}", other_msg);
                }
                Ok(None) => warn!("No complete message decoded for Logon response"),
                Err(e) => error!("Failed to decode Logon response: {}", e),
            }
        }
        Ok(None) => warn!("No response received after Logon"),
        Err(e) => error!("Error reading Logon response: {}", e),
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
    let order_bytes = encode_fix_message(&new_order, &sender_comp_id, &target_comp_id, 2, chrono::Utc::now().timestamp_millis().try_into().unwrap())?;
    write_message(&mut stream, &order_bytes).await?;
    info!("Sent NewOrderSingle: {:?}", new_order);

    // Wait for and decode ExecutionReport
    response_buffer.clear();
    match read_message(&mut stream, &mut response_buffer).await {
        Ok(Some(data)) => {
            let mut decode_buffer = BytesMut::from(data.as_ref());
            match decode_fix_message(&mut decode_buffer) {
                Ok(Some(fix_resp)) if fix_resp.header.msg_type == "8" => {
                    info!("Received ExecutionReport: {:?}", fix_resp.body);
                }
                Ok(Some(other_msg)) => {
                    warn!("Received unexpected message after NewOrderSingle: {:?}", other_msg);
                }
                Ok(None) => warn!("No complete message decoded for ExecutionReport"),
                Err(e) => error!("Failed to decode ExecutionReport: {}", e),
            }
        }
        Ok(None) => warn!("No ExecutionReport received"),
        Err(e) => error!("Error reading ExecutionReport: {}", e),
    }

    info!("Client operations finished.");
    Ok(())
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Initialize logger (from smt_io_adapters)
    if let Err(e) = file_logger::init_logger() {
        eprintln!("Failed to initialize logger: {}", e);
        // Decide if you want to proceed without logging or exit
    }

    let args = Args::parse();

    match args.command {
        Commands::Server { listen_addr } => {
            if let Err(e) = run_server(listen_addr).await {
                error!("Server error: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Client { server_addr, sender_comp_id, target_comp_id } => {
            if let Err(e) = run_client(server_addr, sender_comp_id, target_comp_id).await {
                error!("Client error: {}", e);
                std::process::exit(1);
            }
        }
    }
    Ok(())
}
// Xóa hoàn toàn 3 dòng khai báo biến toàn cục ở đây

