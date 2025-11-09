//! OAuth2 reducer.
//!
//! This module implements the pure business logic for OAuth2 authentication.
//!
//! # Flow
//!
//! ```text
//! 1. InitiateOAuth → Generate CSRF state → RedirectToOAuthProvider effect
//! 2. User authorizes at provider
//! 3. OAuthCallback → Validate state → ExchangeOAuthCode effect
//! 4. OAuthSuccess → Emit events (batch) → CreateSession effect
//! ```
//!
//! # Event Sourcing
//!
//! The OAuth flow emits these events:
//! - UserRegistered (if new user)
//! - OAuthAccountLinked
//! - DeviceRegistered
//! - UserLoggedIn (audit trail)

use crate::actions::{AuthAction, AuthLevel};
use crate::config::OAuthConfig;
use crate::constants::login_methods;
use crate::environment::AuthEnvironment;
use crate::events::AuthEvent;
use crate::providers::{ChallengeStore, OAuth2Provider, UserRepository, DeviceRepository, SessionStore, TokenStore, RiskCalculator, EmailProvider, WebAuthnProvider, OAuthTokenStore, OAuthTokenData};
use crate::state::{AuthState, DeviceId, OAuthState, Session, SessionId, UserId};
use composable_rust_core::async_effect;
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use composable_rust_core::stream::StreamId;
use composable_rust_core::{smallvec, SmallVec};
use chrono::{Duration, Utc};
use std::sync::Arc;

/// OAuth2 reducer.
///
/// Handles OAuth2/OIDC authentication flow with CSRF protection.
#[derive(Debug, Clone)]
pub struct OAuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>
where
    O: OAuth2Provider + Clone + 'static,
    E: EmailProvider + Clone + 'static,
    W: WebAuthnProvider + Clone + 'static,
    S: SessionStore + Clone + 'static,
    T: TokenStore + Clone + 'static,
    U: UserRepository + Clone + 'static,
    D: DeviceRepository + Clone + 'static,
    R: RiskCalculator + Clone + 'static,
    OT: OAuthTokenStore + Clone + 'static,
    C: ChallengeStore + Clone + 'static,
    RL: crate::providers::RateLimiter + Clone + 'static,
{
    /// Configuration for OAuth authentication.
    config: OAuthConfig,

    /// Phantom data to hold type parameters.
    _phantom: std::marker::PhantomData<(O, E, W, S, T, U, D, R, OT, C, RL)>,
}

