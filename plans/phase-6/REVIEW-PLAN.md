# Phase 6 In-Depth Code Review Plan

**Status**: 🔍 Ready to Begin
**Purpose**: Comprehensive review of all auth crate implementations
**Goal**: Identify and fix TODOs, hardcoded values, incomplete implementations, and security issues

---

## 📋 Review Checklist (Per Component)

For each component reviewed, verify:

- ✅ **TODOs** - Document decision or implement missing functionality
- ✅ **Hardcoded Values** - Extract to configuration or constants
- ✅ **Missing Error Handling** - Add proper error paths and recovery
- ✅ **Incomplete Implementations** - Finish or document why deferred
- ✅ **Security Issues** - Check for timing attacks, validation gaps, injection risks
- ✅ **Type Safety** - Verify we're using appropriate types (no stringly-typed data)
- ✅ **Documentation** - Ensure all public APIs are documented
- ✅ **Test Coverage** - Identify gaps in test scenarios
- ✅ **Event Sourcing Alignment** - Verify proper event emission and projection handling

---

## 🎯 Review Phases (By Risk & Complexity)

### **Phase 1: Core Business Logic (Highest Risk)**

These components contain the critical authentication flows and are most likely to have security issues, incomplete implementations, or hardcoded values.

#### 1. Magic Link Reducer (`auth/src/reducers/magic_link.rs`)
**Priority**: HIGH
**Risk Level**: MEDIUM
**Estimated Time**: 1-2 hours

**Review Focus**:
- Token generation: Is it cryptographically secure? (256-bit random)
- Token storage: Are we hashing tokens? Or storing plaintext?
- Expiration handling: Is TTL configurable? Default reasonable?
- Rate limiting: Are we preventing abuse? (5 links per hour mentioned)
- Email sending: Error handling complete?
- Event emission: Are all events emitted properly?
- Device fingerprinting: Is "Web Browser" hardcoded?
- User creation flow: Race conditions possible?

**Known TODOs to Check**:
- Magic link base URL configuration
- Device name parsing from user agent
- Rate limiting implementation

**Security Concerns**:
- Timing attacks on token comparison (should use `constant_time_eq`)
- Token reuse prevention (single-use enforcement)
- Email address validation

---

#### 2. OAuth Reducer (`auth/src/reducers/oauth.rs`)
**Priority**: HIGH
**Risk Level**: HIGH
**Estimated Time**: 2-3 hours

**Review Focus**:
- CSRF state generation: Truly random? Sufficient entropy?
- State storage: Race conditions? Expiration handling?
- State validation: Constant-time comparison?
- Token exchange: Error handling complete?
- Provider configuration: Hardcoded client IDs/secrets?
- Access token storage: Are we storing OAuth tokens? Where?
- Refresh token handling: Implemented or deferred?
- Provider user ID mapping: Correct field extraction?
- Email verification assumption: Always trust provider email?
- Device fingerprinting: Same hardcoded "Web Browser"?

**Known TODOs to Check**:
- OAuth access token and refresh token storage
- Actual provider user ID (currently using placeholder)
- Real OAuth2Provider implementation vs mocks

**Security Concerns**:
- CSRF state must be unpredictable and single-use
- State must expire (5 minutes mentioned - verify)
- Token storage security (are tokens encrypted?)
- Redirect URI validation

---

#### 3. Passkey Reducer (`auth/src/reducers/passkey.rs`)
**Priority**: HIGH
**Risk Level**: CRITICAL
**Estimated Time**: 2-3 hours

**Review Focus**:
- Challenge generation: WebAuthn-compliant?
- Challenge storage: Where? How long? (5 minutes mentioned)
- Counter rollback protection: Fully implemented?
- Public key storage: Correct format (COSE)?
- Credential ID uniqueness: Enforced?
- Origin validation: Implemented in provider?
- RP ID validation: Configurable?
- Authenticator selection: User verification requirements?
- Device linking: Correct association with device_id?
- PasskeyUsed event: Is it being emitted?

**Known TODOs to Check**:
- Challenge retrieval from state (currently "mock_challenge_id")
- WebAuthn configuration (origin, rp_id)
- Counter update mechanism via projections

