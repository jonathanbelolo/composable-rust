//! Input validation utilities for API endpoints.
//!
//! Provides reusable validation functions for common input types to prevent:
//! - Buffer overflow attacks
//! - Database storage issues
//! - Denial of service via large payloads
//! - Malformed data injection

use composable_rust_web::error::AppError;

/// Maximum length for event/entity titles (e.g., event name, section name)
const MAX_TITLE_LENGTH: usize = 200;

/// Maximum length for long text fields (descriptions, addresses)
const MAX_DESCRIPTION_LENGTH: usize = 5000;

/// Maximum length for addresses
const MAX_ADDRESS_LENGTH: usize = 500;

/// Maximum length for city/state names
const MAX_LOCATION_NAME_LENGTH: usize = 100;

/// Maximum length for postal codes
const MAX_POSTAL_CODE_LENGTH: usize = 20;

/// Maximum length for person names
const MAX_PERSON_NAME_LENGTH: usize = 200;

/// Maximum length for payment tokens
const MAX_PAYMENT_TOKEN_LENGTH: usize = 500;

/// Maximum length for reason text (refund, cancellation)
const MAX_REASON_LENGTH: usize = 1000;

/// Maximum length for section names
const MAX_SECTION_NAME_LENGTH: usize = 100;

/// Maximum length for individual seat identifiers
const MAX_SEAT_ID_LENGTH: usize = 20;

/// Maximum number of specific seats in a single reservation
const MAX_SPECIFIC_SEATS: usize = 20;

/// Validate a non-empty string with a maximum length.
///
/// # Arguments
///
/// * `field_name` - Name of the field for error messages
/// * `value` - String value to validate
/// * `max_len` - Maximum allowed length
///
/// # Returns
///
/// `Ok(())` if valid, `Err(AppError)` if validation fails
///
/// # Errors
///
/// Returns `AppError::bad_request` if:
/// - String is empty
/// - String exceeds maximum length
pub fn validate_string_length(
    field_name: &str,
    value: &str,
    max_len: usize,
) -> Result<(), AppError> {
    if value.is_empty() {
        return Err(AppError::bad_request(format!(
            "{field_name} cannot be empty"
        )));
    }

    if value.len() > max_len {
        return Err(AppError::bad_request(format!(
            "{field_name} exceeds maximum length of {max_len} characters (got {} characters)",
            value.len()
        )));
    }

    Ok(())
}

/// Validate an optional string with a maximum length.
///
/// If the value is `None`, validation passes.
///
/// # Arguments
///
/// * `field_name` - Name of the field for error messages
/// * `value` - Optional string value to validate
/// * `max_len` - Maximum allowed length
///
/// # Returns
///
/// `Ok(())` if valid or `None`, `Err(AppError)` if validation fails
///
/// # Errors
///
/// Returns `AppError::bad_request` if:
/// - String is empty (Some(""))
/// - String exceeds maximum length
pub fn validate_optional_string_length(
    field_name: &str,
    value: &Option<String>,
    max_len: usize,
) -> Result<(), AppError> {
    if let Some(v) = value {
        validate_string_length(field_name, v, max_len)?;
    }
    Ok(())
}

/// Validate an exact string length.
///
/// # Arguments
///
/// * `field_name` - Name of the field for error messages
/// * `value` - String value to validate
/// * `expected_len` - Expected exact length
///
/// # Returns
///
/// `Ok(())` if valid, `Err(AppError)` if validation fails
///
/// # Errors
///
/// Returns `AppError::bad_request` if string length doesn't match expected length
pub fn validate_exact_length(
    field_name: &str,
    value: &str,
    expected_len: usize,
) -> Result<(), AppError> {
    if value.len() != expected_len {
        return Err(AppError::bad_request(format!(
            "{field_name} must be exactly {expected_len} characters (got {} characters)",
            value.len()
        )));
    }

    Ok(())
}

/// Validate event title.
///
/// Max length: 200 characters
pub fn validate_event_title(title: &str) -> Result<(), AppError> {
    validate_string_length("Event title", title, MAX_TITLE_LENGTH)
}

/// Validate event description.
///
/// Max length: 5000 characters
pub fn validate_event_description(description: &str) -> Result<(), AppError> {
    validate_string_length("Event description", description, MAX_DESCRIPTION_LENGTH)
}

/// Validate venue name.
///
/// Max length: 200 characters
pub fn validate_venue_name(name: &str) -> Result<(), AppError> {
    validate_string_length("Venue name", name, MAX_TITLE_LENGTH)
}

/// Validate venue address.
///
/// Max length: 500 characters
pub fn validate_venue_address(address: &str) -> Result<(), AppError> {
    validate_string_length("Venue address", address, MAX_ADDRESS_LENGTH)
}