impl<O, E, W, S, T, U, D, R, OT, C, RL> OAuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>
where
    O: OAuth2Provider + Clone + 'static,
    E: EmailProvider + Clone + 'static,
    W: WebAuthnProvider + Clone + 'static,
    S: SessionStore + Clone + 'static,
    T: TokenStore + Clone + 'static,
    U: UserRepository + Clone + 'static,
    D: DeviceRepository + Clone + 'static,
    R: RiskCalculator + Clone + 'static,
    OT: OAuthTokenStore + Clone + 'static,
    C: ChallengeStore + Clone + 'static,
    RL: crate::providers::RateLimiter + Clone + 'static,
{
    /// Create a new OAuth reducer with default configuration.
    ///
    /// Default configuration:
    /// - Base URL: http://localhost:3000
    /// - State TTL: 5 minutes
    /// - Session duration: 24 hours
    ///
    /// For production, use `with_config()` to provide proper configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: OAuthConfig::default(),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Create a reducer with custom configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - OAuth configuration
    ///
    /// # Example
    ///
    /// ```ignore
    /// use composable_rust_auth::config::OAuthConfig;
    /// use composable_rust_auth::reducers::OAuthReducer;
    ///
    /// let config = OAuthConfig::new("https://app.example.com".to_string())
    ///     .with_state_ttl(10);
    ///
    /// let reducer: OAuthReducer<_, _, _, _, _, _, _, _> =
    ///     OAuthReducer::with_config(config);
    /// ```
    #[must_use]
    pub fn with_config(config: OAuthConfig) -> Self {
        Self {
            config,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Create a reducer with custom base URL (legacy).
    ///
    /// # Arguments
    ///
    /// * `base_url` - Base URL for OAuth redirects
    ///
    /// # Deprecated
    ///
    /// Use `with_config()` instead for full configuration.
    #[must_use]
    pub fn with_base_url(base_url: String) -> Self {
        Self {
            config: OAuthConfig::new(base_url),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Generate a cryptographically secure CSRF state parameter.
    ///
    /// Uses 32 bytes of randomness (256 bits).
    fn generate_csrf_state() -> String {
        use base64::Engine;
        let bytes: [u8; 32] = rand::random();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    /// Build redirect URI for OAuth callback.
    #[allow(dead_code)] // Will be used in Phase 6B for effect execution
    fn redirect_uri(&self) -> String {
        format!("{}/auth/oauth/callback", self.config.base_url)
    }
}

impl<O, E, W, S, T, U, D, R, OT, C, RL> Reducer for OAuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>
where
    O: OAuth2Provider + Clone + 'static,
    E: EmailProvider + Clone + 'static,
    W: WebAuthnProvider + Clone + 'static,
    S: SessionStore + Clone + 'static,
    T: TokenStore + Clone + 'static,
    U: UserRepository + Clone + 'static,
    D: DeviceRepository + Clone + 'static,
    R: RiskCalculator + Clone + 'static,
    OT: OAuthTokenStore + Clone + 'static,
    C: ChallengeStore + Clone + 'static,
    RL: crate::providers::RateLimiter + Clone + 'static,
{
    type State = AuthState;
    type Action = AuthAction;
    type Environment = AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ═══════════════════════════════════════════════════════════════════
            // Initiate OAuth Flow
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::InitiateOAuth {
                provider,
                ip_address: _,
                user_agent: _,
                fingerprint: _,
            } => {
                // Generate CSRF state parameter
                let state_param = Self::generate_csrf_state();
                let expires_at = Utc::now() + Duration::minutes(self.config.state_ttl_minutes);

                // Store OAuth state (keep for backward compatibility during migration)
                state.oauth_state = Some(OAuthState {
                    state_param: state_param.clone(),
                    provider,
                    initiated_at: Utc::now(),
                });

                // Store state in TokenStore for atomic single-use semantics
                let token_store = env.tokens.clone();
                let state_for_store = state_param.clone();
                let token_data = crate::providers::TokenData::new(
                    crate::providers::TokenType::OAuthState,
                    state_param.clone(),
                    serde_json::json!({
                        "provider": format!("{:?}", provider),
                    }),
                    expires_at,
                );

                // Build redirect URL and redirect user to OAuth provider
                let redirect_uri = self.redirect_uri();
                let oauth_provider = env.oauth.clone();

                smallvec![
                    // Effect 1: Store OAuth state atomically
                    async_effect! {
                        match token_store.store_token(&state_for_store, token_data).await {
                            Ok(()) => {
                                tracing::debug!("OAuth state stored");
                                None
                            }
                            Err(e) => {
                                // ⚡ SECURITY FIX (BLOCKER #4): Don't leak internal error details
                                tracing::error!("Failed to store OAuth state: {}", e);
                                Some(AuthAction::OAuthFailed {
                                    error: "authentication_failed".to_string(),
                                    error_description: Some("OAuth authentication failed".to_string()),
                                })
                            }
                        }
                    },
                    // Effect 2: Build authorization URL and signal redirect
                    async_effect! {
                        // Build authorization URL
                        match oauth_provider.build_authorization_url(provider, &state_param, &redirect_uri).await {
                            Ok(auth_url) => {
                                tracing::info!(
                                    provider = %provider.as_str(),
                                    "OAuth authorization URL generated"
                                );
                                // Return action for web framework to perform HTTP redirect
                                Some(AuthAction::OAuthAuthorizationUrlReady {
                                    provider,
                                    authorization_url: auth_url,
                                })
                            }
                            Err(e) => {
                                // ⚡ SECURITY FIX (BLOCKER #4): Don't leak internal error details
                                tracing::error!("Failed to generate OAuth authorization URL: {}", e);
                                Some(AuthAction::OAuthFailed {
                                    error: "authentication_failed".to_string(),
                                    error_description: Some("OAuth authentication failed".to_string()),
                                })
                            }
                        }
                    }
                ]
            }

            // ═══════════════════════════════════════════════════════════════════
            // Handle OAuth Callback
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::OAuthCallback {
                code,
                state: state_param,
                ip_address,
                user_agent,
                fingerprint,
            } => {
                // ✅ INPUT VALIDATION: Defense-in-depth validation at entry point
                if let Err(e) = crate::utils::validate_ip_address(&ip_address.to_string()) {
                    tracing::warn!(
                        error = %e,
                        ip_address = %crate::utils::sanitize_ip_for_logging(&ip_address.to_string()),
                        "Invalid IP address during OAuth callback"
                    );
                    return smallvec![async_effect! {
                        Some(AuthAction::OAuthFailed {
                            error: "invalid_request".to_string(),
                            error_description: Some("Invalid request parameters".to_string()),
                        })
                    }];
                }

                if let Err(e) = crate::utils::validate_user_agent(&user_agent) {
                    tracing::warn!(
                        error = %e,
                        user_agent_length = user_agent.len(),
                        "Invalid user agent during OAuth callback"
                    );
                    return smallvec![async_effect! {
                        Some(AuthAction::OAuthFailed {
                            error: "invalid_request".to_string(),
                            error_description: Some("Invalid request parameters".to_string()),
                        })
                    }];
                }

                // Clear OAuth state immediately (one-time use)
                // This ensures tests see the state cleared synchronously
                state.oauth_state = None;

                // ⚡ SECURITY FIX (BLOCKER #2): Atomic OAuth state consumption
                // Use TokenStore.consume_token() to atomically verify and delete
                // This prevents race conditions where two concurrent callbacks
                // both pass validation before either deletes the state.

                let token_store = env.tokens.clone();
                let state_for_consume = state_param.clone();
                let code_clone = code.clone();
                let ip_clone = ip_address;
                let ua_clone = user_agent.clone();
                let fingerprint_clone = fingerprint.clone();
                let redirect_uri = self.redirect_uri();
                let oauth_provider = env.oauth.clone();

                smallvec![async_effect! {
                    // Atomically consume OAuth state (check + delete in one operation)
                    match token_store.consume_token(&state_for_consume, &state_for_consume).await {
                        Ok(Some(token_data)) => {
                            // State was valid, not expired, and has been consumed
                            // Extract provider from token data
                            let provider_str = token_data.data["provider"]
                                .as_str()
                                .unwrap_or("");

                            // Parse provider (this is a simplified version - production needs proper parsing)
                            let provider = if provider_str.contains("Google") {
                                crate::state::OAuthProvider::Google
                            } else if provider_str.contains("GitHub") {
                                crate::state::OAuthProvider::GitHub
                            } else {
                                // ⚡ SECURITY FIX (BLOCKER #4): Don't leak provider details
                                tracing::error!("Unknown OAuth provider: {}", provider_str);
                                return Some(AuthAction::OAuthFailed {
                                    error: "authentication_failed".to_string(),
                                    error_description: Some("OAuth authentication failed".to_string()),
                                });
                            };

                            tracing::info!("OAuth state verified for provider: {:?}", provider);

                            // Exchange authorization code for access token
                            match oauth_provider.exchange_code(provider, &code_clone, &redirect_uri).await {
                                Ok(token_response) => {
                                    // Fetch user info with access token
                                    match oauth_provider.fetch_user_info(provider, &token_response.access_token).await {
                                        Ok(user_info) => {
                                            Some(AuthAction::OAuthSuccess {
                                                email: user_info.email,
                                                name: user_info.name,
                                                provider,
                                                provider_user_id: user_info.provider_user_id,
                                                access_token: token_response.access_token,
                                                refresh_token: token_response.refresh_token,
                                                ip_address: ip_clone,
                                                user_agent: ua_clone,
                                                fingerprint: fingerprint_clone,
                                            })
                                        }
                                        Err(e) => {
                                            // ⚡ SECURITY FIX (BLOCKER #4): Don't leak error details
                                            tracing::error!("Failed to fetch user info: {e}");
                                            Some(AuthAction::OAuthFailed {
                                                error: "authentication_failed".to_string(),
                                                error_description: Some("OAuth authentication failed".to_string()),
                                            })
                                        }
                                    }
                                }
                                Err(e) => {
                                    // ⚡ SECURITY FIX (BLOCKER #4): Don't leak error details
                                    tracing::error!("Failed to exchange code for token: {e}");
                                    Some(AuthAction::OAuthFailed {
                                        error: "authentication_failed".to_string(),
                                        error_description: Some("OAuth authentication failed".to_string()),
                                    })
                                }
                            }
                        }
                        Ok(None) => {
                            // State not found, already used, or expired
                            // ⚡ SECURITY FIX (BLOCKER #4): Don't leak which failure mode
                            tracing::warn!("OAuth callback validation failed");
                            Some(AuthAction::OAuthFailed {
                                error: "authentication_failed".to_string(),
                                error_description: Some("OAuth authentication failed".to_string()),
                            })
                        }
                        Err(e) => {
                            // ⚡ SECURITY FIX (BLOCKER #4): Don't leak internal error details
                            tracing::error!("OAuth state consumption failed: {}", e);
                            Some(AuthAction::OAuthFailed {
                                error: "authentication_failed".to_string(),
                                error_description: Some("OAuth authentication failed".to_string()),
                            })
                        }
                    }
                }]
            }

            // ═══════════════════════════════════════════════════════════════════
            // OAuth Success (Token Exchange Complete) - Emit events (batch)
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::OAuthSuccess {
                email,
                name,
                provider,
                provider_user_id,
                access_token,
                refresh_token,
                ip_address,
                user_agent,
                fingerprint,
            } => {
                // Validate and normalize email from OAuth provider (prevent account collision)
                let email = match crate::utils::normalize_email(&email) {
                    Ok(normalized) => normalized,
                    Err(e) => {
                        tracing::warn!("Invalid email from OAuth provider {}: {}", provider.as_str(), e);
                        return smallvec![async_effect! {
                            Some(AuthAction::OAuthFailed {
                                error: "invalid_email".to_string(),
                                error_description: Some(format!("Invalid email from OAuth provider: {e}")),
                            })
                        }];
                    }
                };

                // Generate IDs upfront
                let user_id = UserId::new();
                let device_id = DeviceId::new();
                let session_id = SessionId::new();
                let now = Utc::now();

                // Build session with placeholder risk score (will be updated via SessionCreated)
                let session = Session {
                    session_id,
                    user_id, // Will be updated with actual user_id from projection
                    device_id,
                    email: email.clone(),
                    created_at: now,
                    last_active: now,
                    expires_at: now + self.config.session_duration,
                    ip_address,
                    user_agent: user_agent.clone(),
                    oauth_provider: Some(provider),
                    login_risk_score: 0.1, // Placeholder - will be updated via SessionCreated
                    idle_timeout: self.config.idle_timeout,
                    enable_sliding_refresh: self.config.enable_sliding_session_refresh,
                };

                // Update state immediately (sessions are ephemeral, not event-sourced)
                // The risk score will be corrected when SessionCreated action is processed
                state.session = Some(session.clone());

                // Query projection and emit events
                let users = env.users.clone();
                let event_store = Arc::clone(&env.event_store);
                let sessions = env.sessions.clone();
                let risk = env.risk.clone();
                let oauth_tokens = env.oauth_tokens.clone();
                let session_duration = self.config.session_duration;
                let max_concurrent_sessions = self.config.max_concurrent_sessions;
                let idle_timeout = self.config.idle_timeout;
                let enable_sliding_refresh = self.config.enable_sliding_session_refresh;
                let email_clone = email.clone();
                let name_clone = name.clone();
                let user_agent_clone = user_agent.clone();
                let fingerprint_clone = fingerprint.clone();

                smallvec![async_effect! {
                    // Check if user exists in projection
                    let existing_user = users.get_user_by_email(&email_clone).await.ok();
                    let final_user_id = existing_user.as_ref().map_or(user_id, |u| u.user_id);

                    // Calculate login risk
                    let risk_assessment = risk.calculate_login_risk(&crate::providers::LoginContext {
                        user_id: Some(final_user_id),
                        email: email_clone.clone(),
                        ip_address,
                        user_agent: user_agent_clone.clone(),
                        device_id: Some(device_id),
                        last_login_location: None,
                        last_login_at: None,
                        fingerprint: fingerprint_clone,
                    }).await.ok().unwrap_or_else(|| {
                        // Fall back to safe default on error
                        crate::providers::RiskAssessment {
                            score: 0.1,
                            level: crate::providers::RiskLevel::Low,
                            factors: vec![],
                            recommended_auth_level: crate::actions::AuthLevel::Basic,
                        }
                    });

                    let login_risk_score = risk_assessment.score;

                    // Build events to emit (all independent, can be batched)
                    let mut events = Vec::new();

                    // 1. UserRegistered event (only if new user)
                    if existing_user.is_none() {
                        events.push(AuthEvent::UserRegistered {
                            user_id: final_user_id,
                            email: email_clone.clone(),
                            name: name_clone.clone(),
                            email_verified: true, // OAuth emails are verified
                            timestamp: now,
                        });
                    }

                    // 2. OAuthAccountLinked event (always)
                    events.push(AuthEvent::OAuthAccountLinked {
                        user_id: final_user_id,
                        provider,
                        provider_user_id: provider_user_id.clone(),
                        provider_email: email_clone.clone(),
                        timestamp: now,
                    });

                    // 3. DeviceRegistered event (always)
                    events.push(AuthEvent::DeviceRegistered {
                        device_id,
                        user_id: final_user_id,
                        name: crate::utils::parse_device_name(&user_agent_clone),
                        device_type: crate::utils::parse_device_type(&user_agent_clone).to_string(),
                        platform: user_agent_clone.clone(),
                        ip_address,
                        timestamp: now,
                    });

                    // 4. UserLoggedIn event (audit trail, always)
                    events.push(AuthEvent::UserLoggedIn {
                        user_id: final_user_id,
                        device_id,
                        session_id,
                        method: format!("{}{}", login_methods::OAUTH_PREFIX, provider.as_str()),
                        auth_level: AuthLevel::Basic,
                        ip_address,
                        user_agent: user_agent_clone.clone(),
                        risk_score: login_risk_score as f64,
                        timestamp: now,
                    });

                    // Serialize all events
                    let serialized_events: Vec<_> = events
                        .iter()
                        .filter_map(|e| e.to_serialized().ok())
                        .collect();

                    if serialized_events.is_empty() {
                        // ⚡ SECURITY FIX (BLOCKER #4): Don't leak internal error details
                        tracing::error!("No events to persist");
                        return Some(AuthAction::OAuthFailed {
                            error: "authentication_failed".to_string(),
                            error_description: Some("OAuth authentication failed".to_string()),
                        });
                    }

                    // Build session
                    let session = Session {
                        session_id,
                        user_id: final_user_id,
                        device_id,
                        email: email_clone.clone(),
                        created_at: now,
                        last_active: now,
                        expires_at: now + session_duration,
                        ip_address,
                        user_agent: user_agent_clone,
                        oauth_provider: Some(provider),
                        login_risk_score,
                        idle_timeout,
                        enable_sliding_refresh,
                    };

                    // Batch append all events to the user stream
                    let stream_id = StreamId::new(format!("user-{}", final_user_id.0));

                    match event_store.append_events(stream_id, None, serialized_events).await {
                        Ok(_version) => {
                            // Events persisted successfully
                            // Now create ephemeral session in Redis
                            if let Err(e) = sessions.create_session(&session, session_duration, max_concurrent_sessions).await {
                                // ⚡ SECURITY FIX (BLOCKER #4): Don't leak internal error details
                                tracing::error!("Failed to create OAuth session for user {} device {}: {}",
                                    final_user_id.0, device_id.0, e);
                                return Some(AuthAction::OAuthFailed {
                                    error: "authentication_failed".to_string(),
                                    error_description: Some("OAuth authentication failed".to_string()),
                                });
                            }

                            // Store OAuth tokens for future refresh (non-fatal if it fails)
                            let token_data = OAuthTokenData {
                                user_id: final_user_id,
                                provider,
                                access_token: access_token.clone(),
                                refresh_token: refresh_token.clone(),
                                expires_at: Some(now + Duration::hours(1)), // Standard OAuth token expiry
                                stored_at: now,
                            };

                            if let Err(e) = oauth_tokens.store_tokens(&token_data).await {
                                tracing::error!(
                                    "Failed to store OAuth tokens for user {} provider {}: {}",
                                    final_user_id.0,
                                    provider.as_str(),
                                    e
                                );
                                // Non-fatal - session still created, user can re-authenticate
                            }

                            // Emit SessionCreated event
                            Some(AuthAction::SessionCreated { session })
                        }
                        Err(e) => {
                            tracing::error!("Failed to persist OAuth events for user {}: {}", final_user_id.0, e);
                            Some(AuthAction::OAuthFailed {
                                error: "event_persistence_failed".to_string(),
                                error_description: Some(format!("Failed to persist events: {e}")),
                            })
                        }
                    }
                }]
            }

            // ═══════════════════════════════════════════════════════════════════
            // OAuth Failed
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::OAuthFailed {
                error: _,
                error_description: _,
            } => {
                // Clear OAuth state
                state.oauth_state = None;

                // TODO: Redirect to error page
                smallvec![Effect::None]
            }

            // ═══════════════════════════════════════════════════════════════════
            // Session Created
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::SessionCreated { session } => {
                // Set session in state (session now has correct risk score from RiskCalculator)
                state.session = Some(session.clone());
                smallvec![Effect::None]
            }

            // ═══════════════════════════════════════════════════════════════════
            // Refresh OAuth Token (Pure Orchestration)
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::RefreshOAuthToken { user_id, provider } => {
                // This is the composable-rust way:
                // Reducer orchestrates effects, doesn't execute them
                let token_store = env.oauth_tokens.clone();
                let oauth_provider = env.oauth.clone();
                let user_id_clone = user_id;
                let provider_clone = provider;

                smallvec![async_effect! {
                    // 1. Get current tokens from storage
                    let tokens = match token_store.get_tokens(user_id_clone, provider_clone).await {
                        Ok(Some(tokens)) => tokens,
                        Ok(None) => {
                            tracing::warn!(
                                user_id = %user_id_clone.0,
                                provider = %provider_clone.as_str(),
                                "No OAuth tokens found for refresh"
                            );
                            return None;
                        }
                        Err(e) => {
                            tracing::error!(
                                user_id = %user_id_clone.0,
                                provider = %provider_clone.as_str(),
                                error = %e,
                                "Failed to get OAuth tokens"
                            );
                            return None;
                        }
                    };

                    // 2. Verify we have a refresh token
                    let refresh_token = match tokens.refresh_token {
                        Some(rt) => rt,
                        None => {
                            tracing::warn!(
                                user_id = %user_id_clone.0,
                                provider = %provider_clone.as_str(),
                                "No refresh token available"
                            );
                            return None;
                        }
                    };

                    // 3. Call OAuth provider to refresh (external API call in effect)
                    match oauth_provider.refresh_token(provider_clone, &refresh_token).await {
                        Ok(token_response) => {
                            tracing::info!(
                                user_id = %user_id_clone.0,
                                provider = %provider_clone.as_str(),
                                "OAuth token refreshed successfully"
                            );

                            // 4. Update stored tokens
                            let updated_tokens = crate::providers::OAuthTokenData {
                                user_id: user_id_clone,
                                provider: provider_clone,
                                access_token: token_response.access_token.clone(),
                                // Keep old refresh token if provider doesn't return new one
                                refresh_token: token_response.refresh_token.or(Some(refresh_token)),
                                expires_at: token_response.expires_at,
                                stored_at: chrono::Utc::now(),
                            };

                            if let Err(e) = token_store.store_tokens(&updated_tokens).await {
                                tracing::error!(
                                    user_id = %user_id_clone.0,
                                    provider = %provider_clone.as_str(),
                                    error = %e,
                                    "Failed to store refreshed tokens"
                                );
                                return None;
                            }

                            // 5. Emit success event
                            Some(AuthAction::OAuthTokenRefreshed {
                                user_id: user_id_clone,
                                provider: provider_clone,
                                access_token: token_response.access_token,
                                expires_at: token_response.expires_at,
                            })
                        }
                        Err(e) => {
                            tracing::error!(
                                user_id = %user_id_clone.0,
                                provider = %provider_clone.as_str(),
                                error = %e,
                                "OAuth token refresh failed"
                            );
                            None
                        }
                    }
                }]
            }

            // ═══════════════════════════════════════════════════════════════════
            // OAuth Token Refreshed (Event)
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::OAuthTokenRefreshed {
                user_id,
                provider,
                access_token: _,
                expires_at,
            } => {
                tracing::info!(
                    user_id = %user_id.0,
                    provider = %provider.as_str(),
                    expires_at = ?expires_at,
                    "OAuth token refresh completed"
                );

                // No state changes needed - tokens already updated in storage
                // This event is for audit/logging purposes
                smallvec![Effect::None]
            }

            // ═══════════════════════════════════════════════════════════════════
            // Other Actions (Not Handled by OAuth Reducer)
            // ═══════════════════════════════════════════════════════════════════
            _ => {
                // This reducer only handles OAuth actions
                smallvec![Effect::None]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // Tests temporarily disabled - will be replaced with proper tests using mock providers
    // See TODO item: "Implement mock OAuth provider for testing"

    // use super::*;
    // use std::net::Ipv4Addr;

    // TODO: Implement mock providers and re-enable tests
}