**Security Concerns**:
- **CRITICAL**: Counter rollback detection (replay attacks)
- Challenge must be single-use
- Origin must match expected value
- User verification enforcement
- Attestation validation (registration)

---

### **Phase 2: Event Sourcing Infrastructure**

Critical for data integrity and audit trail.

#### 4. Events (`auth/src/events.rs`)
**Priority**: HIGH
**Risk Level**: MEDIUM
**Estimated Time**: 1 hour

**Review Focus**:
- Event completeness: Do we have all necessary fields?
- Missing events: LoginFailed? PasswordReset (if applicable)?
- Event versioning: Consistent (all .v1)?
- Serialization: Bincode appropriate? Migration strategy?
- Field types: Using proper domain types (UserId, not strings)?
- Timestamp consistency: All events have timestamps?
- IP address logging: Privacy concerns documented?

**Known Issues**:
- DeviceTrustLevel was unused (now removed from import)
- Need to verify all events match reducer emissions

**Questions**:
- Should we have `MagicLinkSent` event for audit trail?
- Do we need `SessionExpired` event?
- Should counter updates be events or projections only?

---

#### 5. Projection System (`auth/src/projection.rs`)
**Priority**: HIGH
**Risk Level**: HIGH
**Estimated Time**: 2 hours

**Review Focus**:
- Event handler completeness: All events handled?
- Idempotency: Can events be replayed safely?
- Error handling: What happens on DB failure?
- Schema alignment: Do SQL queries match migration?
- Missing handlers: Are audit events intentionally ignored?
- Trust level calculation: How is progressive trust determined?
- ON CONFLICT behavior: Correct for each table?
- Cascade behavior: Proper foreign key handling?

**Known Issues**:
- Just implemented - needs thorough validation
- SQL queries use string literals for enums (device_type, trust_level)
- No checkpoint tracking yet

**Questions**:
- Should we update `updated_at` on every event or only certain ones?
- How do we handle DeviceTrustedByUser → trust level mapping?
- Should LoginAttempted be stored for analytics?

---

### **Phase 3: Provider Implementations**

Ensure mocks are realistic and real implementations are correct.

#### 6. Mock Providers (`auth/src/mocks/`)
**Priority**: MEDIUM
**Risk Level**: LOW
**Estimated Time**: 1-2 hours

**Review Focus**:
- OAuth2Provider mock: Realistic token generation?
- EmailProvider mock: Tracking sent emails?
- WebAuthnProvider mock: Proper challenge/response simulation?
- UserRepository mock: Race conditions in HashMap access?
- DeviceRepository mock: Complete implementation?
- SessionStore mock: TTL expiration working?
- RiskCalculator mock: Configurable scenarios?

**Known Issues**:
- Mocks use HashMap with Mutex - check for deadlocks
- Some mocks may not implement all trait methods

---

#### 7. Store Implementations (`auth/src/stores/`)
**Priority**: HIGH
**Risk Level**: MEDIUM
**Estimated Time**: 1-2 hours

**Review Focus**:
- Redis session store: Connection pooling?
- Session serialization: Efficient format?
- TTL handling: Correct Redis commands?
- Error handling: Retry logic?
- PostgreSQL device repository: Query correctness?
- Connection management: Pool exhaustion handling?

**Known Issues**:
- PostgresDeviceRepository needs refactoring to query-only
- May have direct CRUD operations to remove

---

### **Phase 4: Supporting Infrastructure**

Ensure completeness and consistency.

#### 8. Provider Traits (`auth/src/providers/*.rs`)
**Priority**: MEDIUM
**Risk Level**: LOW
**Estimated Time**: 1 hour

**Review Focus**:
- Method completeness: Any missing operations?
- Return types: Proper error handling?
- Async signatures: Using RPITIT correctly?
- Documentation: All methods documented?
- Query-only enforcement: Documented and verified?

**Files to Review**:
- `providers/oauth.rs` - OAuth2Provider trait
- `providers/email.rs` - EmailProvider trait
- `providers/webauthn.rs` - WebAuthnProvider trait
- `providers/session.rs` - SessionStore trait
- `providers/user.rs` - UserRepository trait
- `providers/device.rs` - DeviceRepository trait
- `providers/risk.rs` - RiskCalculator trait

