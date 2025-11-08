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

/// Validate email address format (returns Result).
///
/// # Rules
///
/// - Length: 3-255 characters
/// - Must contain exactly one `@`
/// - Non-empty local and domain parts
/// - Domain must contain at least one `.`
/// - Only alphanumeric, dots, hyphens, plus, underscore allowed
/// - No control characters or dangerous characters
///
/// # Examples
///
/// ```
/// use composable_rust_auth::utils::validate_email;
///
/// assert!(validate_email("user@example.com").is_ok());
/// assert!(validate_email("user+tag@subdomain.example.com").is_ok());
/// assert!(validate_email("invalid").is_err());
/// assert!(validate_email("@example.com").is_err());
/// ```
///
/// # Errors
///
/// Returns `AuthError::InvalidInput` if validation fails.
pub fn validate_email(email: &str) -> crate::error::Result<()> {
    use crate::error::AuthError;

    // Length validation
    if email.is_empty() {
        return Err(AuthError::InvalidInput("Email cannot be empty".into()));
    }

    if email.len() < 3 {
        return Err(AuthError::InvalidInput(format!(
            "Email too short: {} < 3 chars",
            email.len()
        )));
    }

    if email.len() > 255 {
        return Err(AuthError::InvalidInput(format!(
            "Email too long: {} > 255 chars",
            email.len()
        )));
    }

    // Must contain exactly one @
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return Err(AuthError::InvalidInput(
            "Email must contain exactly one '@'".into(),
        ));
    }

    let local = parts[0];
    let domain = parts[1];

    // Local and domain parts must be non-empty
    if local.is_empty() {
        return Err(AuthError::InvalidInput("Email local part cannot be empty".into()));
    }

    if domain.is_empty() {
        return Err(AuthError::InvalidInput("Email domain cannot be empty".into()));
    }

    // Domain must contain at least one dot
    if !domain.contains('.') {
        return Err(AuthError::InvalidInput(
            "Email domain must contain at least one '.'".into(),
        ));
    }

    // Check for control characters (security)
    if email.chars().any(|c| c.is_control()) {
        return Err(AuthError::InvalidInput(
            "Email contains control characters".into(),
        ));
    }

    // Check for dangerous characters (XSS/injection prevention)
    const DANGEROUS_CHARS: &[char] = &['<', '>', '"', '\'', '&', '\\', '\0'];
    if email.chars().any(|c| DANGEROUS_CHARS.contains(&c)) {
        return Err(AuthError::InvalidInput(
            "Email contains invalid characters".into(),
        ));
    }

    // Character whitelist for local part (before @)
    // ✅ SECURITY: Restrict to ASCII alphanumeric only (no Unicode)
    //
    // Why ASCII-only?
    // - Prevents homograph attacks (e.g., Cyrillic 'а' vs Latin 'a')
    // - Prevents normalization issues (ùser vs user vs USER)
    // - Prevents display issues (emojis, control characters)
    // - Ensures consistent database collation behavior
    //
    // Allows: ASCII a-z, A-Z, 0-9, dot, hyphen, plus, underscore
    let valid_local_chars = |c: char| {
        c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+' || c == '_'
    };

    // Character whitelist for domain part (after @)
    // ✅ SECURITY: Restrict to ASCII alphanumeric only (no internationalized domains)
    // Allows: ASCII a-z, A-Z, 0-9, dot, hyphen
    let valid_domain_chars = |c: char| {
        c.is_ascii_alphanumeric() || c == '.' || c == '-'
    };

    if !local.chars().all(valid_local_chars) {
        return Err(AuthError::InvalidInput(
            "Email local part contains invalid characters".into(),
        ));
    }

    if !domain.chars().all(valid_domain_chars) {
        return Err(AuthError::InvalidInput(
            "Email domain contains invalid characters".into(),
        ));
    }

    // Domain parts between dots must be non-empty
    for part in domain.split('.') {
        if part.is_empty() {
            return Err(AuthError::InvalidInput(
                "Email domain has empty parts between dots".into(),
            ));
        }
    }

    Ok(())
}

/// Validate email address format (returns bool).
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
    validate_email(email).is_ok()
}

