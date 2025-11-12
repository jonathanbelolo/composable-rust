#!/bin/bash
# Script to fix common documentation issues in auth crate

cd /Users/jonathanbelolo/dev/claude/code/composable-rust/auth/src || exit 1

# Fix common technical terms that need backticks (case-insensitive matches in docs)
find . -name "*.rs" -type f -exec sed -i '' -E '
    # In documentation comments only, add backticks around common technical terms
    /^[[:space:]]*\/\/\// {
        # OAuth variations
        s/([^`])OAuth([^`_a-zA-Z0-9])/\1`OAuth`\2/g
        s/([^`])OAuth2([^`_a-zA-Z0-9])/\1`OAuth2`\2/g
        s/([^`])OIDC([^`_a-zA-Z0-9])/\1`OIDC`\2/g

        # WebAuthn variations
        s/([^`])WebAuthn([^`_a-zA-Z0-9])/\1`WebAuthn`\2/g
        s/([^`])FIDO2([^`_a-zA-Z0-9])/\1`FIDO2`\2/g

        # JWT/Token related
        s/([^`])JWT([^`_a-zA-Z0-9])/\1`JWT`\2/g
        s/([^`])JWK([^`_a-zA-Z0-9])/\1`JWK`\2/g
        s/([^`])TOTP([^`_a-zA-Z0-9])/\1`TOTP`\2/g

        # Database/Storage
        s/([^`])Redis([^`_a-zA-Z0-9])/\1`Redis`\2/g
        s/([^`])PostgreSQL([^`_a-zA-Z0-9])/\1`PostgreSQL`\2/g
        s/([^`])Postgres([^`_a-zA-Z0-9])/\1`Postgres`\2/g

        # Protocols/Formats
        s/([^`])CSRF([^`_a-zA-Z0-9])/\1`CSRF`\2/g
        s/([^`])JSON([^`_a-zA-Z0-9])/\1`JSON`\2/g
        s/([^`])PKCE([^`_a-zA-Z0-9])/\1`PKCE`\2/g
        s/([^`])TTL([^`_a-zA-Z0-9])/\1`TTL`\2/g
        s/([^`])UUID([^`_a-zA-Z0-9])/\1`UUID`\2/g

        # Bare URLs - wrap in angle brackets
        s/"(https?:\/\/[^"]+)"/`<\1>`/g
    }
' {} \;

echo "Documentation fixes applied!"
