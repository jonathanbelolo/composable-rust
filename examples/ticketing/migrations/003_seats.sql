-- Individual seat tracking (read model) — enables the QueryFetcher to provide
-- available seat IDs for reservation commands.

CREATE TABLE seats (
    seat_id UUID PRIMARY KEY,
    event_id UUID NOT NULL,
    section TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'available',  -- 'available', 'reserved', 'sold'
    reservation_id UUID,                        -- NULL if available
    expires_at TIMESTAMPTZ,                     -- For reserved seats
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_seats_available ON seats (event_id, section, status)
    WHERE status = 'available';
CREATE INDEX idx_seats_reservation ON seats (reservation_id)
    WHERE reservation_id IS NOT NULL;
CREATE INDEX idx_seats_event_section ON seats (event_id, section);
