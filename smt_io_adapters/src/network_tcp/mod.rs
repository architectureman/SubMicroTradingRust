use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use bytes::{BytesMut, Bytes};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, Duration};
use std::net::SocketAddr;
use tracing::{error, info, debug};

// Performance metrics
static READ_COUNT: AtomicU64 = AtomicU64::new(0);
static WRITE_COUNT: AtomicU64 = AtomicU64::new(0);
static READ_NANOS_TOTAL: AtomicU64 = AtomicU64::new(0);
static WRITE_NANOS_TOTAL: AtomicU64 = AtomicU64::new(0);

// Buffer pool size constants
const DEFAULT_BUFFER_SIZE: usize = 8192; // 8KB default buffer size
const DEFAULT_POOL_SIZE: usize = 256;    // 256 buffers in pool

/// Represents an error that can occur during network operations.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Connection error: {0}")]
    ConnectionError(String),
    #[error("Address parsing error: {0}")]
    AddrParseError(String),
    #[error("Buffer pool exhausted")]
    BufferPoolExhausted,
}

/// Pre-allocated buffer pool for network operations
pub struct NetworkBufferPool {
    buffers: Vec<BytesMut>,
    available_indices: Vec<usize>,
}

impl NetworkBufferPool {
    pub fn new(size: usize, buffer_size: usize) -> Self {
        let mut buffers = Vec::with_capacity(size);
        for _ in 0..size {
            buffers.push(BytesMut::with_capacity(buffer_size));
        }
        
        NetworkBufferPool {
            buffers,
            available_indices: (0..size).collect(),
        }
    }
    
    pub fn get_buffer(&mut self) -> Result<(usize, &mut BytesMut), NetworkError> {
        match self.available_indices.pop() {
            Some(index) => {
                self.buffers[index].clear();
                Ok((index, &mut self.buffers[index]))
            }
            None => Err(NetworkError::BufferPoolExhausted),
        }
    }
    
    pub fn return_buffer(&mut self, buffer_index: usize) {
        if buffer_index < self.buffers.len() {
            self.available_indices.push(buffer_index);
        }
    }
    
    pub fn get_buffer_by_index(&mut self, index: usize) -> Option<&mut BytesMut> {
        self.buffers.get_mut(index)
    }
    
    pub fn default() -> Self {
        Self::new(DEFAULT_POOL_SIZE, DEFAULT_BUFFER_SIZE)
    }
}

/// Optimized TCP connection manager with HFT-focused configuration
pub struct TcpConnectionManager {
    buffer_pool: NetworkBufferPool,
}

impl TcpConnectionManager {
    pub fn new() -> Self {
        TcpConnectionManager {
            buffer_pool: NetworkBufferPool::default(),
        }
    }
    
    /// Establishes a TCP connection optimized for low latency
    pub async fn connect<A: ToSocketAddrs + std::fmt::Debug>(addr: A) -> Result<TcpStream, NetworkError> {
        info!("Connecting to address: {:?}", addr);
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                info!("Connected to {:?}", stream.peer_addr()?);
                
                // Optimize socket for HFT
                configure_socket_for_hft(&stream)?;
                
                Ok(stream)
            }
            Err(e) => {
                error!("Connection failed: {}", e);
                Err(NetworkError::Io(e))
            }
        }
    }

    /// Listens for incoming TCP connections optimized for low latency
    pub async fn listen<A: ToSocketAddrs>(addr: A) -> Result<TcpListener, NetworkError> {
        let listener = TcpListener::bind(addr).await?;
        info!("Listening on {:?}", listener.local_addr()?);
        Ok(listener)
    }
    
    /// Get a buffer from the pool
    pub fn get_buffer(&mut self) -> Result<(usize, &mut BytesMut), NetworkError> {
        self.buffer_pool.get_buffer()
    }
    
    /// Return a buffer to the pool
    pub fn return_buffer(&mut self, buffer_index: usize) {
        self.buffer_pool.return_buffer(buffer_index);
    }
}

/// Configure a TCP socket for HFT performance
fn configure_socket_for_hft(socket: &TcpStream) -> io::Result<()> {
    // Disable Nagle's algorithm for lower latency
    socket.set_nodelay(true)?;
    
    // Set socket buffer sizes (large enough to handle bursts)
    socket.set_send_buffer_size(4 * 1024 * 1024)?; // 4MB
    socket.set_recv_buffer_size(4 * 1024 * 1024)?; // 4MB
    
    // Platform-specific optimizations
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        
        // Get the raw file descriptor
        let fd = socket.as_raw_fd();
        
        // Set socket priority (requires root on Linux)
        unsafe {
            let priority = 6i32; // High priority (range is 0-7)
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PRIORITY,
                &priority as *const i32 as *const libc::c_void,
                std::mem::size_of::<i32>() as libc::socklen_t,
            );
            
            // Enable TCP quick ACK for faster acknowledgments
            let quickack = 1i32;
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_QUICKACK,
                &quickack as *const i32 as *const libc::c_void,
                std::mem::size_of::<i32>() as libc::socklen_t,
            );
            
            // Use low latency mode (if available)
            let low_latency = 1i32;
            // TCP_LOW_LATENCY is not defined in standard libc - you might need custom bindings
            // or use a constant value directly
            const TCP_LOW_LATENCY: i32 = 363; // Value might differ across kernel versions
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                TCP_LOW_LATENCY,
                &low_latency as *const i32 as *const libc::c_void,
                std::mem::size_of::<i32>() as libc::socklen_t,
            );
        }
    }
    
    Ok(())
}