/// Validate billing name.
///
/// Max length: 200 characters
pub fn validate_billing_name(name: &str) -> Result<(), AppError> {
    validate_string_length("Billing name", name, MAX_PERSON_NAME_LENGTH)
}

/// Validate billing address.
///
/// Max length: 500 characters
pub fn validate_billing_address(address: &str) -> Result<(), AppError> {
    validate_string_length("Billing address", address, MAX_ADDRESS_LENGTH)
}

/// Validate city name.
///
/// Max length: 100 characters
pub fn validate_city(city: &str) -> Result<(), AppError> {
    validate_string_length("City", city, MAX_LOCATION_NAME_LENGTH)
}

/// Validate state/province name.
///
/// Max length: 100 characters
pub fn validate_state(state: &str) -> Result<(), AppError> {
    validate_string_length("State/Province", state, MAX_LOCATION_NAME_LENGTH)
}

/// Validate postal code.
///
/// Max length: 20 characters
pub fn validate_postal_code(postal_code: &str) -> Result<(), AppError> {
    validate_string_length("Postal code", postal_code, MAX_POSTAL_CODE_LENGTH)
}

/// Validate ISO 3166-1 alpha-2 country code.
///
/// Must be exactly 2 uppercase letters.
pub fn validate_country_code(country: &str) -> Result<(), AppError> {
    validate_exact_length("Country code", country, 2)?;

    // Check that it's all uppercase letters
    if !country.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(AppError::bad_request(
            "Country code must be 2 uppercase letters (ISO 3166-1 alpha-2)"
        ));
    }

    Ok(())
}

/// Validate payment token from gateway.
///
/// Max length: 500 characters
pub fn validate_payment_token(token: &str) -> Result<(), AppError> {
    validate_string_length("Payment token", token, MAX_PAYMENT_TOKEN_LENGTH)
}

/// Validate credit card last four digits.
///
/// Must be exactly 4 characters
pub fn validate_last_four(last_four: &str) -> Result<(), AppError> {
    validate_exact_length("Last four digits", last_four, 4)?;

    // Check that it's all digits
    if !last_four.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::bad_request(
            "Last four digits must be numeric"
        ));
    }

    Ok(())
}

/// Validate refund reason.
///
/// Max length: 1000 characters
pub fn validate_refund_reason(reason: &str) -> Result<(), AppError> {
    validate_string_length("Refund reason", reason, MAX_REASON_LENGTH)
}

/// Validate cancellation reason.
///
/// Max length: 1000 characters
pub fn validate_cancellation_reason(reason: &str) -> Result<(), AppError> {
    validate_string_length("Cancellation reason", reason, MAX_REASON_LENGTH)
}

/// Validate section name.
///
/// Max length: 100 characters
pub fn validate_section_name(section: &str) -> Result<(), AppError> {
    validate_string_length("Section name", section, MAX_SECTION_NAME_LENGTH)
}

/// Validate specific seat selections.
///
/// - Max 20 seats per reservation
/// - Each seat ID max 20 characters
pub fn validate_specific_seats(seats: &[String]) -> Result<(), AppError> {
    if seats.is_empty() {
        return Err(AppError::bad_request(
            "Specific seats list cannot be empty (omit the field for general admission)"
        ));
    }

    if seats.len() > MAX_SPECIFIC_SEATS {
        return Err(AppError::bad_request(format!(
            "Cannot reserve more than {MAX_SPECIFIC_SEATS} specific seats at once (got {} seats)",
            seats.len()
        )));
    }

    for seat in seats {
        validate_string_length("Seat identifier", seat, MAX_SEAT_ID_LENGTH)?;
    }

    Ok(())
}

// ============================================================================
// Price Validation
// ============================================================================

/// Minimum valid price in dollars (1 cent)
const MIN_PRICE: f64 = 0.01;

/// Maximum valid price in dollars ($1 million - reasonable upper limit for tickets)
const MAX_PRICE: f64 = 1_000_000.00;

/// Maximum refund amount in dollars ($100k - reasonable upper limit for refunds)
const MAX_REFUND_AMOUNT: f64 = 100_000.00;

