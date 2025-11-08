//! Utility functions for authentication.

/// Parse device name from user agent string.
///
/// Attempts to extract a human-readable device name from the user agent.
/// Falls back to generic names if parsing fails.
///
/// For production use with comprehensive device detection, consider using
/// the `woothee` or `uaparser` crates.
///
/// # Examples
///
/// ```
/// use composable_rust_auth::utils::parse_device_name;
///
/// assert_eq!(parse_device_name("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)"), "Mobile Browser");
/// assert_eq!(parse_device_name("Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X)"), "Tablet Browser");
/// assert_eq!(parse_device_name("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"), "Web Browser");
/// ```
#[must_use]
pub fn parse_device_name(user_agent: &str) -> String {
    let ua_lower = user_agent.to_lowercase();

    // Check for mobile devices
    if ua_lower.contains("iphone") || ua_lower.contains("android") && !ua_lower.contains("tablet") {
        return "Mobile Browser".to_string();
    }

    // Check for tablets
    if ua_lower.contains("ipad") || ua_lower.contains("tablet") {
        return "Tablet Browser".to_string();
    }

    // Default to web browser
    "Web Browser".to_string()
}

/// Parse device type from user agent string.
///
/// Returns one of: "mobile", "tablet", "desktop"
///
/// # Examples
///
/// ```
/// use composable_rust_auth::utils::parse_device_type;
///
/// assert_eq!(parse_device_type("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)"), "mobile");
/// assert_eq!(parse_device_type("Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X)"), "tablet");
/// assert_eq!(parse_device_type("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"), "desktop");
/// ```
#[must_use]
pub fn parse_device_type(user_agent: &str) -> &'static str {
    let ua_lower = user_agent.to_lowercase();

    // Check for mobile devices
    if ua_lower.contains("iphone") || ua_lower.contains("android") && !ua_lower.contains("tablet") {
        return "mobile";
    }

    // Check for tablets
    if ua_lower.contains("ipad") || ua_lower.contains("tablet") {
        return "tablet";
    }

    // Default to desktop
    "desktop"
}

/// Validate email address format.
///
/// This performs basic RFC 5322 validation:
/// - Must contain exactly one `@`
/// - Must have non-empty local and domain parts
/// - Length must be between 3 and 255 characters
///
/// For production use, consider using the `email_address` crate for full RFC 5322 compliance.
///
/// # Examples
///
/// ```
/// use composable_rust_auth::utils::is_valid_email;
///
/// assert!(is_valid_email("user@example.com"));
/// assert!(is_valid_email("user+tag@subdomain.example.com"));
/// assert!(!is_valid_email("invalid"));
/// assert!(!is_valid_email("@example.com"));
/// assert!(!is_valid_email("user@"));
/// ```
#[must_use]
pub fn is_valid_email(email: &str) -> bool {
    // Basic validation
    if email.len() < 3 || email.len() > 255 {
        return false;
    }

    // Must contain exactly one @
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return false;
    }

    let local = parts[0];
    let domain = parts[1];

    // Local and domain parts must be non-empty
    if local.is_empty() || domain.is_empty() {
        return false;
    }

    // Domain must contain at least one dot
    if !domain.contains('.') {
        return false;
    }

    // Basic character validation (allow alphanumeric, dots, hyphens, plus, underscore)
    let valid_local_chars = |c: char| {
        c.is_alphanumeric() || c == '.' || c == '-' || c == '+' || c == '_'
    };

    let valid_domain_chars = |c: char| {
        c.is_alphanumeric() || c == '.' || c == '-'
    };

    if !local.chars().all(valid_local_chars) {
        return false;
    }

    if !domain.chars().all(valid_domain_chars) {
        return false;
    }

    // Domain parts between dots must be non-empty
    for part in domain.split('.') {
        if part.is_empty() {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_device_name_mobile() {
        assert_eq!(parse_device_name("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)"), "Mobile Browser");
        assert_eq!(parse_device_name("Mozilla/5.0 (Linux; Android 13)"), "Mobile Browser");
    }

    #[test]
    fn test_parse_device_name_tablet() {
        assert_eq!(parse_device_name("Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X)"), "Tablet Browser");
        assert_eq!(parse_device_name("Mozilla/5.0 (Linux; Android 13; Tablet)"), "Tablet Browser");
    }

    #[test]
    fn test_parse_device_name_desktop() {
        assert_eq!(parse_device_name("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"), "Web Browser");
        assert_eq!(parse_device_name("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)"), "Web Browser");
    }

    #[test]
    fn test_parse_device_type_mobile() {
        assert_eq!(parse_device_type("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)"), "mobile");
        assert_eq!(parse_device_type("Mozilla/5.0 (Linux; Android 13)"), "mobile");
    }

    #[test]
    fn test_parse_device_type_tablet() {
        assert_eq!(parse_device_type("Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X)"), "tablet");
        assert_eq!(parse_device_type("Mozilla/5.0 (Linux; Android 13; Tablet)"), "tablet");
    }

    #[test]
    fn test_parse_device_type_desktop() {
        assert_eq!(parse_device_type("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"), "desktop");
        assert_eq!(parse_device_type("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)"), "desktop");
    }

    #[test]
    fn test_valid_emails() {
        assert!(is_valid_email("user@example.com"));
        assert!(is_valid_email("user.name@example.com"));
        assert!(is_valid_email("user+tag@example.com"));
        assert!(is_valid_email("user_name@subdomain.example.com"));
        assert!(is_valid_email("user-name@example.co.uk"));
    }

    #[test]
    fn test_invalid_emails() {
        assert!(!is_valid_email("invalid"));
        assert!(!is_valid_email("@example.com"));
        assert!(!is_valid_email("user@"));
        assert!(!is_valid_email("user@@example.com"));
        assert!(!is_valid_email("user@.com"));
        assert!(!is_valid_email("user@example."));
        assert!(!is_valid_email("user@example..com"));
        assert!(!is_valid_email(""));
        assert!(!is_valid_email("a@b"));  // No dot in domain
    }

    #[test]
    fn test_email_length_limits() {
        // Too short
        assert!(!is_valid_email("a@"));

        // Valid minimum length
        assert!(is_valid_email("a@b.c"));

        // Too long (>255 chars)
        let long_email = format!("{}@example.com", "a".repeat(250));
        assert!(!is_valid_email(&long_email));
    }
}
