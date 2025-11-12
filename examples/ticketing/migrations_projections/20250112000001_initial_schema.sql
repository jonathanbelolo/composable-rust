-- Initial schema for projections (CQRS read models)
-- These are denormalized, optimized for read queries

-- Event projection for browsing/listing
CREATE TABLE events_projection (
    id UUID PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    venue VARCHAR(255) NOT NULL,
    date TIMESTAMPTZ NOT NULL,
    total_capacity INT NOT NULL,
    status VARCHAR(50) NOT NULL,  -- draft, published, sales_open, sales_closed, cancelled
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_events_date ON events_projection(date);
CREATE INDEX idx_events_status ON events_projection(status);
CREATE INDEX idx_events_venue ON events_projection(venue);

-- Sections within events
CREATE TABLE sections_projection (
    id UUID PRIMARY KEY,
    event_id UUID NOT NULL REFERENCES events_projection(id) ON DELETE CASCADE,
    name VARCHAR(100) NOT NULL,
    rows INT NOT NULL,
    seats_per_row INT NOT NULL,
    price_cents BIGINT NOT NULL,
    UNIQUE(event_id, name)
);

CREATE INDEX idx_sections_event ON sections_projection(event_id);

-- Available seats (real-time availability for seat selection)
CREATE TABLE available_seats_projection (
    id UUID PRIMARY KEY,
    event_id UUID NOT NULL REFERENCES events_projection(id) ON DELETE CASCADE,
    section VARCHAR(100) NOT NULL,
    available_count INT NOT NULL,  -- Real-time available seats in section
    reserved_count INT NOT NULL,    -- Temporarily reserved (5-minute timeout)
    sold_count INT NOT NULL,        -- Permanently sold (payment completed)
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE(event_id, section)
);

CREATE INDEX idx_available_seats_event ON available_seats_projection(event_id);

-- Reservation projection (for customer view)
CREATE TABLE reservations_projection (
    id UUID PRIMARY KEY,
    event_id UUID NOT NULL REFERENCES events_projection(id) ON DELETE CASCADE,
    customer_id UUID NOT NULL,
    section VARCHAR(100) NOT NULL,
    quantity INT NOT NULL,
    status VARCHAR(50) NOT NULL,  -- pending, confirmed, expired, cancelled
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_reservations_customer ON reservations_projection(customer_id);
CREATE INDEX idx_reservations_event ON reservations_projection(event_id);
CREATE INDEX idx_reservations_status ON reservations_projection(status);
CREATE INDEX idx_reservations_expires_at ON reservations_projection(expires_at) WHERE status = 'pending';

-- Tickets projection (final confirmed tickets)
CREATE TABLE tickets_projection (
    id UUID PRIMARY KEY,
    event_id UUID NOT NULL REFERENCES events_projection(id) ON DELETE CASCADE,
    reservation_id UUID NOT NULL REFERENCES reservations_projection(id) ON DELETE CASCADE,
    customer_id UUID NOT NULL,
    section VARCHAR(100) NOT NULL,
    row VARCHAR(10) NOT NULL,
    seat INT NOT NULL,
    price_cents BIGINT NOT NULL,
    status VARCHAR(50) NOT NULL,  -- reserved, confirmed, cancelled
    reserved_at TIMESTAMPTZ NOT NULL,
    confirmed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE(event_id, section, row, seat)
);

CREATE INDEX idx_tickets_customer ON tickets_projection(customer_id);
CREATE INDEX idx_tickets_reservation ON tickets_projection(reservation_id);
CREATE INDEX idx_tickets_event ON tickets_projection(event_id);

-- Customer projection (summary of customer info)
CREATE TABLE customers_projection (
    id UUID PRIMARY KEY,
    email VARCHAR(255) NOT NULL UNIQUE,
    name VARCHAR(255),
    total_reservations INT NOT NULL DEFAULT 0,
    total_tickets INT NOT NULL DEFAULT 0,
    total_spent_cents BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_customers_email ON customers_projection(email);

-- Sales analytics projection (per event summary)
CREATE TABLE sales_analytics_projection (
    event_id UUID PRIMARY KEY REFERENCES events_projection(id) ON DELETE CASCADE,
    total_capacity INT NOT NULL,
    tickets_sold INT NOT NULL DEFAULT 0,
    tickets_reserved INT NOT NULL DEFAULT 0,
    tickets_available INT NOT NULL,
    revenue_cents BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL
);

-- Idempotency tracking (prevent duplicate projection updates)
-- This ensures we only process each event once per projection
CREATE TABLE projection_offsets (
    projection_name VARCHAR(255) NOT NULL,
    aggregate_id UUID NOT NULL,
    last_sequence_number BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (projection_name, aggregate_id)
);

CREATE INDEX idx_projection_offsets_updated ON projection_offsets(updated_at);