/// Validate a price amount (for tickets, services, etc.).
///
/// Checks:
/// - Not NaN or Infinity
/// - Positive (>= minimum)
/// - Within reasonable range (< maximum)
/// - Has valid precision (max 2 decimal places)
///
/// # Arguments
///
/// * `field_name` - Name of the field for error messages
/// * `price` - Price value to validate
/// * `min` - Minimum allowed price
/// * `max` - Maximum allowed price
///
/// # Returns
///
/// `Ok(())` if valid, `Err(AppError)` if validation fails
///
/// # Errors
///
/// Returns `AppError::bad_request` if:
/// - Price is NaN or Infinity
/// - Price is negative or zero
/// - Price exceeds maximum
/// - Price has more than 2 decimal places
pub fn validate_price(
    field_name: &str,
    price: f64,
    min: f64,
    max: f64,
) -> Result<(), AppError> {
    // Check for NaN or Infinity
    if !price.is_finite() {
        return Err(AppError::bad_request(format!(
            "{field_name} must be a valid number"
        )));
    }

    // Check minimum
    if price < min {
        return Err(AppError::bad_request(format!(
            "{field_name} must be at least ${min:.2}"
        )));
    }

    // Check maximum
    if price > max {
        return Err(AppError::bad_request(format!(
            "{field_name} cannot exceed ${max:.2}"
        )));
    }

    // Check precision (max 2 decimal places for currency)
    let cents = (price * 100.0).round();
    let reconstructed = cents / 100.0;
    // Use small epsilon to account for floating-point rounding, but catch extra decimals
    if (price - reconstructed).abs() > 0.0001 {
        return Err(AppError::bad_request(format!(
            "{field_name} can have at most 2 decimal places"
        )));
    }

    Ok(())
}

/// Validate a ticket price.
///
/// Min: $0.01, Max: $1,000,000.00
pub fn validate_ticket_price(price: f64) -> Result<(), AppError> {
    validate_price("Ticket price", price, MIN_PRICE, MAX_PRICE)
}

/// Validate a refund amount.
///
/// Min: $0.01, Max: $100,000.00
pub fn validate_refund_amount(amount: f64) -> Result<(), AppError> {
    validate_price("Refund amount", amount, MIN_PRICE, MAX_REFUND_AMOUNT)
}

/// Validate an optional refund amount.
///
/// If `None`, validation passes. If `Some`, validates the amount.
pub fn validate_optional_refund_amount(amount: Option<f64>) -> Result<(), AppError> {
    if let Some(amt) = amount {
        validate_refund_amount(amt)?;
    }
    Ok(())
}

// ============================================================================
// Safe Money Conversion
// ============================================================================

/// Safely convert f64 dollars to `Money` with overflow checking.
///
/// ✅ SECURITY FIX (HIGH #10): Safe money conversion with overflow checking
///
/// # Arguments
///
/// - `dollars`: Price in dollars as f64 (assumed already validated)
///
/// # Returns
///
/// `Money` value representing the amount in cents
///
/// # Errors
///
/// Returns error if conversion would overflow u64
pub fn dollars_to_money(dollars: f64) -> Result<crate::types::Money, AppError> {
    // Convert to cents (multiply by 100)
    let cents_f64 = dollars * 100.0;

    // Check if it fits in u64 range
    if cents_f64 < 0.0 || cents_f64 > (u64::MAX as f64) {
        return Err(AppError::bad_request(
            "Price conversion overflow: value too large",
        ));
    }

    // Round to nearest cent and convert to u64
    // round() is safe here because we've validated range above
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let cents = cents_f64.round() as u64;

    Ok(crate::types::Money::from_cents(cents))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_string_length_success() {
        assert!(validate_string_length("test", "hello", 10).is_ok());
    }

    #[test]
    fn test_validate_string_length_empty() {
        let result = validate_string_length("test", "", 10);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_string_length_too_long() {
        let result = validate_string_length("test", "hello world!", 5);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_exact_length_success() {
        assert!(validate_exact_length("test", "1234", 4).is_ok());
    }

    #[test]
    fn test_validate_exact_length_wrong() {
        let result = validate_exact_length("test", "123", 4);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_country_code_success() {
        assert!(validate_country_code("US").is_ok());
        assert!(validate_country_code("GB").is_ok());
    }

    #[test]
    fn test_validate_country_code_lowercase() {
        let result = validate_country_code("us");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_country_code_wrong_length() {
        let result = validate_country_code("USA");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_last_four_success() {
        assert!(validate_last_four("1234").is_ok());
    }

    #[test]
    fn test_validate_last_four_non_numeric() {
        let result = validate_last_four("12ab");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_specific_seats_success() {
        assert!(validate_specific_seats(&["A1".to_string(), "A2".to_string()]).is_ok());
    }

    #[test]
    fn test_validate_specific_seats_empty() {
        let result = validate_specific_seats(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_specific_seats_too_many() {
        let seats: Vec<String> = (0..25).map(|i| format!("A{i}")).collect();
        let result = validate_specific_seats(&seats);
        assert!(result.is_err());
    }
}
