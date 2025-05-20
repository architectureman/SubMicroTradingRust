// /home/ubuntu/SubMicroTradingRust/SubMicroTradingRust_workspace/tests/fix_flow_integration_test.rs

use smt_core::common_types::{OrderType, Side, Symbol, TimeInForce};
use smt_core::protocol_defs::fix_messages::{FixLogon, FixMessage, FixMessageBody, FixNewOrderSingle};
use smt_core::codec_engine::fix_codec::{encode_fix_message, decode_fix_message, CodecError};
use smt_io_adapters::file_logger;
use bytes::BytesMut;
use rust_decimal_macros::dec;

#[test]
fn test_full_fix_new_order_single_encode_decode_flow() -> Result<(), CodecError> {
    // Initialize logger (optional, but good for debugging if tests fail)
    let _ = file_logger::init_logger(); // Allow error if already initialized by another test

    let original_nos = FixNewOrderSingle {
        cl_ord_id: "IntegrationTestOrd1".to_string(),
        symbol: Symbol::new("BTC/USD"),
        side: Side::Buy,
        transact_time: 1678886400123, // Example timestamp
        order_qty: dec!(1.25),
        ord_type: OrderType::Limit,
        price: Some(dec!(60000.50)),
        tif: Some(TimeInForce::GTC),
    };

    // Encode the message
    let encoded_bytes = encode_fix_message(
        &original_nos, 
        "INTEGRATION_SENDER", 
        "INTEGRATION_TARGET", 
        101, 
        1678886400123
    )?;

    // Simulate network transfer: create a mutable buffer from encoded bytes
    let mut buffer = BytesMut::from(encoded_bytes.as_ref());

    // Decode the message
    let decoded_msg_opt = decode_fix_message(&mut buffer)?;

    assert!(decoded_msg_opt.is_some(), "Decoded message should not be None");
    let decoded_msg = decoded_msg_opt.unwrap();

    // Verify header details (optional, but good practice)
    assert_eq!(decoded_msg.header.msg_type, "D");
    assert_eq!(decoded_msg.header.sender_comp_id, "INTEGRATION_SENDER");
    assert_eq!(decoded_msg.header.target_comp_id, "INTEGRATION_TARGET");
    assert_eq!(decoded_msg.header.msg_seq_num, 101);

    // Verify body content
    if let FixMessageBody::NewOrderSingle(decoded_nos) = decoded_msg.body {
        assert_eq!(decoded_nos.cl_ord_id, original_nos.cl_ord_id);
        assert_eq!(decoded_nos.symbol, original_nos.symbol);
        assert_eq!(decoded_nos.side, original_nos.side);
        assert_eq!(decoded_nos.transact_time, original_nos.transact_time);
        assert_eq!(decoded_nos.order_qty, original_nos.order_qty);
        assert_eq!(decoded_nos.ord_type, original_nos.ord_type);
        assert_eq!(decoded_nos.price, original_nos.price);
        assert_eq!(decoded_nos.tif, original_nos.tif);
    } else {
        panic!("Decoded message body is not NewOrderSingle as expected.");
    }

    Ok(())
}

#[test]
fn test_full_fix_logon_encode_decode_flow() -> Result<(), CodecError> {
    let _ = file_logger::init_logger();

    let original_logon = FixLogon {
        encrypt_method: 0,
        heart_bt_int: 30,
        reset_seq_num_flag: Some(true),
    };

    let encoded_bytes = encode_fix_message(
        &original_logon, 
        "CLIENT_APP", 
        "FIX_SERVER", 
        1, 
        1678886000000
    )?;

    let mut buffer = BytesMut::from(encoded_bytes.as_ref());
    let decoded_msg_opt = decode_fix_message(&mut buffer)?;

    assert!(decoded_msg_opt.is_some(), "Decoded Logon message should not be None");
    let decoded_msg = decoded_msg_opt.unwrap();

    assert_eq!(decoded_msg.header.msg_type, "A");
    assert_eq!(decoded_msg.header.sender_comp_id, "CLIENT_APP");

    if let FixMessageBody::Logon(decoded_logon) = decoded_msg.body {
        assert_eq!(decoded_logon.encrypt_method, original_logon.encrypt_method);
        assert_eq!(decoded_logon.heart_bt_int, original_logon.heart_bt_int);
        assert_eq!(decoded_logon.reset_seq_num_flag, original_logon.reset_seq_num_flag);
    } else {
        panic!("Decoded message body is not Logon as expected.");
    }

    Ok(())
}

