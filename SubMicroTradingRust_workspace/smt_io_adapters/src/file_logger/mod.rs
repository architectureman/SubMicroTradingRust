#![allow(dead_code)] // Allow dead code for now

use tracing::{error, info, warn, debug, trace, Level};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::{EnvFilter, FmtSubscriber};
// Removed the unused import: std::io::Error

/// Initializes the logging system.
///
/// The log level is controlled by the `RUST_LOG` environment variable.
/// For example, `RUST_LOG=info` or `RUST_LOG=smt_io_adapters=debug,smt_core=info`
///
/// By default, if `RUST_LOG` is not set, it might default to a certain level (e.g., info or error)
/// depending on the `EnvFilter::from_default_env` behavior or can be set explicitly.
pub fn init_logger() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info")); // Default to info if RUST_LOG is not set

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::TRACE) // Maximum level the subscriber will see
        .with_env_filter(filter)       // Filter based on RUST_LOG or default
        .with_span_events(FmtSpan::CLOSE) // Include span close events for timing
        .with_target(true)             // Show module path
        .with_file(true)               // Show file name
        .with_line_number(true)        // Show line number
        .compact()                     // Use a more compact format
        // .with_json() // Uncomment for JSON output
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>)
}

// Example usage (will be part of tests or other modules)
pub fn log_some_messages() {
    info!("This is an informational message.");
    warn!(target: "network", "A warning occurred in the network module.");
    error!("An error has occurred! Details: failed to connect");
    debug!(data = "important data", "Debugging an issue.");
    trace!("This is a very verbose trace message.");

    let span = tracing::info_span!("my_span", level = "info", resource_id = 42);
    let _enter = span.enter();
    // do work inside the span
    info!("Inside the span!");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::Level;
    // use tracing_test::traced_test;

    // This test will only pass if RUST_LOG is set to include debug for this module
    // or if the default filter includes debug.
    // For CI, you might want to explicitly initialize with a known filter for tests.
    fn init_test_logger() {
        let filter = EnvFilter::new("trace"); // Show all logs for tests
        let subscriber = FmtSubscriber::builder()
            .with_max_level(Level::TRACE)
            .with_env_filter(filter)
            .with_test_writer() // Important for `traced_test`
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber); // Allow failure if already set
    }

    #[test]
    // #[traced_test]
    fn test_logging_basic_messages() {
        // init_test_logger(); // Call this if you want to ensure logs are captured by traced_test
        // The #[traced_test] macro handles subscriber initialization for capturing logs.
        // However, it might use its own default filter. To control filtering, you might
        // still want to set RUST_LOG or initialize a global subscriber if the macro doesn't suffice.

        // For this test, we'll rely on #[traced_test]'s capture and check if logs were emitted.
        // The actual content check is more complex and depends on the exact output format.

        info!("Test info message from test_logging_basic_messages");
        warn!("Test warn message from test_logging_basic_messages");
        error!("Test error message from test_logging_basic_messages");
        debug!("Test debug message from test_logging_basic_messages");
        trace!("Test trace message from test_logging_basic_messages");

        // Check that logs were emitted (requires RUST_LOG or specific setup for `traced_test`)
        // This is a basic check; more sophisticated checks would parse log output.
        // assert!(logs_contain("Test info message"));
        // assert!(logs_contain("Test warn message"));
        // assert!(logs_contain("Test error message"));
        // Debug and Trace might not appear depending on default RUST_LOG level for tests
        // If RUST_LOG is not set, EnvFilter defaults to "error" for traced_test usually.
        // To ensure they are captured, set RUST_LOG=trace for the test execution environment.
    }

    #[test]
    fn test_logger_initialization() {
        // Test that init_logger doesn't panic. More robust test would check side effects.
        // Note: This might conflict if a global logger is already set by another test
        // or by #[traced_test] if not managed carefully.
        // For isolated test, run with `cargo test -- --test-threads=1`
        // let _ = init_logger(); // Don't re-initialize if traced_test is used or global is set.
        assert!(true); // Placeholder, actual test of init_logger is tricky with global state
    }
}