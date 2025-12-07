-- Create a view for users_projection to support the auth library's PostgresUserRepository
-- The auth library expects a `users_projection` table with `user_id` column,
-- but the ticketing app uses `users` table with `id` column.

-- Create view that maps column names
CREATE VIEW users_projection AS
SELECT
    id AS user_id,
    email,
    name,
    email_verified,
    created_at,
    updated_at
FROM users;

-- Create INSERT rule to make the view updatable
-- This allows PostgresUserRepository.create_user() to work
CREATE OR REPLACE RULE users_projection_insert AS
ON INSERT TO users_projection
DO INSTEAD
INSERT INTO users (id, email, name, email_verified, created_at, updated_at)
VALUES (NEW.user_id, NEW.email, NEW.name, NEW.email_verified, NEW.created_at, NEW.updated_at);

-- Create UPDATE rule to support user updates through the view
CREATE OR REPLACE RULE users_projection_update AS
ON UPDATE TO users_projection
DO INSTEAD
UPDATE users
SET
    email = NEW.email,
    name = NEW.name,
    email_verified = NEW.email_verified,
    updated_at = NEW.updated_at
WHERE id = OLD.user_id;

-- Create DELETE rule to support user deletion through the view
CREATE OR REPLACE RULE users_projection_delete AS
ON DELETE TO users_projection
DO INSTEAD
DELETE FROM users WHERE id = OLD.user_id;
