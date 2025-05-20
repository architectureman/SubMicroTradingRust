#![allow(dead_code)] // Allow dead code for now

use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use bytes::{BytesMut, Bytes};
use std::io; // Use std::io::Error for tokio io errors
use std::net::SocketAddr;
use tracing::{error, info, debug};

/// Represents an error that can occur during network operations.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Connection error: {0}")]
    ConnectionError(String),
    #[error("Address parsing error: {0}")]
    AddrParseError(String),
    // Add other specific network errors as needed
}

/// A manager for TCP connections.
/// This is a conceptual struct; actual usage might involve directly using TcpListener/TcpStream
/// or a more sophisticated connection pool.
pub struct TcpConnectionManager;

impl TcpConnectionManager {
    /// Establishes a TCP connection to the specified address.
    pub async fn connect<A: ToSocketAddrs + std::fmt::Debug>(addr: A) -> Result<TcpStream, NetworkError> {
        info!("Attempting to connect to address: {:?}", addr);
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                info!("Successfully connected to {:?}", stream.peer_addr()?);
                Ok(stream)
            }
            Err(e) => {
                error!("Failed to connect: {}", e);
                Err(NetworkError::Io(e))
            }
        }
    }

    /// Listens for incoming TCP connections on the specified address.
    pub async fn listen<A: ToSocketAddrs>(addr: A) -> Result<TcpListener, NetworkError> {
        let listener = TcpListener::bind(addr).await?;
        info!("Listening on {:?}", listener.local_addr()?);
        Ok(listener)
    }
}

/// Reads a message from a TCP stream.
/// This is a basic example; actual message framing (e.g., length-prefixing) would be needed.
/// Assumes messages are newline-terminated for this simple example, or reads up to buffer capacity.
pub async fn read_message(stream: &mut TcpStream, buffer: &mut BytesMut) -> Result<Option<Bytes>, NetworkError> {
    // A simple strategy: read available bytes. Real implementation needs framing.
    // For a framed protocol, you would read the length prefix first, then the message body.
    let mut temp_buf = [0u8; 4096]; // Read in chunks
    match stream.read(&mut temp_buf).await {
        Ok(0) => {
            debug!("Connection closed by peer while reading.");
            Ok(None) // Connection closed
        }
        Ok(n) => {
            buffer.extend_from_slice(&temp_buf[..n]);
            // This is where message framing logic would go.
            // For now, just return what was read if anything.
            if !buffer.is_empty() {
                // Example: if we had a delimiter or fixed length, we'd extract one message.
                // Here, we just return the whole buffer content for simplicity.
                let message = buffer.split().freeze(); // Empties the buffer
                Ok(Some(message))
            } else {
                Ok(None)
            }
        }
        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
            Ok(None) // No data available right now, not an error
        }
        Err(e) => {
            error!("Error reading from stream: {}", e);
            Err(NetworkError::Io(e))
        }
    }
}

/// Writes a message to a TCP stream.
pub async fn write_message(stream: &mut TcpStream, message: &[u8]) -> Result<(), NetworkError> {
    // For a framed protocol, you might prefix the message with its length here.
    match stream.write_all(message).await {
        Ok(_) => {
            stream.flush().await?; // Ensure all data is sent
            Ok(())
        }
        Err(e) => {
            error!("Error writing to stream: {}", e);
            Err(NetworkError::Io(e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;
    use std::time::Duration;
    use crate::file_logger; // Assuming file_logger is in the same crate

    fn setup_logger() {
        // Initialize logger for tests, if not already done by a higher-level test runner
        // This is basic; a real test setup might use tracing_test or a similar crate.
        let _ = file_logger::init_logger();
    }

    #[test]
    fn test_tcp_connection_and_messaging() {
        setup_logger();
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let listener_addr = "127.0.0.1:0".parse::<SocketAddr>().unwrap(); // OS assigns a port
            let listener = TcpConnectionManager::listen(listener_addr).await.expect("Failed to start listener");
            let actual_listener_addr = listener.local_addr().unwrap();

            info!("Test listener started on {}", actual_listener_addr);

            let server_handle = tokio::spawn(async move {
                match listener.accept().await {
                    Ok((mut socket, addr)) => {
                        info!("Server: Accepted connection from {}", addr);
                        let mut buf = BytesMut::with_capacity(1024);
                        match read_message(&mut socket, &mut buf).await {
                            Ok(Some(data)) => {
                                info!("Server: Received: {:?}", String::from_utf8_lossy(&data));
                                assert_eq!(data, Bytes::from_static(b"hello server"));
                                write_message(&mut socket, b"hello client").await.expect("Server: Failed to write");
                            }
                            Ok(None) => error!("Server: No data received or connection closed early"),
                            Err(e) => error!("Server: Error reading: {}", e),
                        }
                    }
                    Err(e) => {
                        error!("Server: Failed to accept connection: {}", e);
                    }
                }
            });

            // Give the server a moment to start (not ideal, but simple for a test)
            tokio::time::sleep(Duration::from_millis(100)).await;

            let client_handle = tokio::spawn(async move {
                match TcpConnectionManager::connect(actual_listener_addr).await {
                    Ok(mut stream) => {
                        info!("Client: Connected to {}", actual_listener_addr);
                        write_message(&mut stream, b"hello server").await.expect("Client: Failed to write");
                        let mut buf = BytesMut::with_capacity(1024);
                        match read_message(&mut stream, &mut buf).await {
                            Ok(Some(data)) => {
                                info!("Client: Received: {:?}", String::from_utf8_lossy(&data));
                                assert_eq!(data, Bytes::from_static(b"hello client"));
                            }
                            Ok(None) => error!("Client: No data received or connection closed early"),
                            Err(e) => error!("Client: Error reading: {}", e),
                        }
                    }
                    Err(e) => {
                        error!("Client: Failed to connect: {}", e);
                        panic!("Client connection failed");
                    }
                }
            });

            // Wait for both client and server to complete
            let _ = tokio::try_join!(server_handle, client_handle).expect("Test tasks failed");
            info!("Test completed successfully");
        });
    }

     #[test]
    fn test_connect_failure() {
        setup_logger();
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            // Use an address that is unlikely to be listening
            let non_existent_addr = "127.0.0.1:1"; // Port 1 is usually privileged or unused
            match TcpConnectionManager::connect(non_existent_addr).await {
                Ok(_) => panic!("Connection to non-existent server should fail"),
                Err(e) => {
                    info!("Successfully failed to connect to {}: {}", non_existent_addr, e);
                    match e {
                        NetworkError::Io(io_err) => {
                            assert_eq!(io_err.kind(), std::io::ErrorKind::ConnectionRefused);
                        }
                        _ => panic!("Expected IoError::ConnectionRefused, got {:?}", e),
                    }
                }
            }
        });
    }
}

