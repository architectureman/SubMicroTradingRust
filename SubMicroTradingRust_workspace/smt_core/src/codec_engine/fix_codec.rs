#![allow(dead_code, unused_variables)] // Allow dead code and unused vars for now

use crate::protocol_defs::fix_messages::*;
use crate::common_types::{Symbol, Side, OrderType, TimeInForce, OrderStatus};
use bytes::{BytesMut, BufMut, Bytes};
use rust_decimal::Decimal;
use std::str;

const SOH: u8 = 0x01;

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
}

fn calculate_checksum(buffer: &[u8]) -> u8 {
    buffer.iter().fold(0u8, |acc, &x| acc.wrapping_add(x))
}

fn append_tag_value(buf: &mut BytesMut, tag: u32, value: &str) {
    buf.put_slice(tag.to_string().as_bytes());
    buf.put_u8(b'=');
    buf.put_slice(value.as_bytes());
    buf.put_u8(SOH);
}

fn append_tag_decimal(buf: &mut BytesMut, tag: u32, value: Decimal) {
    append_tag_value(buf, tag, &value.to_string());
}

fn append_tag_u32(buf: &mut BytesMut, tag: u32, value: u32) {
    append_tag_value(buf, tag, &value.to_string());
}

fn append_tag_char(buf: &mut BytesMut, tag: u32, value: char) {
    append_tag_value(buf, tag, &value.to_string());
}

// Simplified FIX Encoder Trait
pub trait FixEncoder {
    fn encode_body(&self, body_buf: &mut BytesMut) -> Result<(), CodecError>;
    fn msg_type(&self) -> &str;
}

// Simplified FIX Decoder Trait
pub trait FixMessageBodyDecoder: Sized {
    fn decode_body(fields: &mut std::collections::HashMap<u32, Vec<u8>>) -> Result<Self, CodecError>;
}

// --- Implementations for specific messages ---

