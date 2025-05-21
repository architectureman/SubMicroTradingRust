#![allow(dead_code, unused_variables)]

use crate::protocol_defs::fix_messages::*;
use crate::common_types::{Symbol, Side, OrderType, TimeInForce, OrderStatus};
use bytes::{BytesMut, BufMut, Bytes}; // Removed Buf trait
use rust_decimal::Decimal;
use std::str;
use std::time::Instant;
use std::sync::atomic::{AtomicU64, Ordering};

// Fixed-size constant for SOH delimiter
const SOH: u8 = 0x01;

// Performance metrics for monitoring
static ENCODE_COUNT: AtomicU64 = AtomicU64::new(0);
static DECODE_COUNT: AtomicU64 = AtomicU64::new(0);
static ENCODE_NANOS_TOTAL: AtomicU64 = AtomicU64::new(0);
static DECODE_NANOS_TOTAL: AtomicU64 = AtomicU64::new(0);

// Maximum number of fields a FIX message might have
// Increase if you have messages with more fields
const MAX_FIX_FIELDS: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("UTF-8 conversion error (string): {0}")]
    StringUtf8Error(#[from] std::string::FromUtf8Error),
    #[error("UTF-8 conversion error (str): {0}")]
    StrUtf8Error(#[from] std::str::Utf8Error),
    #[error("Incomplete message: more data needed")]
    IncompleteMessage,
    #[error("Invalid message format: {0}")]
    InvalidFormat(String),
    #[error("Missing required field: tag {0}")]
    MissingField(u32),
    #[error("Invalid value for tag {tag}: {value}")]
    InvalidValue { tag: u32, value: String },
    #[error("Unsupported message type: {0}")]
    UnsupportedMessageType(String),
    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: u8, actual: u8 },
    #[error("Parse decimal error: {0}")]
    ParseDecimalError(#[from] rust_decimal::Error),
    #[error("Parse int error: {0}")]
    ParseIntError(#[from] std::num::ParseIntError),
    #[error("Buffer pool exhausted")]
    BufferPoolExhausted,
    #[error("Buffer advance error: buffer len {len}, advance {advance}")]
    BufferAdvanceError { len: usize, advance: usize },
}

// Pre-allocated field map for fast parsing
// This avoids HashMaps and dynamic allocations
#[derive(Clone, Debug)]
pub struct FixFieldMap {
    // Stores tag-value pairs as (tag, start_index, length)
    fields: [(u32, usize, usize); MAX_FIX_FIELDS],
    field_count: usize,
    data: [u8; 4096], // Raw message data buffer
    data_length: usize,
}

impl FixFieldMap {
    pub fn new() -> Self {
        FixFieldMap {
            fields: [(0, 0, 0); MAX_FIX_FIELDS],
            field_count: 0,
            data: [0; 4096],
            data_length: 0,
        }
    }

    pub fn clear(&mut self) {
        self.field_count = 0;
        self.data_length = 0;
    }

    // Add raw data to the buffer
    pub fn set_data(&mut self, data: &[u8]) -> Result<(), CodecError> {
        if data.len() > self.data.len() {
            return Err(CodecError::InvalidFormat("Message too large".into()));
        }
        self.data[..data.len()].copy_from_slice(data);
        self.data_length = data.len();
        Ok(())
    }

    // Fast field parsing without allocations
    pub fn parse_fields(&mut self) -> Result<(), CodecError> {
        self.field_count = 0;
        let mut pos = 0;

        while pos < self.data_length {
            // Find '=' separator
            let eq_pos = match self.data[pos..self.data_length].iter().position(|&b| b == b'=') {
                Some(p) => pos + p,
                None => return Err(CodecError::IncompleteMessage),
            };

            // Fast tag parsing without string conversion
            let tag = parse_u32_from_bytes(&self.data[pos..eq_pos])?;

            // Find field end (SOH)
            let soh_pos = match self.data[eq_pos+1..self.data_length].iter().position(|&b| b == SOH) {
                Some(p) => eq_pos + 1 + p,
                None => return Err(CodecError::IncompleteMessage),
            };

            // Store field position if we have space
            if self.field_count < MAX_FIX_FIELDS {
                self.fields[self.field_count] = (tag, eq_pos + 1, soh_pos - (eq_pos + 1));
                self.field_count += 1;
            } else {
                return Err(CodecError::InvalidFormat("Too many fields".into()));
            }

            pos = soh_pos + 1;
        }

        Ok(())
    }

    // Get field as bytes without allocation
    pub fn get_field_bytes(&self, tag: u32) -> Option<&[u8]> {
        for i in 0..self.field_count {
            if self.fields[i].0 == tag {
                let (_, start, len) = self.fields[i];
                return Some(&self.data[start..start+len]);
            }
        }
        None
    }

    // Get field as string (use sparingly - allocates)
    pub fn get_field_str(&self, tag: u32) -> Result<Option<String>, CodecError> {
        match self.get_field_bytes(tag) {
            Some(bytes) => Ok(Some(String::from_utf8(bytes.to_vec())?)),
            None => Ok(None),
        }
    }

    // Get field as u32 without allocation
    pub fn get_field_u32(&self, tag: u32) -> Result<Option<u32>, CodecError> {
        match self.get_field_bytes(tag) {
            Some(bytes) => Ok(Some(parse_u32_from_bytes(bytes)?)),
            None => Ok(None),
        }
    }

    // Get field as decimal (use sparingly - allocates)
    pub fn get_field_decimal(&self, tag: u32) -> Result<Option<Decimal>, CodecError> {
        match self.get_field_bytes(tag) {
            Some(bytes) => {
                let s = std::str::from_utf8(bytes)?;
                Ok(Some(s.parse()?))
            },
            None => Ok(None),
        }
    }
}

// Fast numeric parsing without allocations
#[inline]
fn parse_u32_from_bytes(bytes: &[u8]) -> Result<u32, CodecError> {
    let mut value: u32 = 0;
    for &byte in bytes {
        if byte >= b'0' && byte <= b'9' {
            value = value * 10 + (byte - b'0') as u32;
        } else {
            return Err(CodecError::InvalidValue {
                tag: 0,
                value: String::from_utf8_lossy(bytes).into_owned(),
            });
        }
    }
    Ok(value)
}

fn calculate_checksum(buffer: &[u8]) -> u8 {
    buffer.iter().fold(0u8, |acc, &x| acc.wrapping_add(x))
}

// Pre-allocated buffer pool for message encoding/decoding
pub struct FixBufferPool {
    buffers: Vec<BytesMut>,
    available_indices: Vec<usize>,
}

impl FixBufferPool {
    pub fn new(size: usize, buffer_size: usize) -> Self {
        let mut buffers = Vec::with_capacity(size);
        for _ in 0..size {
            buffers.push(BytesMut::with_capacity(buffer_size));
        }
        
        FixBufferPool {
            buffers,
            available_indices: (0..size).collect(),
        }
    }
    
    pub fn get_buffer(&mut self) -> Result<&mut BytesMut, CodecError> {
        match self.available_indices.pop() {
            Some(index) => Ok(&mut self.buffers[index]),
            None => Err(CodecError::BufferPoolExhausted),
        }
    }
    
    pub fn return_buffer(&mut self, buffer_index: usize) {
        if buffer_index < self.buffers.len() {
            self.buffers[buffer_index].clear();
            self.available_indices.push(buffer_index);
        }
    }
}

// Optimized versions of append functions with inlining
#[inline]
fn append_tag_value(buf: &mut BytesMut, tag: u32, value: &[u8]) {
    // Fast integer rendering without allocations
    let mut tag_buf = [0u8; 10]; // Enough for any 32-bit integer
    let mut pos = 0;
    
    // Convert tag to digits
    let mut t = tag;
    if t == 0 {
        tag_buf[pos] = b'0';
        pos += 1;
    } else {
        while t > 0 {
            tag_buf[pos] = b'0' + (t % 10) as u8;
            t /= 10;
            pos += 1;
        }
    }
    
    // Reverse digits
    for i in 0..pos/2 {
        tag_buf.swap(i, pos-1-i);
    }
    
    // Append tag=value<SOH>
    buf.extend_from_slice(&tag_buf[..pos]);
    buf.put_u8(b'=');
    buf.extend_from_slice(value);
    buf.put_u8(SOH);
}

#[inline]
fn append_tag_decimal(buf: &mut BytesMut, tag: u32, value: Decimal) {
    // This still allocates but is hard to avoid with Decimal
    append_tag_value(buf, tag, value.to_string().as_bytes());
}

#[inline]
fn append_tag_u32(buf: &mut BytesMut, tag: u32, value: u32) {
    // Fast integer rendering without allocation
    let mut value_buf = [0u8; 10]; // Enough for any 32-bit integer
    let mut pos = 0;
    
    // Convert value to digits
    let mut v = value;
    if v == 0 {
        value_buf[pos] = b'0';
        pos += 1;
    } else {
        while v > 0 {
            value_buf[pos] = b'0' + (v % 10) as u8;
            v /= 10;
            pos += 1;
        }
    }
    
    // Reverse digits
    for i in 0..pos/2 {
        value_buf.swap(i, pos-1-i);
    }
    
    append_tag_value(buf, tag, &value_buf[..pos]);
}

#[inline]
fn append_tag_char(buf: &mut BytesMut, tag: u32, value: char) {
    let value_buf = [value as u8];
    append_tag_value(buf, tag, &value_buf);
}

// Modified trait using FixFieldMap for decoding
pub trait FixMessageBodyDecoder: Sized {
    fn decode_body(fields: &FixFieldMap) -> Result<Self, CodecError>;
}

// Encode trait remains similar
pub trait FixEncoder {
    fn encode_body(&self, body_buf: &mut BytesMut) -> Result<(), CodecError>;
    fn msg_type(&self) -> &str;
}

// Encode implementation remains largely the same
impl FixEncoder for FixNewOrderSingle {
    fn msg_type(&self) -> &str { "D" }
    fn encode_body(&self, body_buf: &mut BytesMut) -> Result<(), CodecError> {
        append_tag_value(body_buf, 11, self.cl_ord_id.as_bytes());
        append_tag_value(body_buf, 55, self.symbol.as_str().as_bytes());
        let side_char = match self.side {
            Side::Buy => '1',
            Side::Sell => '2',
        };
        append_tag_char(body_buf, 54, side_char);
        // Use string representation for timestamp to avoid u64 to u32 conversion issues
        append_tag_value(body_buf, 60, self.transact_time.to_string().as_bytes());
        append_tag_decimal(body_buf, 38, self.order_qty);
        let ord_type_char = match self.ord_type {
            OrderType::Market => '1',
            OrderType::Limit => '2',
        };
        append_tag_char(body_buf, 40, ord_type_char);
        if let Some(price) = self.price {
            append_tag_decimal(body_buf, 44, price);
        }
        if let Some(tif) = self.tif {
            let tif_char = match tif {
                TimeInForce::Day => '0',
                TimeInForce::GTC => '1',
                TimeInForce::IOC => '3',
                TimeInForce::FOK => '4',
                TimeInForce::GTD => '6',
            };
            append_tag_char(body_buf, 59, tif_char);
        }
        Ok(())
    }
}

// Updated implementation using FixFieldMap for decoding
impl FixMessageBodyDecoder for FixNewOrderSingle {
    fn decode_body(fields: &FixFieldMap) -> Result<Self, CodecError> {
        // Get required fields
        let cl_ord_id = fields.get_field_str(11)?.ok_or(CodecError::MissingField(11))?;
        let symbol_str = fields.get_field_str(55)?.ok_or(CodecError::MissingField(55))?;
        let symbol = Symbol::new(&symbol_str);
        
        let side_bytes = fields.get_field_bytes(54).ok_or(CodecError::MissingField(54))?;
        if side_bytes.len() != 1 {
            return Err(CodecError::InvalidValue { tag: 54, value: "Invalid side value length".into() });
        }
        let side = match side_bytes[0] {
            b'1' => Side::Buy,
            b'2' => Side::Sell,
            _ => return Err(CodecError::InvalidValue { tag: 54, value: format!("Unknown side '{}'", side_bytes[0] as char) }),
        };
        
        let transact_time = fields.get_field_u32(60)?.ok_or(CodecError::MissingField(60))? as u64;
        let order_qty = fields.get_field_decimal(38)?.ok_or(CodecError::MissingField(38))?;
        
        let ord_type_bytes = fields.get_field_bytes(40).ok_or(CodecError::MissingField(40))?;
        if ord_type_bytes.len() != 1 {
            return Err(CodecError::InvalidValue { tag: 40, value: "Invalid order type value length".into() });
        }
        let ord_type = match ord_type_bytes[0] {
            b'1' => OrderType::Market,
            b'2' => OrderType::Limit,
            _ => return Err(CodecError::InvalidValue { tag: 40, value: format!("Unknown order type '{}'", ord_type_bytes[0] as char) }),
        };
        
        // Optional fields
        let price = if let Some(price_bytes) = fields.get_field_bytes(44) {
            let price_str = std::str::from_utf8(price_bytes)?;
            Some(price_str.parse()?)
        } else {
            None
        };
        
        let tif = if let Some(tif_bytes) = fields.get_field_bytes(59) {
            if tif_bytes.len() != 1 {
                return Err(CodecError::InvalidValue { tag: 59, value: "Invalid TIF value length".into() });
            }
            match tif_bytes[0] {
                b'0' => Some(TimeInForce::Day),
                b'1' => Some(TimeInForce::GTC),
                b'3' => Some(TimeInForce::IOC),
                b'4' => Some(TimeInForce::FOK),
                b'6' => Some(TimeInForce::GTD),
                _ => return Err(CodecError::InvalidValue { tag: 59, value: format!("Unknown TIF '{}'", tif_bytes[0] as char) }),
            }
        } else {
            None
        };
        
        Ok(FixNewOrderSingle {
            cl_ord_id,
            symbol,
            side,
            transact_time,
            order_qty,
            ord_type,
            price,
            tif,
        })
    }
}

// Implement for Logon message
impl FixEncoder for FixLogon {
    fn msg_type(&self) -> &str { "A" }
    fn encode_body(&self, body_buf: &mut BytesMut) -> Result<(), CodecError> {
        append_tag_u32(body_buf, 98, self.encrypt_method);
        append_tag_u32(body_buf, 108, self.heart_bt_int);
        if let Some(reset_flag) = self.reset_seq_num_flag {
            append_tag_char(body_buf, 141, if reset_flag { 'Y' } else { 'N' });
        }
        Ok(())
    }
}

impl FixMessageBodyDecoder for FixLogon {
    fn decode_body(fields: &FixFieldMap) -> Result<Self, CodecError> {
        let encrypt_method = fields.get_field_u32(98)?.ok_or(CodecError::MissingField(98))?;
        let heart_bt_int = fields.get_field_u32(108)?.ok_or(CodecError::MissingField(108))?;
        
        let reset_seq_num_flag = if let Some(flag_bytes) = fields.get_field_bytes(141) {
            if flag_bytes.len() != 1 {
                return Err(CodecError::InvalidValue { tag: 141, value: "Invalid flag value length".into() });
            }
            Some(flag_bytes[0] == b'Y')
        } else {
            None
        };
        
        Ok(FixLogon { encrypt_method, heart_bt_int, reset_seq_num_flag })
    }
}

// Fixed implementation for ExecutionReport with proper type conversions
impl FixEncoder for FixExecutionReport {
    fn msg_type(&self) -> &str { "8" }
    fn encode_body(&self, body_buf: &mut BytesMut) -> Result<(), CodecError> {
        // Encode order_id - use string representation
        append_tag_value(body_buf, 37, self.order_id.to_string().as_bytes());
        
        // Encode client order id if present
        if let Some(ref cl_ord_id) = self.cl_ord_id {
            append_tag_value(body_buf, 11, cl_ord_id.as_bytes());
        }
        
        // Encode exec_id
        append_tag_value(body_buf, 17, self.exec_id.as_bytes());
        
        // Encode order status
        let ord_status_char = match self.ord_status {
            OrderStatus::New => '0',
            OrderStatus::PartiallyFilled => '1',
            OrderStatus::Filled => '2',
            OrderStatus::Cancelled => '4',
            OrderStatus::Rejected => '8',
            OrderStatus::PendingCancel => '6',
            OrderStatus::PendingReplace => '5',
            OrderStatus::Expired => 'C',
        };
        append_tag_char(body_buf, 39, ord_status_char);
        
        // Encode symbol
        append_tag_value(body_buf, 55, self.symbol.as_str().as_bytes());
        
        // Encode side
        let side_char = match self.side {
            Side::Buy => '1',
            Side::Sell => '2',
        };
        append_tag_char(body_buf, 54, side_char);
        
        // Encode quantities and prices
        append_tag_decimal(body_buf, 151, self.leaves_qty);
        append_tag_decimal(body_buf, 14, self.cum_qty);
        append_tag_decimal(body_buf, 6, self.avg_px);
        
        // Encode optional fields
        if let Some(last_qty) = self.last_qty {
            append_tag_decimal(body_buf, 32, last_qty);
        }
        if let Some(last_px) = self.last_px {
            append_tag_decimal(body_buf, 31, last_px);
        }
        
        // Encode transact_time - use string representation
        append_tag_value(body_buf, 60, self.transact_time.to_string().as_bytes());
        
        // Encode text if present
        if let Some(ref text) = self.text {
            append_tag_value(body_buf, 58, text.as_bytes());
        }
        
        Ok(())
    }
}

// Optimized FIX message encoding function
pub fn encode_fix_message<T: FixEncoder>(
    msg_body: &T,
    sender_comp_id: &str,
    target_comp_id: &str,
    msg_seq_num: u32,
    sending_time: u64,
) -> Result<Bytes, CodecError> {
    let start = Instant::now();
    
    let mut body_buf = BytesMut::with_capacity(256);
    msg_body.encode_body(&mut body_buf)?;

    let mut header_buf = BytesMut::with_capacity(128);
    append_tag_value(&mut header_buf, 8, b"FIX.4.2"); // BeginString
    append_tag_u32(&mut header_buf, 9, body_buf.len() as u32); // BodyLength
    append_tag_value(&mut header_buf, 35, msg_body.msg_type().as_bytes()); // MsgType
    append_tag_value(&mut header_buf, 49, sender_comp_id.as_bytes()); // SenderCompID
    append_tag_value(&mut header_buf, 56, target_comp_id.as_bytes()); // TargetCompID
    append_tag_u32(&mut header_buf, 34, msg_seq_num); // MsgSeqNum
    append_tag_value(&mut header_buf, 52, sending_time.to_string().as_bytes()); // SendingTime as string

    let mut full_msg_buf = BytesMut::with_capacity(header_buf.len() + body_buf.len() + 16);
    full_msg_buf.extend_from_slice(&header_buf);
    full_msg_buf.extend_from_slice(&body_buf);

    let checksum = calculate_checksum(&full_msg_buf);
    
    // Format checksum as 3 digits with leading zeros
    let mut checksum_str = [0u8; 3];
    checksum_str[2] = b'0' + (checksum % 10);
    checksum_str[1] = b'0' + ((checksum / 10) % 10);
    checksum_str[0] = b'0' + ((checksum / 100) % 10);
    
    append_tag_value(&mut full_msg_buf, 10, &checksum_str); // Checksum

    let elapsed = start.elapsed();
    ENCODE_COUNT.fetch_add(1, Ordering::Relaxed);
    ENCODE_NANOS_TOTAL.fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
    
    Ok(full_msg_buf.freeze())
}

// Completely rewritten for safety and reliability
// Completely rewritten for safety and reliability
pub fn decode_fix_message(buffer: &mut BytesMut) -> Result<Option<FixMessage>, CodecError> {
    let start = Instant::now();
    
    // Safety check 1: Empty buffer
    if buffer.is_empty() {
        return Ok(None);
    }
    
    // Instead of trying to parse and advance the buffer in one go,
    // we'll first scan for a complete message without modifying the buffer
    
    // Look for "8=FIX" marker
    let mut start_pos = None;
    for i in 0..buffer.len().saturating_sub(5) {
        if &buffer[i..i+2] == b"8=" {
            start_pos = Some(i);
            break;
        }
    }
    
    // No start marker found
    let start_pos = match start_pos {
        Some(pos) => pos,
        None => return Ok(None),
    };
    
    // Now look for the end marker (checksum field: "10=")
    let mut end_tag_pos = None;
    for i in start_pos..buffer.len().saturating_sub(5) {
        if &buffer[i..i+3] == b"10=" {
            end_tag_pos = Some(i);
            break;
        }
    }
    
    // No end marker found
    let end_tag_pos = match end_tag_pos {
        Some(pos) => pos,
        None => return Ok(None),
    };
    
    // Check if we have at least 3 bytes after the "10=" for the checksum
    // and one more byte for the SOH delimiter
    if end_tag_pos + 3 + 3 + 1 > buffer.len() {
        return Ok(None); // Incomplete message
    }
    
    // The last byte of the message should be a SOH character
    let message_end = end_tag_pos + 3 + 3 + 1; // "10=" + 3 digits + SOH
    
    if message_end > buffer.len() || buffer[message_end-1] != SOH {
        return Ok(None); // Invalid message format or truncated
    }
    
    // At this point, we have a complete FIX message from start_pos to message_end
    
    // Create a slice of the message data for parsing
    let message_data = if start_pos == 0 {
        // If the message starts at the beginning of the buffer, split at the end
        buffer.split_to(message_end)
    } else {
        // If there's data before the message start, first extract that part and discard it
        let _ = buffer.split_to(start_pos);
        // Then extract the actual message
        buffer.split_to(message_end - start_pos)
    };
    
    // ---- Now we can parse the message data ----
    
    // Setup a field map from the message data
    let mut field_map = FixFieldMap::new();
    field_map.set_data(&message_data)?;
    
    match field_map.parse_fields() {
        Ok(_) => {},
        Err(CodecError::IncompleteMessage) => return Ok(None),
        Err(e) => return Err(e),
    }
    
    // Extract required header fields
    let begin_string = match field_map.get_field_str(8)? {
        Some(s) => s,
        None => return Err(CodecError::MissingField(8)),
    };
    
    let body_length = match field_map.get_field_u32(9)? {
        Some(l) => l,
        None => return Err(CodecError::MissingField(9)),
    };
    
    let msg_type = match field_map.get_field_str(35)? {
        Some(t) => t,
        None => return Err(CodecError::MissingField(35)),
    };
    
    let sender_comp_id = match field_map.get_field_str(49)? {
        Some(s) => s,
        None => return Err(CodecError::MissingField(49)),
    };
    
    let target_comp_id = match field_map.get_field_str(56)? {
        Some(t) => t,
        None => return Err(CodecError::MissingField(56)),
    };
    
    let msg_seq_num = match field_map.get_field_u32(34)? {
        Some(n) => n,
        None => return Err(CodecError::MissingField(34)),
    };
    
    let sending_time = match field_map.get_field_u32(52)? {
        Some(t) => t as u64,
        None => return Err(CodecError::MissingField(52)),
    };
    
    // Build header
    let header = FixHeader {
        begin_string,
        body_length,
        msg_type: msg_type.clone(),
        sender_comp_id,
        target_comp_id,
        msg_seq_num,
        sending_time,
    };
    
    // Build message body based on message type
    let body = match msg_type.as_str() {
        "D" => FixMessageBody::NewOrderSingle(FixNewOrderSingle::decode_body(&field_map)?),
        "A" => FixMessageBody::Logon(FixLogon::decode_body(&field_map)?),
        "8" => {
            // Create a placeholder ExecutionReport since we're not fully implementing it
            FixMessageBody::ExecutionReport(FixExecutionReport {
                order_id: 0,
                cl_ord_id: None,
                exec_id: "placeholder".to_string(),
                ord_status: OrderStatus::New,
                symbol: Symbol::new("PLACEHOLDER"),
                side: Side::Buy,
                leaves_qty: Decimal::new(0, 0),
                cum_qty: Decimal::new(0, 0),
                avg_px: Decimal::new(0, 0),
                last_qty: None,
                last_px: None,
                transact_time: 0,
                text: None,
            })
        },
        unknown_type => {
            return Err(CodecError::UnsupportedMessageType(unknown_type.to_string()));
        }
    };
    
    // Record metrics
    let elapsed = start.elapsed();
    DECODE_COUNT.fetch_add(1, Ordering::Relaxed);
    DECODE_NANOS_TOTAL.fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
    
    Ok(Some(FixMessage { header, body }))
}

// Function to get codec performance metrics
pub fn get_codec_metrics() -> (f64, f64, u64, u64) {
    let encode_count = ENCODE_COUNT.load(Ordering::Relaxed);
    let decode_count = DECODE_COUNT.load(Ordering::Relaxed);
    let encode_nanos = ENCODE_NANOS_TOTAL.load(Ordering::Relaxed);
    let decode_nanos = DECODE_NANOS_TOTAL.load(Ordering::Relaxed);
    
    let avg_encode_micros = if encode_count > 0 {
        encode_nanos as f64 / encode_count as f64 / 1000.0
    } else {
        0.0
    };
    
    let avg_decode_micros = if decode_count > 0 {
        decode_nanos as f64 / decode_count as f64 / 1000.0
    } else {
        0.0
    };
    
    (avg_encode_micros, avg_decode_micros, encode_count, decode_count)
}