#![allow(dead_code)] // Allow dead code for now

use crate::common_types::{Price, Quantity, Symbol, OrderID, Timestamp, Side, OrderType, TimeInForce};
use rust_decimal::Decimal;

// --- FIX Message Header --- (Simplified for now)
#[derive(Debug, Clone, PartialEq)]
pub struct FixHeader {
    pub begin_string: String, // Tag 8
    pub body_length: u32,    // Tag 9
    pub msg_type: String,    // Tag 35
    pub sender_comp_id: String, // Tag 49
    pub target_comp_id: String, // Tag 56
    pub msg_seq_num: u32,    // Tag 34
    pub sending_time: Timestamp, // Tag 52 (Simplified to Timestamp)
                                // Add other header fields as needed: PossDupFlag, OrigSendingTime, etc.
}

// --- FIX Message Trailer --- (Simplified)
#[derive(Debug, Clone, PartialEq)]
pub struct FixTrailer {
    pub checksum: String, // Tag 10
}

// --- Specific FIX Messages ---

// D - New Order Single
#[derive(Debug, Clone, PartialEq)]
pub struct FixNewOrderSingle {
    // Header and Trailer will be handled by a generic FixMessage wrapper or during encoding/decoding
    pub cl_ord_id: String,       // Tag 11
    pub symbol: Symbol,          // Tag 55
    pub side: Side,              // Tag 54 (e.g., '1' for Buy, '2' for Sell - map to enum)
    pub transact_time: Timestamp,// Tag 60
    pub order_qty: Quantity,     // Tag 38
    pub ord_type: OrderType,     // Tag 40 (e.g., '1' for Market, '2' for Limit)
    pub price: Option<Price>,    // Tag 44 (Present for Limit orders)
    pub tif: Option<TimeInForce>,// Tag 59 (TimeInForce)
                                 // Add other fields: HandlInst, Account, Currency, etc.
}

// 8 - Execution Report
#[derive(Debug, Clone, PartialEq)]
pub struct FixExecutionReport {
    pub order_id: OrderID,       // Tag 37 (Exchange Order ID)
    pub cl_ord_id: Option<String>,   // Tag 11 (Client Order ID, optional if system generated OrderID is primary)
    pub exec_id: String,         // Tag 17 (Unique identifier for this execution report)
    pub ord_status: crate::common_types::OrderStatus, // Tag 39 (e.g., '0' New, '1' PartiallyFilled, '2' Filled)
    pub symbol: Symbol,          // Tag 55
    pub side: Side,              // Tag 54
    pub leaves_qty: Quantity,    // Tag 151 (Remaining quantity)
    pub cum_qty: Quantity,       // Tag 14 (Total quantity filled for this order)
    pub avg_px: Price,           // Tag 6 (Average price of all fills on this order)
    pub last_qty: Option<Quantity>,// Tag 32 (Quantity of this specific fill/execution)
    pub last_px: Option<Price>,  // Tag 31 (Price of this specific fill/execution)
    pub transact_time: Timestamp,// Tag 60
    pub text: Option<String>,    // Tag 58 (Optional text message)
                                 // Add other fields: ExecType, Account, etc.
}

// A - Logon
#[derive(Debug, Clone, PartialEq)]
pub struct FixLogon {
    pub encrypt_method: u32, // Tag 98 (0 = None/Other, 7 = DES, etc.)
    pub heart_bt_int: u32,   // Tag 108 (Heartbeat interval in seconds)
    pub reset_seq_num_flag: Option<bool>, // Tag 141
                                      // Add other fields: Username, Password, DefaultApplVerID, etc.
}

// 5 - Logout
#[derive(Debug, Clone, PartialEq)]
pub struct FixLogout {
    pub text: Option<String>, // Tag 58 (Optional reason for logout)
}

// 0 - Heartbeat
#[derive(Debug, Clone, PartialEq)]
pub struct FixHeartbeat {
    pub test_req_id: Option<String>, // Tag 112 (Required if this heartbeat is in response to a TestRequest)
}

// 1 - Test Request
#[derive(Debug, Clone, PartialEq)]
pub struct FixTestRequest {
    pub test_req_id: String, // Tag 112
}

// 2 - Resend Request
#[derive(Debug, Clone, PartialEq)]
pub struct FixResendRequest {
    pub begin_seq_no: u32, // Tag 7
    pub end_seq_no: u32,   // Tag 16 (0 for all messages after BeginSeqNo)
}

// 4 - Sequence Reset
#[derive(Debug, Clone, PartialEq)]
pub struct FixSequenceReset {
    pub new_seq_no: u32,       // Tag 36
    pub gap_fill_flag: Option<bool>, // Tag 123 (true if this message is to fill a sequence gap)
}


// Generic enum to represent any FIX message type for easier handling in some cases
// This will be populated by the FIX messages defined above.
// Meta-programming (macros) would be very helpful here to auto-generate this enum
// and the From/Into implementations.
#[derive(Debug, Clone, PartialEq)]
pub enum FixMessageBody {
    NewOrderSingle(FixNewOrderSingle),
    ExecutionReport(FixExecutionReport),
    Logon(FixLogon),
    Logout(FixLogout),
    Heartbeat(FixHeartbeat),
    TestRequest(FixTestRequest),
    ResendRequest(FixResendRequest),
    SequenceReset(FixSequenceReset),
    // ... other message types
}

// A complete FIX message including header, body, and trailer
#[derive(Debug, Clone, PartialEq)]
pub struct FixMessage {
    pub header: FixHeader,
    pub body: FixMessageBody,
    // Trailer is often calculated on the fly during serialization
    // pub trailer: FixTrailer, // Or handle checksum calculation separately
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::common_types::OrderStatus;
    use rust_decimal_macros::dec;

    #[test]
    fn create_fix_new_order_single() {
        let nos = FixNewOrderSingle {
            cl_ord_id: "test_ord_123".to_string(),
            symbol: Symbol::new("EUR/USD"),
            side: Side::Buy,
            transact_time: 0, // Placeholder
            order_qty: dec!(100.0),
            ord_type: OrderType::Limit,
            price: Some(dec!(1.12345)),
            tif: Some(TimeInForce::Day),
        };
        assert_eq!(nos.cl_ord_id, "test_ord_123");
        assert_eq!(nos.ord_type, OrderType::Limit);
    }

    #[test]
    fn create_fix_execution_report() {
        let er = FixExecutionReport {
            order_id: 1,
            cl_ord_id: Some("client_ord_001".to_string()),
            exec_id: "exec_id_987".to_string(),
            ord_status: OrderStatus::Filled,
            symbol: Symbol::new("AAPL"),
            side: Side::Sell,
            leaves_qty: dec!(0.0),
            cum_qty: dec!(50.0),
            avg_px: dec!(175.50),
            last_qty: Some(dec!(50.0)),
            last_px: Some(dec!(175.50)),
            transact_time: 123456789, // Placeholder
            text: Some("Order Filled".to_string()),
        };
        assert_eq!(er.ord_status, OrderStatus::Filled);
        assert_eq!(er.avg_px, dec!(175.50));
    }

    #[test]
    fn create_fix_logon() {
        let logon = FixLogon {
            encrypt_method: 0,
            heart_bt_int: 30,
            reset_seq_num_flag: Some(true),
        };
        assert_eq!(logon.heart_bt_int, 30);
    }
}

