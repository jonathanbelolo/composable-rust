-- Projections database schema for the ticketing system
-- Relational schema optimized for read queries

-- ═══════════════════════════════════════════════════════════════════════════
-- Events Projection
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE events (
    event_id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    owner_id UUID NOT NULL,
    venue JSONB NOT NULL,
    event_date TIMESTAMPTZ NOT NULL,
    pricing_tiers JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft',
    cancellation_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_events_owner ON events(owner_id);
CREATE INDEX idx_events_status ON events(status);
CREATE INDEX idx_events_date ON events(event_date);

-- ═══════════════════════════════════════════════════════════════════════════
-- Inventory Projection (seat availability per section)
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE inventory (
    event_id UUID NOT NULL,
    section TEXT NOT NULL,
    total_capacity INTEGER NOT NULL,
    available_seats INTEGER NOT NULL,
    reserved_seats INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (event_id, section)
);

CREATE INDEX idx_inventory_event ON inventory(event_id);

-- ═══════════════════════════════════════════════════════════════════════════
-- Reservations (inventory-side tracking for expiration)
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE reservations (
    reservation_id UUID PRIMARY KEY,
    event_id UUID NOT NULL,
    section TEXT NOT NULL,
    seat_count INTEGER NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_reservations_event ON reservations(event_id);
CREATE INDEX idx_reservations_expiration ON reservations(status, expires_at)
    WHERE status = 'active';

-- ═══════════════════════════════════════════════════════════════════════════
-- Reservations Projection (full reservation details for API queries)
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE reservations_projection (
    id UUID PRIMARY KEY,
    event_id UUID NOT NULL,
    customer_id UUID NOT NULL,
    section TEXT NOT NULL,
    quantity INTEGER NOT NULL,
    status TEXT NOT NULL,
    total_amount_cents BIGINT NOT NULL,
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ
);

CREATE INDEX idx_reservations_projection_customer ON reservations_projection(customer_id, created_at DESC);
CREATE INDEX idx_reservations_projection_event ON reservations_projection(event_id);
CREATE INDEX idx_reservations_projection_status ON reservations_projection(status);

-- ═══════════════════════════════════════════════════════════════════════════
-- Payments Projection
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE payments (
    payment_id UUID PRIMARY KEY,
    reservation_id UUID NOT NULL,
    customer_id UUID NOT NULL,
    amount_cents BIGINT NOT NULL,
    payment_method TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'processing',
    transaction_id TEXT,
    failure_reason TEXT,
    refund_amount_cents BIGINT,
    refund_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_payments_reservation ON payments(reservation_id);
CREATE INDEX idx_payments_customer ON payments(customer_id);
CREATE INDEX idx_payments_status ON payments(status);

-- ═══════════════════════════════════════════════════════════════════════════
-- Sales Analytics Projection
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE sales_analytics_projection (
    event_id UUID PRIMARY KEY,
    total_revenue BIGINT NOT NULL DEFAULT 0,
    tickets_sold INTEGER NOT NULL DEFAULT 0,
    completed_reservations INTEGER NOT NULL DEFAULT 0,
    cancelled_reservations INTEGER NOT NULL DEFAULT 0,
    average_ticket_price BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE sales_analytics_sections (
    event_id UUID NOT NULL,
    section TEXT NOT NULL,
    revenue BIGINT NOT NULL DEFAULT 0,
    tickets_sold INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (event_id, section)
);

-- ═══════════════════════════════════════════════════════════════════════════
-- Customer Analytics
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE customer_profiles (
    customer_id UUID PRIMARY KEY,
    total_spent BIGINT NOT NULL DEFAULT 0,
    total_tickets INTEGER NOT NULL DEFAULT 0,
    purchase_count INTEGER NOT NULL DEFAULT 0,
    favorite_section TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_customer_profiles_spent ON customer_profiles(total_spent DESC);

CREATE TABLE customer_event_attendance (
    customer_id UUID NOT NULL,
    event_id UUID NOT NULL,
    first_attended_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (customer_id, event_id)
);

CREATE INDEX idx_customer_attendance_event ON customer_event_attendance(event_id);

CREATE TABLE customer_purchases (
    id BIGSERIAL PRIMARY KEY,
    customer_id UUID NOT NULL,
    reservation_id UUID NOT NULL UNIQUE,
    event_id UUID NOT NULL,
    section TEXT NOT NULL,
    ticket_count INTEGER NOT NULL,
    amount_paid BIGINT NOT NULL,
    completed_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_customer_purchases_customer ON customer_purchases(customer_id, completed_at DESC);
CREATE INDEX idx_customer_purchases_event ON customer_purchases(event_id);