---

#### 9. State & Actions (`auth/src/state.rs`, `auth/src/actions.rs`)
**Priority**: MEDIUM
**Risk Level**: LOW
**Estimated Time**: 30 minutes

**Review Focus**:
- Missing fields: Any data we need but don't have?
- Type safety: Using NewType pattern correctly?
- Serialization: Proper derives?
- Documentation: Complete?
- Constants: Any hardcoded values to extract?

---

#### 10. Error Handling (`auth/src/error.rs`)
**Priority**: MEDIUM
**Risk Level**: LOW
**Estimated Time**: 30 minutes

**Review Focus**:
- Error variant completeness: Missing cases?
- Error context: Enough information for debugging?
- User-facing vs internal errors: Proper separation?
- Conversion implementations: Complete From impls?
- Security: Not leaking sensitive information?

---

## 🔍 Review Process (Per Component)

For each component, follow this process:

1. **Scan for TODOs**
   ```bash
   grep -n "TODO\|FIXME\|XXX\|HACK" <file>
   ```

2. **Identify Hardcoded Values**
   - Look for string literals that should be config
   - Magic numbers without const declarations
   - URLs, timeouts, limits, etc.

3. **Check Error Handling**
   - Every `.await?` should have proper error context
   - No `.unwrap()` or `.expect()` in library code
   - Errors should be logged at appropriate level

4. **Verify Security**
   - Timing attack resistance (use constant_time_eq)
   - Input validation (email, URLs, tokens)
   - SQL injection prevention (parameterized queries)
   - XSS prevention (no raw HTML in emails)

5. **Test Coverage Analysis**
   - Are happy paths tested?
   - Are error paths tested?
   - Are edge cases tested?
   - Are security properties tested?

6. **Document Findings**
   - Create GitHub issues for each TODO
   - Document decisions for deferred items
   - Update code with improved documentation

---

## 📊 Review Output Format

For each component reviewed, create a report:

### Component: `<component_name>`

#### TODOs Found
- [ ] Line X: Description - **Decision**: [Implement Now / Defer / Won't Fix]

#### Hardcoded Values
- [ ] Line X: Value - **Solution**: Extract to `<const/config>`

#### Missing Error Handling
- [ ] Line X: Issue - **Fix**: Add proper error handling

#### Security Issues
- [ ] Line X: Vulnerability - **Severity**: [Critical/High/Medium/Low]

#### Test Gaps
- [ ] Scenario: Description - **Priority**: [High/Medium/Low]

#### Recommendations
- Bullet point list of improvements

---

## 🎯 Success Criteria

Review is complete when:

- [ ] All TODOs are either fixed or documented as deferred
- [ ] No hardcoded values remain (all extracted to config/constants)
- [ ] All error paths have proper handling
- [ ] No security issues remain (or documented as known risks)
- [ ] Test coverage gaps are documented
- [ ] All public APIs have documentation
- [ ] Code passes `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] Code passes `cargo test --all-features`

---

## 📅 Estimated Timeline

| Phase | Components | Estimated Time | Priority |
|-------|-----------|----------------|----------|
| Phase 1 | Magic Link, OAuth, Passkey Reducers | 5-8 hours | HIGH |
| Phase 2 | Events, Projections | 3 hours | HIGH |
| Phase 3 | Mocks, Stores | 2-3 hours | MEDIUM |
| Phase 4 | Traits, State, Errors | 2 hours | MEDIUM |

**Total Estimated Time**: 12-16 hours

---

## 🚀 Getting Started

**Recommended Starting Point**: Magic Link Reducer

**Reasons**:
1. Simplest authentication flow
2. Good baseline for establishing review process
3. Fewer dependencies than OAuth or Passkey
4. Security critical but less complex than WebAuthn

**Next Steps**:
1. Review Magic Link Reducer thoroughly
2. Document findings in format above
3. Fix critical issues immediately
4. Create issues for deferred items
5. Move to OAuth Reducer
6. Continue through remaining phases

---

## 📝 Notes

- This review assumes event sourcing implementation is complete
- Focus is on code quality, security, and completeness
- Not about adding new features - about finishing existing ones
- Security issues take absolute priority
- Document all decisions for audit trail
