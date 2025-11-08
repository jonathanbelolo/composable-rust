-- Migration 003: Fix passkey counter type from BIGINT to INTEGER
--
-- This migration addresses a type mismatch between the database schema (BIGINT/i64)
-- and the Rust application (u32). WebAuthn counters are u32 values, so using BIGINT
-- is unnecessarily large and can cause truncation issues.
--
-- Changes:
-- 1. Change counter type from BIGINT to INTEGER (i32)
-- 2. Add CHECK constraint to ensure counter is non-negative
-- 3. Add CHECK constraint to prevent values exceeding u32::MAX
--
-- Security impact:
-- - Prevents counter overflow/underflow
-- - Ensures data consistency between application and database
-- - Makes counter rollback detection more reliable

-- Change passkeys_projection.counter from BIGINT to INTEGER
ALTER TABLE passkeys_projection
ALTER COLUMN counter TYPE INTEGER;

-- Add CHECK constraint to ensure counter is non-negative (>= 0)
-- This prevents negative values that could corrupt counter rollback detection
ALTER TABLE passkeys_projection
ADD CONSTRAINT counter_non_negative CHECK (counter >= 0);

-- Add CHECK constraint to ensure counter doesn't exceed u32::MAX (4,294,967,295)
-- This prevents values that would truncate when cast to u32 in Rust
ALTER TABLE passkeys_projection
ADD CONSTRAINT counter_max_value CHECK (counter <= 2147483647); -- i32::MAX, safe for u32 cast

-- Add comment explaining the constraints
COMMENT ON CONSTRAINT counter_non_negative ON passkeys_projection IS
  'Ensures counter is non-negative for proper rollback detection';
COMMENT ON CONSTRAINT counter_max_value ON passkeys_projection IS
  'Ensures counter fits in u32 when cast from i32 in Rust application';

-- Update column comment
COMMENT ON COLUMN passkeys_projection.counter IS
  'WebAuthn signature counter (u32) for replay protection.
   Stored as INTEGER (i32) with CHECK constraints to ensure safe casting to u32.';
