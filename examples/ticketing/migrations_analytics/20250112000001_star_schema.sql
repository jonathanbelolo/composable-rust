-- Analytics database with star schema for OLAP queries
-- Optimized for business intelligence and reporting

-- Fact table: Ticket sales (central fact table)
CREATE TABLE fact_ticket_sales (
    id BIGSERIAL PRIMARY KEY,
    ticket_id UUID NOT NULL,
    event_id UUID NOT NULL,
    customer_id UUID NOT NULL,
    reservation_id UUID NOT NULL,

    -- Time dimension (denormalized for query performance)
    sale_date DATE NOT NULL,
    sale_timestamp TIMESTAMPTZ NOT NULL,
    sale_hour INT NOT NULL,
    sale_day_of_week INT NOT NULL,  -- 0=Sunday, 6=Saturday
    sale_week_of_year INT NOT NULL,
    sale_month INT NOT NULL,
    sale_quarter INT NOT NULL,
    sale_year INT NOT NULL,

    -- Measures
    quantity INT NOT NULL DEFAULT 1,
    price_cents BIGINT NOT NULL,
    revenue_cents BIGINT NOT NULL,

    -- Additional dimensions
    section VARCHAR(100) NOT NULL,
    row VARCHAR(10) NOT NULL,
    seat INT NOT NULL,

    -- Status tracking
    status VARCHAR(50) NOT NULL,  -- confirmed, cancelled
    cancelled_at TIMESTAMPTZ,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes for common queries
CREATE INDEX idx_fact_sales_date ON fact_ticket_sales(sale_date);
CREATE INDEX idx_fact_sales_timestamp ON fact_ticket_sales(sale_timestamp);
CREATE INDEX idx_fact_sales_event ON fact_ticket_sales(event_id);
CREATE INDEX idx_fact_sales_customer ON fact_ticket_sales(customer_id);
CREATE INDEX idx_fact_sales_status ON fact_ticket_sales(status);

-- Composite indexes for common aggregation queries
CREATE INDEX idx_fact_sales_event_date ON fact_ticket_sales(event_id, sale_date);
CREATE INDEX idx_fact_sales_date_status ON fact_ticket_sales(sale_date, status);

-- Dimension table: Events
CREATE TABLE dim_events (
    id UUID PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    venue VARCHAR(255) NOT NULL,
    event_date TIMESTAMPTZ NOT NULL,
    total_capacity INT NOT NULL,
    status VARCHAR(50) NOT NULL,

    -- Denormalized attributes for reporting
    event_month INT NOT NULL,
    event_quarter INT NOT NULL,
    event_year INT NOT NULL,
    event_day_of_week INT NOT NULL,

    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_dim_events_date ON dim_events(event_date);
CREATE INDEX idx_dim_events_venue ON dim_events(venue);
CREATE INDEX idx_dim_events_status ON dim_events(status);

-- Dimension table: Customers
CREATE TABLE dim_customers (
    id UUID PRIMARY KEY,
    email VARCHAR(255) NOT NULL,
    name VARCHAR(255),

    -- Aggregated metrics
    total_purchases INT NOT NULL DEFAULT 0,
    total_spent_cents BIGINT NOT NULL DEFAULT 0,
    first_purchase_at TIMESTAMPTZ,
    last_purchase_at TIMESTAMPTZ,

    -- Customer segments (for analysis)
    customer_segment VARCHAR(50),  -- vip, regular, new

    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_dim_customers_email ON dim_customers(email);
CREATE INDEX idx_dim_customers_segment ON dim_customers(customer_segment);

-- Dimension table: Calendar (for time-based analysis)
CREATE TABLE dim_calendar (
    date_key DATE PRIMARY KEY,
    year INT NOT NULL,
    quarter INT NOT NULL,
    month INT NOT NULL,
    week_of_year INT NOT NULL,
    day_of_month INT NOT NULL,
    day_of_week INT NOT NULL,
    day_name VARCHAR(10) NOT NULL,
    month_name VARCHAR(10) NOT NULL,
    is_weekend BOOLEAN NOT NULL,
    is_holiday BOOLEAN NOT NULL DEFAULT FALSE,
    holiday_name VARCHAR(100)
);

CREATE INDEX idx_dim_calendar_year_month ON dim_calendar(year, month);
CREATE INDEX idx_dim_calendar_quarter ON dim_calendar(year, quarter);

-- Pre-populate calendar dimension with 10 years of dates
INSERT INTO dim_calendar (date_key, year, quarter, month, week_of_year, day_of_month, day_of_week, day_name, month_name, is_weekend)
SELECT
    date_series::DATE,
    EXTRACT(YEAR FROM date_series)::INT,
    EXTRACT(QUARTER FROM date_series)::INT,
    EXTRACT(MONTH FROM date_series)::INT,
    EXTRACT(WEEK FROM date_series)::INT,
    EXTRACT(DAY FROM date_series)::INT,
    EXTRACT(DOW FROM date_series)::INT,
    TO_CHAR(date_series, 'Day'),
    TO_CHAR(date_series, 'Month'),
    EXTRACT(DOW FROM date_series) IN (0, 6)
FROM generate_series(
    '2020-01-01'::DATE,
    '2030-12-31'::DATE,
    '1 day'::INTERVAL
) AS date_series;

-- Aggregated table: Daily sales summary (materialized for performance)
CREATE TABLE agg_daily_sales (
    date_key DATE NOT NULL,
    event_id UUID NOT NULL,

    -- Aggregated metrics
    tickets_sold INT NOT NULL DEFAULT 0,
    revenue_cents BIGINT NOT NULL DEFAULT 0,
    unique_customers INT NOT NULL DEFAULT 0,

    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (date_key, event_id)
);

CREATE INDEX idx_agg_daily_sales_date ON agg_daily_sales(date_key);
CREATE INDEX idx_agg_daily_sales_event ON agg_daily_sales(event_id);

-- Aggregated table: Monthly revenue by venue (for trend analysis)
CREATE TABLE agg_monthly_revenue (
    year INT NOT NULL,
    month INT NOT NULL,
    venue VARCHAR(255) NOT NULL,

    -- Aggregated metrics
    total_events INT NOT NULL DEFAULT 0,
    total_tickets_sold INT NOT NULL DEFAULT 0,
    total_revenue_cents BIGINT NOT NULL DEFAULT 0,
    avg_ticket_price_cents BIGINT NOT NULL DEFAULT 0,

    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (year, month, venue)
);

CREATE INDEX idx_agg_monthly_revenue_period ON agg_monthly_revenue(year, month);

-- Idempotency tracking for analytics ETL
CREATE TABLE analytics_offsets (
    aggregate_id UUID PRIMARY KEY,
    last_sequence_number BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