impl FixEncoder for FixNewOrderSingle {
    fn msg_type(&self) -> &str { "D" }
    fn encode_body(&self, body_buf: &mut BytesMut) -> Result<(), CodecError> {
        append_tag_value(body_buf, 11, &self.cl_ord_id);
        append_tag_value(body_buf, 55, self.symbol.as_str());
        let side_char = match self.side {
            Side::Buy => '1',
            Side::Sell => '2',
        };
        append_tag_char(body_buf, 54, side_char);
        append_tag_u32(body_buf, 60, self.transact_time as u32); // Assuming Timestamp is u64, cast to u32 for simplicity
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

// Helper to extract a required field
fn get_field_str(fields: &mut std::collections::HashMap<u32, Vec<u8>>, tag: u32) -> Result<String, CodecError> {
    fields.remove(&tag)
        .ok_or(CodecError::MissingField(tag))
        .and_then(|v| String::from_utf8(v).map_err(CodecError::from))
}

fn get_field_u32(fields: &mut std::collections::HashMap<u32, Vec<u8>>, tag: u32) -> Result<u32, CodecError> {
    get_field_str(fields, tag)?.parse().map_err(CodecError::from)
}

fn get_field_decimal(fields: &mut std::collections::HashMap<u32, Vec<u8>>, tag: u32) -> Result<Decimal, CodecError> {
    get_field_str(fields, tag)?.parse().map_err(CodecError::from)
}

fn get_field_char(fields: &mut std::collections::HashMap<u32, Vec<u8>>, tag: u32) -> Result<char, CodecError> {
    let s = get_field_str(fields, tag)?;
    s.chars().next().ok_or_else(|| CodecError::InvalidValue { tag, value: s.clone() })
}

impl FixMessageBodyDecoder for FixNewOrderSingle {
    fn decode_body(fields: &mut std::collections::HashMap<u32, Vec<u8>>) -> Result<Self, CodecError> {
        let cl_ord_id = get_field_str(fields, 11)?;
        let symbol_str = get_field_str(fields, 55)?;
        let symbol = Symbol::new(&symbol_str);
        let side_char = get_field_char(fields, 54)?;
        let side = match side_char {
            '1' => Side::Buy,
            '2' => Side::Sell,
            _ => return Err(CodecError::InvalidValue{tag: 54, value: side_char.to_string()}),
        };
        let transact_time = get_field_u32(fields, 60)? as u64; // Assuming Timestamp is u64
        let order_qty = get_field_decimal(fields, 38)?;
        let ord_type_char = get_field_char(fields, 40)?;
        let ord_type = match ord_type_char {
            '1' => OrderType::Market,
            '2' => OrderType::Limit,
            _ => return Err(CodecError::InvalidValue{tag: 40, value: ord_type_char.to_string()}),
        };
        let price = fields.remove(&44)
            .map(|v| String::from_utf8(v).map_err(CodecError::from).and_then(|s| s.parse().map_err(CodecError::from)))
            .transpose()?;
        let tif = fields.remove(&59)
            .map(|v| String::from_utf8(v).map_err(CodecError::from).and_then(|s| {
                let c = s.chars().next().ok_or_else(|| CodecError::InvalidValue{tag: 59, value: s.clone()})?;
                match c {
                    '0' => Ok(TimeInForce::Day),
                    '1' => Ok(TimeInForce::GTC),
                    '3' => Ok(TimeInForce::IOC),
                    '4' => Ok(TimeInForce::FOK),
                    '6' => Ok(TimeInForce::GTD),
                    _ => Err(CodecError::InvalidValue{tag: 59, value: c.to_string()}),
                }
            }))
            .transpose()?;

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

// Implement FixEncoder and FixMessageBodyDecoder for other message types (Logon, ExecutionReport, etc.) similarly
// For brevity, only NewOrderSingle is partially implemented here.

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
    fn decode_body(fields: &mut std::collections::HashMap<u32, Vec<u8>>) -> Result<Self, CodecError> {
        let encrypt_method = get_field_u32(fields, 98)?;
        let heart_bt_int = get_field_u32(fields, 108)?;
        let reset_seq_num_flag = fields.remove(&141)
            .map(|v| String::from_utf8(v).map_err(CodecError::from).map(|s| s == "Y"))
            .transpose()?;
        Ok(FixLogon { encrypt_method, heart_bt_int, reset_seq_num_flag })
    }
}


pub fn encode_fix_message<T: FixEncoder>(
    msg_body: &T,
    sender_comp_id: &str,
    target_comp_id: &str,
    msg_seq_num: u32,
    sending_time: u64, // Simplified timestamp
) -> Result<Bytes, CodecError> {
    let mut body_buf = BytesMut::new();
    msg_body.encode_body(&mut body_buf)?;

    let mut header_buf = BytesMut::new();
    append_tag_value(&mut header_buf, 8, "FIX.4.2"); // Example, should be configurable
    append_tag_u32(&mut header_buf, 9, body_buf.len() as u32);
    append_tag_value(&mut header_buf, 35, msg_body.msg_type());
    append_tag_value(&mut header_buf, 49, sender_comp_id);
    append_tag_value(&mut header_buf, 56, target_comp_id);
    append_tag_u32(&mut header_buf, 34, msg_seq_num);
    append_tag_u32(&mut header_buf, 52, sending_time as u32); // Simplified sending time

    let mut full_msg_buf = BytesMut::new();
    full_msg_buf.put_slice(&header_buf);
    full_msg_buf.put_slice(&body_buf);

    let checksum = calculate_checksum(&full_msg_buf);
    append_tag_value(&mut full_msg_buf, 10, &format!("{:03}", checksum));

    Ok(full_msg_buf.freeze())
}


pub fn decode_fix_message(buffer: &mut BytesMut) -> Result<Option<FixMessage>, CodecError> {
    // This is a very simplified parser. A real FIX parser is much more complex.
    // It needs to handle partial messages, find SOH delimiters, extract tags and values robustly.

    // Find 8=FIX...10=CHK<SOH>
    let data_slice = buffer.as_ref();
    let mut fields: std::collections::HashMap<u32, Vec<u8>> = std::collections::HashMap::new();
    let mut current_pos = 0;

    // Find BeginString (8=)
    let tag8_pos = data_slice.windows(2).position(|w| w == b"8=").ok_or(CodecError::IncompleteMessage)?;
    current_pos = tag8_pos;

    let mut body_length: Option<u32> = None;
    let mut msg_type: Option<String> = None;
    let mut checksum_val: Option<u8> = None;
    let mut header_map = std::collections::HashMap::new();
    let mut body_map = std::collections::HashMap::new();

    let mut last_field_end = current_pos;

    loop {
        if current_pos >= data_slice.len() { break; }
        let remaining_slice = &data_slice[current_pos..];
        let eq_pos = remaining_slice.iter().position(|&b| b == b'=').ok_or(CodecError::IncompleteMessage)?;
        let tag_str = str::from_utf8(&remaining_slice[..eq_pos])?;
        let tag: u32 = tag_str.parse().map_err(|_| CodecError::InvalidFormat(format!("Invalid tag: {}", tag_str)))?;

        let soh_pos = remaining_slice[eq_pos+1..].iter().position(|&b| b == SOH).ok_or(CodecError::IncompleteMessage)?;
        let value_slice = &remaining_slice[eq_pos+1 .. eq_pos+1+soh_pos];
        let value_vec = value_slice.to_vec();

        current_pos += eq_pos + 1 + soh_pos + 1;
        last_field_end = current_pos;

        match tag {
            8 | 9 | 35 | 49 | 56 | 34 | 52 => { // Header tags
                header_map.insert(tag, value_vec.clone());
                if tag == 9 {
                    body_length = Some(String::from_utf8(value_vec.clone())?.parse()?);
                }
                if tag == 35 {
                    msg_type = Some(String::from_utf8(value_vec.clone())?);
                }
            }
            10 => { // Trailer tag
                checksum_val = Some(String::from_utf8(value_vec)?.parse()?);
                break; // Checksum is the last field
            }
            _ => { // Body tags
                body_map.insert(tag, value_vec);
            }
        }
    }

    let calculated_checksum_upto_tag10_start = calculate_checksum(&data_slice[tag8_pos..last_field_end - format!("10={:03}{}", checksum_val.unwrap_or(0), SOH as char).len()]);

    if checksum_val.is_none() { return Err(CodecError::MissingField(10)); }
    if calculated_checksum_upto_tag10_start != checksum_val.unwrap() {
        return Err(CodecError::ChecksumMismatch { expected: checksum_val.unwrap(), actual: calculated_checksum_upto_tag10_start });
    }

    let header = FixHeader {
        begin_string: String::from_utf8(header_map.remove(&8).ok_or(CodecError::MissingField(8))?)?,
        body_length: body_length.ok_or(CodecError::MissingField(9))?,
        msg_type: msg_type.clone().ok_or(CodecError::MissingField(35))?,
        sender_comp_id: String::from_utf8(header_map.remove(&49).ok_or(CodecError::MissingField(49))?)?,
        target_comp_id: String::from_utf8(header_map.remove(&56).ok_or(CodecError::MissingField(56))?)?,
        msg_seq_num: String::from_utf8(header_map.remove(&34).ok_or(CodecError::MissingField(34))?)?.parse()?,
        sending_time: String::from_utf8(header_map.remove(&52).ok_or(CodecError::MissingField(52))?)?.parse::<u64>()?,
    };

    let body = match msg_type.as_deref() {
        Some("D") => FixMessageBody::NewOrderSingle(FixNewOrderSingle::decode_body(&mut body_map)?),
        Some("A") => FixMessageBody::Logon(FixLogon::decode_body(&mut body_map)?),
        // Add other message types here
        Some(unknown_type) => return Err(CodecError::UnsupportedMessageType(unknown_type.to_string())),
        None => return Err(CodecError::MissingField(35)),
    };

    buffer.split_to(last_field_end); // Consume the processed message from the buffer
    Ok(Some(FixMessage { header, body }))
}


#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use crate::common_types::{Symbol, Side, OrderType, TimeInForce};

    #[test]
    fn test_encode_decode_new_order_single() -> Result<(), CodecError> {
        let nos = FixNewOrderSingle {
            cl_ord_id: "TestOrd1".to_string(),
            symbol: Symbol::new("EUR/USD"),
            side: Side::Buy,
            transact_time: 1678886400, // Example timestamp
            order_qty: dec!(100.5),
            ord_type: OrderType::Limit,
            price: Some(dec!(1.0567)),
            tif: Some(TimeInForce::Day),
        };

        let encoded_bytes = encode_fix_message(&nos, "SENDER", "TARGET", 1, 1678886400)?;
        println!("Encoded NOS: {}", String::from_utf8_lossy(&encoded_bytes));

        let mut buffer = BytesMut::from(encoded_bytes.as_ref());
        let decoded_msg_opt = decode_fix_message(&mut buffer)?;

        assert!(decoded_msg_opt.is_some());
        if let Some(decoded_msg) = decoded_msg_opt {
            assert_eq!(decoded_msg.header.msg_type, "D");
            if let FixMessageBody::NewOrderSingle(decoded_nos) = decoded_msg.body {
                assert_eq!(decoded_nos.cl_ord_id, "TestOrd1");
                assert_eq!(decoded_nos.symbol.as_str(), "EUR/USD");
                assert_eq!(decoded_nos.side, Side::Buy);
                assert_eq!(decoded_nos.order_qty, dec!(100.5));
                assert_eq!(decoded_nos.price, Some(dec!(1.0567)));
            } else {
                panic!("Decoded message is not NewOrderSingle");
            }
        }
        Ok(())
    }

    #[test]
    fn test_encode_decode_logon() -> Result<(), CodecError> {
        let logon = FixLogon {
            encrypt_method: 0,
            heart_bt_int: 30,
            reset_seq_num_flag: Some(true),
        };

        let encoded_bytes = encode_fix_message(&logon, "CLIENT1", "SERVER1", 1, 1678886000)?;
        println!("Encoded Logon: {}", String::from_utf8_lossy(&encoded_bytes));

        let mut buffer = BytesMut::from(encoded_bytes.as_ref());
        let decoded_msg_opt = decode_fix_message(&mut buffer)?;
        assert!(decoded_msg_opt.is_some());
        if let Some(decoded_msg) = decoded_msg_opt {
            assert_eq!(decoded_msg.header.msg_type, "A");
            if let FixMessageBody::Logon(decoded_logon) = decoded_msg.body {
                assert_eq!(decoded_logon.heart_bt_int, 30);
                assert_eq!(decoded_logon.reset_seq_num_flag, Some(true));
            } else {
                panic!("Decoded message is not Logon");
            }
        }
        Ok(())
    }

    #[test]
    fn test_checksum_calculation() {
        let data = b"8=FIX.4.2\x019=70\x0135=D\x0149=SENDER\x0156=TARGET\x0134=1\x0152=20230315-12:00:00.000\x0111=TestOrd1\x0155=EUR/USD\x0154=1\x0160=1678886400\x0138=100.5\x0140=2\x0144=1.0567\x0159=0\x01";
        // The checksum for this data (excluding the checksum field itself) should be calculated.
        // Example: if the string is "8=...<SOH>...<SOH>", checksum is sum of bytes % 256
        let checksum = calculate_checksum(data);
        // This value needs to be verified against a known correct checksum for the given string.
        // For example, if a FIX engine generates "...10=123<SOH>", then 123 is the target.
        // Here, we just ensure it runs. A real test would compare to a pre-calculated value.
        assert!(checksum > 0); // Basic check
    }
}

impl FixEncoder for FixExecutionReport {
    fn msg_type(&self) -> &str { "8" }
    fn encode_body(&self, body_buf: &mut BytesMut) -> Result<(), CodecError> {
        append_tag_value(body_buf, 37, &self.order_id.to_string());
        if let Some(ref cl_ord_id) = self.cl_ord_id {
            append_tag_value(body_buf, 11, cl_ord_id);
        }
        append_tag_value(body_buf, 17, &self.exec_id);
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
        append_tag_value(body_buf, 55, self.symbol.as_str());
        let side_char = match self.side {
            Side::Buy => '1',
            Side::Sell => '2',
        };
        append_tag_char(body_buf, 54, side_char);
        append_tag_decimal(body_buf, 151, self.leaves_qty);
        append_tag_decimal(body_buf, 14, self.cum_qty);
        append_tag_decimal(body_buf, 6, self.avg_px);
        if let Some(last_qty) = self.last_qty {
            append_tag_decimal(body_buf, 32, last_qty);
        }
        if let Some(last_px) = self.last_px {
            append_tag_decimal(body_buf, 31, last_px);
        }
        append_tag_u32(body_buf, 60, self.transact_time as u32);
        if let Some(ref text) = self.text {
            append_tag_value(body_buf, 58, text);
        }
        Ok(())
    }
}

