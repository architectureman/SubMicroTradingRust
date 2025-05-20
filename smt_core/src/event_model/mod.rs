#![allow(dead_code)] // Allow dead code for now as events are being defined

use crate::common_types::{Order, OrderID, Price, Quantity, Symbol, Timestamp};

// Market Data Events
#[derive(Debug, Clone, PartialEq)]
pub enum MarketEvent {
    Trade {
        symbol: Symbol,
        price: Price,
        quantity: Quantity,
        timestamp: Timestamp,
        // Potentially add trade_id, aggressor_side, etc.
    },
    TopOfBookUpdate {
        symbol: Symbol,
        bid_price: Option<Price>,
        bid_quantity: Option<Quantity>,
        ask_price: Option<Price>,
        ask_quantity: Option<Quantity>,
        timestamp: Timestamp,
    },
    DepthUpdate {
        symbol: Symbol,
        bids: Vec<(Price, Quantity)>, // List of (price, quantity) for bids
        asks: Vec<(Price, Quantity)>, // List of (price, quantity) for asks
        timestamp: Timestamp,
        is_snapshot: bool, // True if this is a full snapshot, false if incremental
    },
    // Add other market events like InstrumentStatus, News, etc.
}

// Order Lifecycle Events
#[derive(Debug, Clone, PartialEq)]
pub enum OrderEvent {
    // Events initiated by the trading system / OMS
    OrderAccepted {
        order_id: OrderID,
        client_order_id: String,
        symbol: Symbol,
        timestamp: Timestamp,
    },
    OrderRejected {
        client_order_id: Option<String>, // May not have an internal OrderID if rejected early
        order_id: Option<OrderID>,
        symbol: Symbol,
        reason: String, // Reason for rejection
        timestamp: Timestamp,
    },
    OrderCancelled {
        order_id: OrderID,
        timestamp: Timestamp,
    },
    CancelRejected {
        order_id: OrderID,
        reason: String,
        timestamp: Timestamp,
    },
    OrderReplaced {
        original_order_id: OrderID, // ID of the order that was replaced
        new_order_id: OrderID,      // ID of the new (replacing) order
        new_order_details: Box<Order>, // Contains the full state of the new order
        timestamp: Timestamp,
    },
    ReplaceRejected {
        order_id: OrderID,
        reason: String,
        timestamp: Timestamp,
    },
    OrderExpired {
        order_id: OrderID,
        timestamp: Timestamp,
    },
    // Events originating from the exchange/matching engine
    OrderPartiallyFilled {
        order_id: OrderID,
        fill_price: Price,
        fill_quantity: Quantity,
        leaves_quantity: Quantity, // Remaining quantity on the order
        trade_id: String,        // Exchange-assigned trade ID
        timestamp: Timestamp,
    },
    OrderFilled {
        order_id: OrderID,
        fill_price: Price,        // Could be average price if filled in multiple parts not individually reported
        fill_quantity: Quantity,    // Total quantity filled for this event (could be last part or full)
        trade_id: Option<String>, // Exchange-assigned trade ID, if applicable
        timestamp: Timestamp,
    },
    // Potentially other events like OrderStatusUpdate from exchange
}

// System-level or Administrative Events (Optional, can be expanded)
#[derive(Debug, Clone, PartialEq)]
pub enum SystemEvent {
    Connected {
        session_id: String, // Identifier for the session (e.g., FIX session comp IDs)
        timestamp: Timestamp,
    },
    Disconnected {
        session_id: String,
        reason: Option<String>,
        timestamp: Timestamp,
    },
    Heartbeat {
        session_id: String,
        timestamp: Timestamp,
    },
    Error {
        message: String,
        details: Option<String>,
        timestamp: Timestamp,
    },
}

// A generic event enum to encompass all event types if needed for a central bus
// Or, keep them separate and have different handlers/streams for each type
#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    Market(MarketEvent),
    Order(OrderEvent),
    System(SystemEvent),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common_types::{Side, OrderType, TimeInForce};
    use rust_decimal_macros::dec;

    #[test]
    fn market_event_creation() {
        let trade = MarketEvent::Trade {
            symbol: Symbol::new("BTC/USD"),
            price: dec!(50000.0),
            quantity: dec!(0.5),
            timestamp: 1234567890,
        };
        match trade {
            MarketEvent::Trade { symbol, price, .. } => {
                assert_eq!(symbol.as_str(), "BTC/USD");
                assert_eq!(price, dec!(50000.0));
            }
            _ => panic!("Incorrect MarketEvent type"),
        }
    }

    #[test]
    fn order_event_creation() {
        let accepted = OrderEvent::OrderAccepted {
            order_id: 1001,
            client_order_id: "client_abc".to_string(),
            symbol: Symbol::new("ETH/USD"),
            timestamp: 1234567891,
        };
        if let OrderEvent::OrderAccepted { order_id, .. } = accepted {
            assert_eq!(order_id, 1001);
        } else {
            panic!("Incorrect OrderEvent type");
        }

        let filled = OrderEvent::OrderFilled {
            order_id: 1002,
            fill_price: dec!(4000.0),
            fill_quantity: dec!(2.0),
            trade_id: Some("trade_xyz".to_string()),
            timestamp: 1234567892,
        };
        if let OrderEvent::OrderFilled { fill_price, .. } = filled {
            assert_eq!(fill_price, dec!(4000.0));
        } else {
            panic!("Incorrect OrderEvent type");
        }
    }

    #[test]
    fn app_event_wrapping() {
        let market_ev = MarketEvent::TopOfBookUpdate {
            symbol: Symbol::new("SOL/USD"),
            bid_price: Some(dec!(150.0)),
            bid_quantity: Some(dec!(10.0)),
            ask_price: Some(dec!(150.5)),
            ask_quantity: Some(dec!(12.0)),
            timestamp: 1234567893,
        };
        let app_ev = AppEvent::Market(market_ev.clone());

        match app_ev {
            AppEvent::Market(MarketEvent::TopOfBookUpdate { symbol, .. }) => {
                assert_eq!(symbol.as_str(), "SOL/USD");
            }
            _ => panic!("Incorrect AppEvent wrapping or type"),
        }
    }
}

