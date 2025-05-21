#![allow(dead_code)] // Allow dead code for now as types are being defined

use rust_decimal::Decimal;
// Removed the unused import from here

// Basic numeric types
pub type Price = Decimal;
pub type Quantity = Decimal;
pub type OrderID = u64;
pub type Timestamp = u64; // nanoseconds since epoch, or as appropriate

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Symbol(pub [u8; 16]); // Example: Fixed-size array for symbol, adjust as needed

impl Symbol {
    pub fn new(s: &str) -> Self {
        let mut arr = [0u8; 16];
        let bytes = s.as_bytes();
        let len = std::cmp::min(bytes.len(), 16);
        arr[..len].copy_from_slice(&bytes[..len]);
        Symbol(arr)
    }

    pub fn as_str(&self) -> &str {
        // Find the first null byte or end of array
        let end = self.0.iter().position(|&x| x == 0).unwrap_or(16);
        std::str::from_utf8(&self.0[..end]).unwrap_or("") // Handle potential UTF-8 errors gracefully
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OrderType {
    Market,
    Limit,
    // Add other order types as needed: Stop, StopLimit, etc.
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OrderStatus {
    New,             // The order has been accepted by the system but not yet processed by the matching engine
    PartiallyFilled,
    Filled,
    Cancelled,       // The order has been cancelled by the user or system
    Rejected,        // The order has been rejected by the system (e.g. risk violation, invalid parameters)
    PendingCancel,   // A cancel request has been received, but the order is not yet cancelled
    PendingReplace,  // A replace request has been received, but the order is not yet replaced/re-evaluated
    Expired,         // The order has expired due to time-in-force constraints
    // Add other statuses as needed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimeInForce {
    Day,     // Good for the Day
    GTC,     // Good Till Cancel
    IOC,     // Immediate Or Cancel
    FOK,     // Fill Or Kill
    GTD,     // Good Till Date
    // Add other TIFs as needed
}

// Example of a more complex type: Order
#[derive(Debug, Clone, PartialEq)] // Removed Copy as Order can be larger
pub struct Order {
    pub id: OrderID,
    pub symbol: Symbol,
    pub side: Side,
    pub order_type: OrderType,
    pub quantity: Quantity,
    pub price: Option<Price>, // Price is optional for Market orders
    pub status: OrderStatus,
    pub tif: TimeInForce,
    pub timestamp: Timestamp, // Time of order creation or last update
    pub client_order_id: String, // Optional: client assigned ID
}

impl Order {
    // Basic constructor - more sophisticated builders can be added
    pub fn new(
        id: OrderID,
        symbol: Symbol,
        side: Side,
        order_type: OrderType,
        quantity: Quantity,
        price: Option<Price>,
        tif: TimeInForce,
        timestamp: Timestamp,
        client_order_id: String,
    ) -> Self {
        Order {
            id,
            symbol,
            side,
            order_type,
            quantity,
            price,
            status: OrderStatus::New,
            tif,
            timestamp,
            client_order_id,
        }
    }
}

// You might want to add tests in a sub-module or a separate tests file
#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec; // Moved the import here where it's used

    #[test]
    fn symbol_creation_and_conversion() {
        let sym = Symbol::new("EUR/USD");
        assert_eq!(sym.as_str(), "EUR/USD");

        let long_sym = Symbol::new("VERYLONGSYMBOLNAMEEXCEEDINGLIMIT");
        assert_eq!(long_sym.as_str(), "VERYLONGSYMBOLNA"); // Truncated
    }

    #[test]
    fn order_creation() {
        let order = Order::new(
            1,
            Symbol::new("AAPL"),
            Side::Buy,
            OrderType::Limit,
            dec!(100.0),
            Some(dec!(150.25)),
            TimeInForce::Day,
            0, // Replace with actual timestamping logic
            "client_ord_123".to_string(),
        );
        assert_eq!(order.id, 1);
        assert_eq!(order.status, OrderStatus::New);
        assert_eq!(order.price, Some(dec!(150.25)));
    }
}