/// Legacy email validation (kept for backward compatibility).
#[must_use]
#[allow(dead_code)]
fn is_valid_email_legacy(email: &str) -> bool {
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

/// Validate device name.
///
/// # Rules
///
/// - Length: 1-255 characters
/// - No control characters (`\0`, `\r`, `\n`, etc.)
/// - No script injection characters (`<`, `>`, `"`, `'`, `&`)
///
/// # Examples
///
/// ```
/// use composable_rust_auth::utils::validate_device_name;
///
/// assert!(validate_device_name("iPhone 15 Pro").is_ok());
/// assert!(validate_device_name("Work Laptop").is_ok());
/// assert!(validate_device_name("").is_err()); // Empty
/// assert!(validate_device_name(&"A".repeat(256)).is_err()); // Too long
/// ```
///
/// # Errors
///
/// Returns `AuthError::InvalidInput` if validation fails.
pub fn validate_device_name(name: &str) -> crate::error::Result<()> {
    use crate::error::AuthError;

    if name.is_empty() {
        return Err(AuthError::InvalidInput("Device name cannot be empty".into()));
    }

    if name.len() > 255 {
        return Err(AuthError::InvalidInput(format!(
            "Device name too long: {} > 255 chars",
            name.len()
        )));
    }

    // Check for control characters
    if name.chars().any(|c| c.is_control()) {
        return Err(AuthError::InvalidInput(
            "Device name contains control characters".into(),
        ));
    }

    // Check for injection characters (stored XSS prevention)
    const DANGEROUS_CHARS: &[char] = &['<', '>', '"', '\'', '&', '\0'];
    if name.chars().any(|c| DANGEROUS_CHARS.contains(&c)) {
        return Err(AuthError::InvalidInput(
            "Device name contains invalid characters".into(),
        ));
    }

    Ok(())
}

/// Validate platform string.
///
/// # Rules
///
/// - Length: 1-500 characters
/// - Must be ASCII (user agents are ASCII)
///
/// # Examples
///
/// ```
/// use composable_rust_auth::utils::validate_platform;
///
/// assert!(validate_platform("Mozilla/5.0 (Windows NT 10.0; Win64; x64)").is_ok());
/// assert!(validate_platform("Linux").is_ok());
/// assert!(validate_platform("").is_err()); // Empty
/// assert!(validate_platform(&"A".repeat(501)).is_err()); // Too long
/// ```
///
/// # Errors
///
/// Returns `AuthError::InvalidInput` if validation fails.
pub fn validate_platform(platform: &str) -> crate::error::Result<()> {
    use crate::error::AuthError;

    if platform.is_empty() {
        return Err(AuthError::InvalidInput("Platform cannot be empty".into()));
    }

    if platform.len() > 500 {
        return Err(AuthError::InvalidInput(format!(
            "Platform string too long: {} > 500 chars",
            platform.len()
        )));
    }

    // Platform should be ASCII (user agents are ASCII)
    if !platform.is_ascii() {
        return Err(AuthError::InvalidInput(
            "Platform must be ASCII".into(),
        ));
    }

    Ok(())
}

/// Validate user agent string.
///
/// # Rules
///
/// - Length: 1-1000 characters
/// - Must be ASCII (user agents are ASCII per HTTP spec)
/// - No control characters
///
/// # Security
///
/// Limits length to prevent:
/// - HTTP header injection attacks (e.g., `User-Agent: foo\r\nX-Malicious: bar`)
/// - DoS via excessive memory usage
/// - Log injection attacks
///
/// # Examples
///
/// ```
/// use composable_rust_auth::utils::validate_user_agent;
///
/// assert!(validate_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)").is_ok());
/// assert!(validate_user_agent("curl/7.68.0").is_ok());
/// assert!(validate_user_agent("").is_err()); // Empty
/// assert!(validate_user_agent(&"A".repeat(1001)).is_err()); // Too long
/// ```
///
/// # Errors
///
/// Returns `AuthError::InvalidInput` if validation fails.
pub fn validate_user_agent(user_agent: &str) -> crate::error::Result<()> {
    use crate::error::AuthError;

    if user_agent.is_empty() {
        return Err(AuthError::InvalidInput("User-Agent cannot be empty".into()));
    }

    if user_agent.len() > 1000 {
        return Err(AuthError::InvalidInput(format!(
            "User-Agent too long: {} > 1000 chars",
            user_agent.len()
        )));
    }

    // User-Agent should be ASCII (per HTTP spec)
    if !user_agent.is_ascii() {
        return Err(AuthError::InvalidInput(
            "User-Agent must be ASCII".into(),
        ));
    }

    // Check for control characters (especially \r, \n for header injection)
    if user_agent.chars().any(|c| c.is_control()) {
        return Err(AuthError::InvalidInput(
            "User-Agent contains control characters (possible header injection)".into(),
        ));
    }

    Ok(())
}

/// Validate IP address format.
///
/// # Rules
///
/// - Must be a valid IPv4 or IPv6 address
/// - No leading/trailing whitespace
/// - No malicious patterns (e.g., SQL injection)
///
/// # Examples
///
/// ```
/// use composable_rust_auth::utils::validate_ip_address;
///
/// assert!(validate_ip_address("127.0.0.1").is_ok());
/// assert!(validate_ip_address("192.168.1.1").is_ok());
/// assert!(validate_ip_address("::1").is_ok());
/// assert!(validate_ip_address("2001:0db8:85a3::8a2e:0370:7334").is_ok());
/// assert!(validate_ip_address("invalid").is_err());
/// assert!(validate_ip_address("999.999.999.999").is_err());
/// ```
///
/// # Errors
///
/// Returns `AuthError::InvalidInput` if validation fails.
pub fn validate_ip_address(ip: &str) -> crate::error::Result<()> {
    use crate::error::AuthError;

    if ip.is_empty() {
        return Err(AuthError::InvalidInput("IP address cannot be empty".into()));
    }

    // Reject whitespace (prevents bypass attacks)
    if ip.trim() != ip {
        return Err(AuthError::InvalidInput(
            "IP address contains leading/trailing whitespace".into(),
        ));
    }

    // Length sanity check (prevent DoS)
    // IPv4: max 15 chars (255.255.255.255)
    // IPv6: max 45 chars (8 groups of 4 hex + 7 colons + potential IPv4 suffix)
    if ip.len() > 45 {
        return Err(AuthError::InvalidInput(format!(
            "IP address too long: {} > 45 chars",
            ip.len()
        )));
    }

    // Check for SQL injection patterns (defense-in-depth)
    const DANGEROUS_CHARS: &[char] = &['\'', '"', ';', '-', '\\', '\0'];
    if ip.chars().any(|c| DANGEROUS_CHARS.contains(&c)) {
        return Err(AuthError::InvalidInput(
            "IP address contains invalid characters".into(),
        ));
    }

    // Parse as IpAddr to validate format
    // This handles both IPv4 and IPv6
    ip.parse::<std::net::IpAddr>().map_err(|_| {
        AuthError::InvalidInput(format!("Invalid IP address format: {ip}"))
    })?;

    Ok(())
}

/// Sanitize IP address for logging.
///
/// # Privacy
///
/// Truncates the last octet of IPv4 addresses or last 64 bits of IPv6
/// addresses to protect user privacy while retaining useful geographic info.
///
/// # Examples
///
/// ```
/// use composable_rust_auth::utils::sanitize_ip_for_logging;
///
/// assert_eq!(sanitize_ip_for_logging("192.168.1.100"), "192.168.1.0");
/// assert_eq!(sanitize_ip_for_logging("2001:0db8:85a3::8a2e:0370:7334"), "2001:db8:85a3::");
/// ```
#[must_use]
pub fn sanitize_ip_for_logging(ip: &str) -> String {
    use std::net::IpAddr;

    match ip.parse::<IpAddr>() {
        Ok(IpAddr::V4(ipv4)) => {
            // Zero out last octet
            let octets = ipv4.octets();
            format!("{}.{}.{}.0", octets[0], octets[1], octets[2])
        }
        Ok(IpAddr::V6(ipv6)) => {
            // Zero out last 64 bits (interface identifier)
            let segments = ipv6.segments();
            format!(
                "{:x}:{:x}:{:x}:{:x}::",
                segments[0], segments[1], segments[2], segments[3]
            )
        }
        Err(_) => {
            // Invalid IP - return masked version
            "[invalid]".to_string()
        }
    }
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

    #[test]
    fn test_device_name_validation_valid() {
        // Valid device names
        assert!(validate_device_name("iPhone 15 Pro").is_ok());
        assert!(validate_device_name("Work Laptop").is_ok());
        assert!(validate_device_name("My Device").is_ok());
        assert!(validate_device_name("a").is_ok()); // Minimum length
        assert!(validate_device_name(&"A".repeat(255)).is_ok()); // Maximum length
    }

    #[test]
    fn test_device_name_validation_empty() {
        let result = validate_device_name("");
        assert!(result.is_err());
        assert!(matches!(result, Err(crate::error::AuthError::InvalidInput(_))));
    }

    #[test]
    fn test_device_name_validation_too_long() {
        let result = validate_device_name(&"A".repeat(256));
        assert!(result.is_err());
        assert!(matches!(result, Err(crate::error::AuthError::InvalidInput(_))));
    }

    #[test]
    fn test_device_name_validation_control_chars() {
        assert!(validate_device_name("Name\0WithNull").is_err()); // Null byte
        assert!(validate_device_name("Name\nWithNewline").is_err()); // Newline
        assert!(validate_device_name("Name\rWithCarriage").is_err()); // Carriage return
        assert!(validate_device_name("Name\tWithTab").is_err()); // Tab
    }

    #[test]
    fn test_device_name_validation_xss_prevention() {
        // XSS attack vectors
        assert!(validate_device_name("<script>alert(1)</script>").is_err());
        assert!(validate_device_name("Name<img src=x>").is_err());
        assert!(validate_device_name("Name\"onclick=\"alert(1)\"").is_err());
        assert!(validate_device_name("Name'onclick='alert(1)'").is_err());
        assert!(validate_device_name("Name&amp;Test").is_err());
    }

    #[test]
    fn test_platform_validation_valid() {
        // Valid platforms
        assert!(validate_platform("Mozilla/5.0 (Windows NT 10.0; Win64; x64)").is_ok());
        assert!(validate_platform("Linux").is_ok());
        assert!(validate_platform("Darwin").is_ok());
        assert!(validate_platform("a").is_ok()); // Minimum length
        assert!(validate_platform(&"A".repeat(500)).is_ok()); // Maximum length
    }

    #[test]
    fn test_platform_validation_empty() {
        let result = validate_platform("");
        assert!(result.is_err());
        assert!(matches!(result, Err(crate::error::AuthError::InvalidInput(_))));
    }

    #[test]
    fn test_platform_validation_too_long() {
        let result = validate_platform(&"A".repeat(501));
        assert!(result.is_err());
        assert!(matches!(result, Err(crate::error::AuthError::InvalidInput(_))));
    }

    #[test]
    fn test_platform_validation_non_ascii() {
        assert!(validate_platform("Platform™").is_err()); // Non-ASCII character
        assert!(validate_platform("платформа").is_err()); // Cyrillic
        assert!(validate_platform("プラットフォーム").is_err()); // Japanese
    }

    // ═══════════════════════════════════════════════════════════
    // Email Validation Tests (validate_email)
    // ═══════════════════════════════════════════════════════════

    #[test]
    fn test_validate_email_valid() {
        // Valid email addresses
        assert!(validate_email("user@example.com").is_ok());
        assert!(validate_email("user.name@example.com").is_ok());
        assert!(validate_email("user+tag@example.com").is_ok());
        assert!(validate_email("user_name@subdomain.example.com").is_ok());
        assert!(validate_email("user-name@example.co.uk").is_ok());
        assert!(validate_email("a@b.c").is_ok()); // Minimum valid length
    }

    #[test]
    fn test_validate_email_empty() {
        let result = validate_email("");
        assert!(result.is_err());
        assert!(matches!(result, Err(crate::error::AuthError::InvalidInput(_))));
    }

    #[test]
    fn test_validate_email_too_short() {
        assert!(validate_email("a@").is_err()); // Too short
        assert!(validate_email("ab").is_err()); // No @
    }

    #[test]
    fn test_validate_email_too_long() {
        let long_email = format!("{}@example.com", "a".repeat(250));
        let result = validate_email(&long_email);
        assert!(result.is_err());
        assert!(matches!(result, Err(crate::error::AuthError::InvalidInput(_))));
    }

    #[test]
    fn test_validate_email_no_at_sign() {
        assert!(validate_email("invalid").is_err());
        assert!(validate_email("invalid.example.com").is_err());
    }

    #[test]
    fn test_validate_email_multiple_at_signs() {
        assert!(validate_email("user@@example.com").is_err());
        assert!(validate_email("user@domain@example.com").is_err());
    }

    #[test]
    fn test_validate_email_empty_local_part() {
        assert!(validate_email("@example.com").is_err());
    }

    #[test]
    fn test_validate_email_empty_domain() {
        assert!(validate_email("user@").is_err());
    }

    #[test]
    fn test_validate_email_no_dot_in_domain() {
        assert!(validate_email("user@example").is_err());
        assert!(validate_email("a@b").is_err());
    }

    #[test]
    fn test_validate_email_empty_domain_parts() {
        assert!(validate_email("user@example.").is_err()); // Empty part after dot
        assert!(validate_email("user@.example.com").is_err()); // Empty part before dot
        assert!(validate_email("user@example..com").is_err()); // Empty part between dots
    }

    #[test]
    fn test_validate_email_control_characters() {
        assert!(validate_email("user\0@example.com").is_err()); // Null byte
        assert!(validate_email("user\n@example.com").is_err()); // Newline
        assert!(validate_email("user\r@example.com").is_err()); // Carriage return
        assert!(validate_email("user\t@example.com").is_err()); // Tab
        assert!(validate_email("user@example.com\n").is_err()); // Newline at end
    }

    #[test]
    fn test_validate_email_xss_prevention() {
        // XSS attack vectors
        assert!(validate_email("user<script>@example.com").is_err());
        assert!(validate_email("user@example.com<img>").is_err());
        assert!(validate_email("user\"onclick\"@example.com").is_err());
        assert!(validate_email("user'onclick'@example.com").is_err());
        assert!(validate_email("user&amp;@example.com").is_err());
        assert!(validate_email("user\\@example.com").is_err()); // Backslash
    }

    #[test]
    fn test_validate_email_injection_prevention() {
        // SQL injection patterns
        assert!(validate_email("user';DROP TABLE users--@example.com").is_err());
        assert!(validate_email("admin'--@example.com").is_err());

        // Command injection patterns
        assert!(validate_email("user@example.com;rm -rf /").is_err());
        assert!(validate_email("user@example.com`whoami`").is_err());
    }

    #[test]
    fn test_validate_email_invalid_local_chars() {
        assert!(validate_email("user*@example.com").is_err()); // Asterisk
        assert!(validate_email("user!@example.com").is_err()); // Exclamation
        assert!(validate_email("user#@example.com").is_err()); // Hash
        assert!(validate_email("user$@example.com").is_err()); // Dollar
        assert!(validate_email("user%@example.com").is_err()); // Percent
        assert!(validate_email("user^@example.com").is_err()); // Caret
        assert!(validate_email("user(@example.com").is_err()); // Parenthesis
        assert!(validate_email("user)@example.com").is_err()); // Parenthesis
    }

    #[test]
    fn test_validate_email_invalid_domain_chars() {
        assert!(validate_email("user@exam_ple.com").is_err()); // Underscore in domain
        assert!(validate_email("user@exam+ple.com").is_err()); // Plus in domain
        assert!(validate_email("user@exam*ple.com").is_err()); // Asterisk in domain
        assert!(validate_email("user@exam ple.com").is_err()); // Space in domain
    }

    #[test]
    fn test_validate_email_valid_special_chars() {
        // These should be ALLOWED (RFC 5322 compliant)
        assert!(validate_email("user.name@example.com").is_ok()); // Dot in local
        assert!(validate_email("user-name@example.com").is_ok()); // Hyphen in local
        assert!(validate_email("user+tag@example.com").is_ok()); // Plus in local
        assert!(validate_email("user_name@example.com").is_ok()); // Underscore in local
        assert!(validate_email("user@example-domain.com").is_ok()); // Hyphen in domain
    }

    #[test]
    fn test_validate_email_reject_unicode() {
        // ✅ SECURITY: Reject Unicode characters to prevent homograph attacks

        // Cyrillic 'а' (U+0430) looks like Latin 'a' (U+0061)
        assert!(validate_email("аdmin@example.com").is_err(), "Should reject Cyrillic characters");

        // Greek letter 'α' (U+03B1)
        assert!(validate_email("αlpha@example.com").is_err(), "Should reject Greek characters");

        // Chinese characters
        assert!(validate_email("用户@example.com").is_err(), "Should reject Chinese characters");

        // Emoji (considered alphanumeric by Unicode)
        assert!(validate_email("user😀@example.com").is_err(), "Should reject emojis");

        // Accented characters
        assert!(validate_email("üser@example.com").is_err(), "Should reject accented characters");
        assert!(validate_email("user@éxample.com").is_err(), "Should reject accented domains");

        // Mixed Unicode and ASCII (more subtle attack)
        assert!(validate_email("usеr@example.com").is_err(), "Should reject mixed Cyrillic/ASCII");

        // Ensure ASCII is still allowed
        assert!(validate_email("user@example.com").is_ok(), "Should allow pure ASCII");
        assert!(validate_email("USER123@EXAMPLE.COM").is_ok(), "Should allow uppercase ASCII");
    }

    #[test]
    fn test_is_valid_email_backward_compatibility() {
        // Ensure is_valid_email() still works for backward compatibility
        assert!(is_valid_email("user@example.com"));
        assert!(is_valid_email("user.name@example.com"));
        assert!(!is_valid_email("invalid"));
        assert!(!is_valid_email("@example.com"));
        assert!(!is_valid_email("user@"));
        assert!(!is_valid_email("user@@example.com"));
    }

    // ═══════════════════════════════════════════════════════════
    // User-Agent Validation Tests
    // ═══════════════════════════════════════════════════════════

    #[test]
    fn test_validate_user_agent_valid() {
        // Valid user agents
        assert!(validate_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)").is_ok());
        assert!(validate_user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)").is_ok());
        assert!(validate_user_agent("curl/7.68.0").is_ok());
        assert!(validate_user_agent("PostmanRuntime/7.26.8").is_ok());
        assert!(validate_user_agent("a").is_ok()); // Minimum length
        assert!(validate_user_agent(&"A".repeat(1000)).is_ok()); // Maximum length
    }

    #[test]
    fn test_validate_user_agent_empty() {
        let result = validate_user_agent("");
        assert!(result.is_err());
        assert!(matches!(result, Err(crate::error::AuthError::InvalidInput(_))));
    }

    #[test]
    fn test_validate_user_agent_too_long() {
        let result = validate_user_agent(&"A".repeat(1001));
        assert!(result.is_err());
        assert!(matches!(result, Err(crate::error::AuthError::InvalidInput(_))));
    }

    #[test]
    fn test_validate_user_agent_non_ascii() {
        assert!(validate_user_agent("Mozilla™").is_err()); // Non-ASCII character
        assert!(validate_user_agent("браузер").is_err()); // Cyrillic
        assert!(validate_user_agent("ブラウザ").is_err()); // Japanese
    }

    #[test]
    fn test_validate_user_agent_header_injection() {
        // HTTP header injection attacks
        assert!(validate_user_agent("Mozilla/5.0\r\nX-Malicious: foo").is_err()); // CRLF injection
        assert!(validate_user_agent("Mozilla/5.0\nX-Malicious: foo").is_err()); // LF injection
        assert!(validate_user_agent("Mozilla/5.0\rX-Malicious: foo").is_err()); // CR injection
        assert!(validate_user_agent("Mozilla\0WithNull").is_err()); // Null byte
        assert!(validate_user_agent("Mozilla\tWithTab").is_err()); // Tab
    }

    // ═══════════════════════════════════════════════════════════
    // IP Address Validation Tests
    // ═══════════════════════════════════════════════════════════

    #[test]
    fn test_validate_ip_address_valid_ipv4() {
        // Valid IPv4 addresses
        assert!(validate_ip_address("127.0.0.1").is_ok());
        assert!(validate_ip_address("192.168.1.1").is_ok());
        assert!(validate_ip_address("10.0.0.1").is_ok());
        assert!(validate_ip_address("255.255.255.255").is_ok());
        assert!(validate_ip_address("0.0.0.0").is_ok());
    }

    #[test]
    fn test_validate_ip_address_valid_ipv6() {
        // Valid IPv6 addresses
        assert!(validate_ip_address("::1").is_ok()); // Loopback
        assert!(validate_ip_address("::").is_ok()); // All zeros
        assert!(validate_ip_address("2001:0db8:85a3::8a2e:0370:7334").is_ok());
        assert!(validate_ip_address("2001:db8::1").is_ok());
        assert!(validate_ip_address("fe80::1").is_ok());
        assert!(validate_ip_address("::ffff:192.168.1.1").is_ok()); // IPv4-mapped IPv6
    }

    #[test]
    fn test_validate_ip_address_empty() {
        let result = validate_ip_address("");
        assert!(result.is_err());
        assert!(matches!(result, Err(crate::error::AuthError::InvalidInput(_))));
    }

    #[test]
    fn test_validate_ip_address_invalid_format() {
        assert!(validate_ip_address("invalid").is_err());
        assert!(validate_ip_address("999.999.999.999").is_err());
        assert!(validate_ip_address("192.168.1").is_err()); // Missing octet
        assert!(validate_ip_address("192.168.1.1.1").is_err()); // Too many octets
        assert!(validate_ip_address("192.168.1.").is_err()); // Trailing dot
        assert!(validate_ip_address(".192.168.1.1").is_err()); // Leading dot
    }

    #[test]
    fn test_validate_ip_address_whitespace() {
        assert!(validate_ip_address(" 192.168.1.1").is_err()); // Leading space
        assert!(validate_ip_address("192.168.1.1 ").is_err()); // Trailing space
        assert!(validate_ip_address("192.168. 1.1").is_err()); // Space in middle
    }

    #[test]
    fn test_validate_ip_address_injection_prevention() {
        // SQL injection patterns
        assert!(validate_ip_address("192.168.1.1'; DROP TABLE--").is_err());
        assert!(validate_ip_address("192.168.1.1'").is_err());
        assert!(validate_ip_address("192.168.1.1\"").is_err());
        assert!(validate_ip_address("192.168.1.1;").is_err());
        assert!(validate_ip_address("192.168.1.1--").is_err());
        assert!(validate_ip_address("192.168.1.1\\").is_err());
        assert!(validate_ip_address("192.168.1.1\0").is_err());
    }

    #[test]
    fn test_validate_ip_address_too_long() {
        // DoS prevention
        let long_ip = "1".repeat(50);
        let result = validate_ip_address(&long_ip);
        assert!(result.is_err());
    }

    // ═══════════════════════════════════════════════════════════
    // IP Sanitization Tests (Privacy)
    // ═══════════════════════════════════════════════════════════

    #[test]
    fn test_sanitize_ip_for_logging_ipv4() {
        assert_eq!(sanitize_ip_for_logging("192.168.1.100"), "192.168.1.0");
        assert_eq!(sanitize_ip_for_logging("10.0.0.255"), "10.0.0.0");
        assert_eq!(sanitize_ip_for_logging("127.0.0.1"), "127.0.0.0");
    }

    #[test]
    fn test_sanitize_ip_for_logging_ipv6() {
        assert_eq!(sanitize_ip_for_logging("2001:0db8:85a3::8a2e:0370:7334"), "2001:db8:85a3:0::");
        assert_eq!(sanitize_ip_for_logging("2001:db8::1"), "2001:db8:0:0::");
        assert_eq!(sanitize_ip_for_logging("::1"), "0:0:0:0::");
        assert_eq!(sanitize_ip_for_logging("fe80::1"), "fe80:0:0:0::");
    }

    #[test]
    fn test_sanitize_ip_for_logging_invalid() {
        assert_eq!(sanitize_ip_for_logging("invalid"), "[invalid]");
        assert_eq!(sanitize_ip_for_logging("999.999.999.999"), "[invalid]");
        assert_eq!(sanitize_ip_for_logging(""), "[invalid]");
    }
}
