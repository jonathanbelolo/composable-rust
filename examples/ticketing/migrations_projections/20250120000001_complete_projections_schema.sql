-- Complete projection schema for ticketing system
-- This is a consolidated migration that creates all projection tables in one clean pass
-- Designed for fresh infrastructure - no DROP statements, all idempotent

-- =============================================================================
-- Generic Projection Infrastructure
-- =============================================================================

-- Generic projection data table (key-value store)
-- Use this for simple projections that don't need complex queries
CREATE TABLE IF NOT EXISTS projection_data (
    key TEXT PRIMARY KEY,
    data BYTEA NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_projection_data_updated
    ON projection_data(updated_at);

-- Projection checkpoint table (tracks progress through event stream)
-- Each projection maintains a checkpoint showing where it has processed
CREATE TABLE IF NOT EXISTS projection_checkpoints (
    projection_name TEXT PRIMARY KEY,
    event_offset BIGINT NOT NULL,
    event_timestamp TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_projection_checkpoints_updated
    ON projection_checkpoints(updated_at);

-- =============================================================================
-- Available Seats Projection
-- =============================================================================

-- Tracks available seats per event and section for fast availability queries
CREATE TABLE IF NOT EXISTS available_seats_projection (
    event_id UUID NOT NULL,
    section VARCHAR(100) NOT NULL,
    total_capacity INT NOT NULL,
    reserved INT NOT NULL DEFAULT 0,
    sold INT NOT NULL DEFAULT 0,
    available INT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (event_id, section)
);

CREATE INDEX IF NOT EXISTS idx_available_seats_event_id
    ON available_seats_projection(event_id);
CREATE INDEX IF NOT EXISTS idx_available_seats_section
    ON available_seats_projection(section);

-- Idempotency tracking for reservation processing
CREATE TABLE IF NOT EXISTS processed_reservations (
    reservation_id UUID PRIMARY KEY,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_processed_reservations_processed_at
    ON processed_reservations(processed_at);

-- Individual seat assignments for inventory snapshot projection
-- Provides complete denormalized view of individual seat states
CREATE TABLE IF NOT EXISTS seat_assignments (
    seat_id UUID PRIMARY KEY,
    event_id UUID NOT NULL,
    section TEXT NOT NULL,
    status TEXT NOT NULL,  -- 'available', 'reserved', 'sold'
    seat_number TEXT,  -- NULL for general admission
    reserved_by UUID,  -- Reservation ID (NULL if not reserved)
    expires_at TIMESTAMPTZ,  -- Reservation expiration (NULL if not reserved)
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Index for loading all seats for an event/section
CREATE INDEX IF NOT EXISTS idx_seat_assignments_event_section
    ON seat_assignments(event_id, section);

-- Index for filtering by status (e.g., "find available seats")
CREATE INDEX IF NOT EXISTS idx_seat_assignments_status
    ON seat_assignments(event_id, section, status);

-- Index for reservation expiration cleanup queries
CREATE INDEX IF NOT EXISTS idx_seat_assignments_expires
    ON seat_assignments(expires_at)
    WHERE expires_at IS NOT NULL;

-- =============================================================================
-- Events Projection (JSONB-based)
-- =============================================================================

-- Event catalog with full Event domain object stored as JSONB
-- Optimized for flexible queries with JSONB indexes
CREATE TABLE IF NOT EXISTS events_projection (
    id UUID PRIMARY KEY,
    data JSONB NOT NULL,           -- Full Event domain object
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- JSONB field indexes for common query patterns
CREATE INDEX IF NOT EXISTS idx_events_status
    ON events_projection((data->>'status'));
CREATE INDEX IF NOT EXISTS idx_events_date
    ON events_projection((data->'date'->>'datetime'));
CREATE INDEX IF NOT EXISTS idx_events_name
    ON events_projection((data->>'name'));

-- GIN index for full-text search and containment queries
CREATE INDEX IF NOT EXISTS idx_events_data_gin
    ON events_projection USING GIN(data);

-- =============================================================================
-- Sales Analytics Projection
-- =============================================================================

-- Sales metrics per event (denormalized for fast queries)
CREATE TABLE IF NOT EXISTS sales_analytics_projection (
    event_id UUID PRIMARY KEY,
    total_revenue BIGINT NOT NULL DEFAULT 0,  -- in cents
    tickets_sold INT NOT NULL DEFAULT 0,
    completed_reservations INT NOT NULL DEFAULT 0,
    cancelled_reservations INT NOT NULL DEFAULT 0,
    average_ticket_price BIGINT NOT NULL DEFAULT 0,  -- in cents
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_sales_analytics_revenue
    ON sales_analytics_projection(total_revenue DESC);
CREATE INDEX IF NOT EXISTS idx_sales_analytics_tickets
    ON sales_analytics_projection(tickets_sold DESC);

-- Revenue by section (one row per event-section combination)
CREATE TABLE IF NOT EXISTS sales_by_section (
    event_id UUID NOT NULL,
    section VARCHAR(100) NOT NULL,
    revenue BIGINT NOT NULL DEFAULT 0,  -- in cents
    tickets_sold INT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (event_id, section)
);

CREATE INDEX IF NOT EXISTS idx_sales_by_section_revenue
    ON sales_by_section(event_id, revenue DESC);

-- Pending reservations for sales analytics (idempotency tracking)
CREATE TABLE IF NOT EXISTS sales_pending_reservations (
    reservation_id UUID PRIMARY KEY,
    event_id UUID NOT NULL,
    section VARCHAR(100) NOT NULL,
    amount BIGINT NOT NULL,  -- in cents
    ticket_count INT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_sales_pending_event
    ON sales_pending_reservations(event_id);

-- =============================================================================
-- Customer History Projection
-- =============================================================================

-- Customer profiles (summary data)
CREATE TABLE IF NOT EXISTS customer_profiles (
    customer_id UUID PRIMARY KEY,
    total_spent BIGINT NOT NULL DEFAULT 0,  -- in cents
    total_tickets INT NOT NULL DEFAULT 0,
    purchase_count INT NOT NULL DEFAULT 0,
    favorite_section VARCHAR(100),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_customer_total_spent
    ON customer_profiles(total_spent DESC);
CREATE INDEX IF NOT EXISTS idx_customer_purchase_count
    ON customer_profiles(purchase_count DESC);

-- Individual purchases (detailed history)
CREATE TABLE IF NOT EXISTS customer_purchases (
    id BIGSERIAL PRIMARY KEY,
    customer_id UUID NOT NULL REFERENCES customer_profiles(customer_id),
    reservation_id UUID NOT NULL UNIQUE,
    event_id UUID NOT NULL,
    section VARCHAR(100) NOT NULL,
    ticket_count INT NOT NULL,
    amount_paid BIGINT NOT NULL,  -- in cents
    tickets JSONB NOT NULL,  -- Array of ticket IDs
    completed_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_customer_purchases_customer
    ON customer_purchases(customer_id, completed_at DESC);
CREATE INDEX IF NOT EXISTS idx_customer_purchases_event
    ON customer_purchases(event_id);
CREATE INDEX IF NOT EXISTS idx_customer_purchases_completed
    ON customer_purchases(completed_at DESC);

-- Customer event attendance (for fast "has_attended_event" queries)
CREATE TABLE IF NOT EXISTS customer_event_attendance (
    customer_id UUID NOT NULL,
    event_id UUID NOT NULL,
    first_attended_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (customer_id, event_id)
);

CREATE INDEX IF NOT EXISTS idx_attendance_by_event
    ON customer_event_attendance(event_id);

-- Pending reservations for customer history (idempotency tracking)
CREATE TABLE IF NOT EXISTS customer_pending_reservations (
    reservation_id UUID PRIMARY KEY,
    customer_id UUID NOT NULL,
    event_id UUID NOT NULL,
    section VARCHAR(100) NOT NULL,
    ticket_count INT NOT NULL,
    amount BIGINT NOT NULL,  -- in cents
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_customer_pending_customer
    ON customer_pending_reservations(customer_id);