/// Reads a message from a TCP stream with optimal performance
pub async fn read_message(stream: &mut TcpStream, buffer: &mut BytesMut) -> Result<Option<Bytes>, NetworkError> {
    let start = Instant::now();
    
    // Read from socket directly into the buffer
    match stream.read_buf(buffer).await {
        Ok(0) => {
            debug!("Connection closed by peer while reading");
            Ok(None) // Connection closed
        }
        Ok(n) => {
            // Record metrics
            let elapsed = start.elapsed();
            READ_COUNT.fetch_add(1, Ordering::Relaxed);
            READ_NANOS_TOTAL.fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
            
            debug!("Read {} bytes from socket in {:.2}µs", n, elapsed.as_micros());
            
            if n > 0 {
                // Return a copy of what we read
                // In a real HFT system, you'd use zero-copy techniques here
                let message = buffer.clone().freeze();
                Ok(Some(message))
            } else {
                Ok(None)
            }
        }
        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
            Ok(None) // No data available right now
        }
        Err(e) => {
            error!("Error reading from stream: {}", e);
            Err(NetworkError::Io(e))
        }
    }
}

/// Writes a message to a TCP stream with optimal performance
pub async fn write_message(stream: &mut TcpStream, message: &[u8]) -> Result<(), NetworkError> {
    let start = Instant::now();
    
    // Write message directly to socket
    match stream.write_all(message).await {
        Ok(_) => {
            // Only flush if necessary - flushing adds latency but ensures delivery
            // For HFT, you might want to consider the tradeoff
            stream.flush().await?;
            
            // Record metrics
            let elapsed = start.elapsed();
            WRITE_COUNT.fetch_add(1, Ordering::Relaxed);
            WRITE_NANOS_TOTAL.fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
            
            debug!("Wrote {} bytes to socket in {:.2}µs", message.len(), elapsed.as_micros());
            Ok(())
        }
        Err(e) => {
            error!("Error writing to stream: {}", e);
            Err(NetworkError::Io(e))
        }
    }
}

/// Get network performance metrics
pub fn get_network_metrics() -> (f64, f64, u64, u64) {
    let read_count = READ_COUNT.load(Ordering::Relaxed);
    let write_count = WRITE_COUNT.load(Ordering::Relaxed);
    let read_nanos = READ_NANOS_TOTAL.load(Ordering::Relaxed);
    let write_nanos = WRITE_NANOS_TOTAL.load(Ordering::Relaxed);
    
    let avg_read_micros = if read_count > 0 {
        read_nanos as f64 / read_count as f64 / 1000.0
    } else {
        0.0
    };
    
    let avg_write_micros = if write_count > 0 {
        write_nanos as f64 / write_count as f64 / 1000.0
    } else {
        0.0
    };
    
    (avg_read_micros, avg_write_micros, read_count, write_count)
}

/// High-performance connection pool for reusing connections
pub struct ConnectionPool {
    connections: Vec<Option<TcpStream>>,
    address: String,
}

impl ConnectionPool {
    pub fn new(capacity: usize, address: String) -> Self {
        let connections = vec![None; capacity];
        ConnectionPool { connections, address }
    }
    
    pub async fn get_connection(&mut self) -> Result<(usize, &mut TcpStream), NetworkError> {
        // Find an available connection or create a new one
        for (i, conn) in self.connections.iter_mut().enumerate() {
            if conn.is_none() {
                // Create a new connection
                let stream = TcpConnectionManager::connect(&self.address).await?;
                *conn = Some(stream);
                return Ok((i, conn.as_mut().unwrap()));
            }
        }
        
        // If we get here, all connections are in use
        Err(NetworkError::ConnectionError("Connection pool exhausted".into()))
    }
    
    pub fn return_connection(&mut self, index: usize) {
        if index < self.connections.len() {
            // Instead of destroying, we keep the connection for reuse
            // In a real implementation, you'd check if the connection is still valid
        }
    }
    
    pub fn get_connection_by_index(&mut self, index: usize) -> Option<&mut TcpStream> {
        if index < self.connections.len() {
            self.connections[index].as_mut()
        } else {
            None
        }
    }
}