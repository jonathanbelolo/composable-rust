# Load Testing Scripts

This directory contains k6 load testing scripts for the ticketing system.

## Prerequisites

1. **Install k6**:
   ```bash
   # macOS
   brew install k6

   # Linux
   sudo gpg -k
   sudo gpg --no-default-keyring --keyring /usr/share/keyrings/k6-archive-keyring.gpg --keyserver hkp://keyserver.ubuntu.com:80 --recv-keys C5AD17C747E3415A3642D57D77C6C491D6AC1D69
   echo "deb [signed-by=/usr/share/keyrings/k6-archive-keyring.gpg] https://dl.k6.io/deb stable main" | sudo tee /etc/apt/sources.list.d/k6.list
   sudo apt-get update
   sudo apt-get install k6

   # Windows
   choco install k6
   ```

2. **Start the ticketing system**:
   ```bash
   # Start infrastructure
   docker compose up -d

   # Run migrations
   DATABASE_URL="postgresql://postgres:postgres@localhost:5436/ticketing_events" sqlx migrate run
   PROJECTION_DATABASE_URL="postgresql://postgres:postgres@localhost:5433/ticketing_projections" sqlx migrate run
   DATABASE_URL="postgresql://postgres:postgres@localhost:5435/ticketing_auth" sqlx migrate run

   # Start the server
   cargo run
   ```

3. **Set environment variable** (required for test token bypass):
   ```bash
   export AUTH_TEST_TOKEN=test-token-12345
   ```

## Running Load Tests

### Basic Run

```bash
k6 run scripts/load_test.js
```

### With Custom Event

If you want to use a pre-created event (recommended for repeated tests):

```bash
# Create an event first
curl -X POST http://localhost:8080/api/events \
  -H "Authorization: Bearer test-token-12345" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Load Test Event",
    "description": "Pre-created event for load testing",
    "start_time": "2025-12-31T20:00:00Z",
    "end_time": "2025-12-31T23:00:00Z",
    "venue_name": "Load Test Arena",
    "venue_address": "123 Test Street"
  }'

# Use the returned event_id
TEST_EVENT_ID=<event_id> k6 run scripts/load_test.js
```

### With Detailed Output

```bash
# Export results to JSON
k6 run --out json=results.json scripts/load_test.js

# Export summary
k6 run --summary-export=summary.json scripts/load_test.js

# Combine both
k6 run --out json=results.json --summary-export=summary.json scripts/load_test.js
```

### Custom Load Profile

Modify the script's `options.stages` to customize the load profile:

```javascript
stages: [
  { duration: '2m', target: 200 },  // Heavier load
  { duration: '5m', target: 200 },  // Sustained peak
  { duration: '2m', target: 0 },    // Ramp down
],
```

## Understanding Results

### Key Metrics

k6 reports several important metrics:

1. **http_req_duration**: Response time for HTTP requests
   - `avg`: Average response time
   - `p(95)`: 95th percentile (95% of requests faster than this)
   - `p(99)`: 99th percentile (99% of requests faster than this)

2. **http_req_failed**: Percentage of failed requests
   - Target: < 1% (rate < 0.01)

3. **reservation_success_rate**: Custom metric for successful reservations
   - Target: > 99% (rate > 0.99)

4. **reservation_duration**: Time to complete reservation flow
   - Target: p(95) < 600ms

### Interpreting Thresholds

The script defines SLA thresholds:

```javascript
thresholds: {
  'http_req_duration': [
    'p(95)<500',  // ✅ PASS if 95th percentile < 500ms
    'p(99)<1000', // ✅ PASS if 99th percentile < 1000ms
  ],
  'http_req_failed': [
    'rate<0.01',  // ✅ PASS if error rate < 1%
  ],
}
```

If any threshold fails, k6 will:
- Mark it with ❌ in the output
- Exit with non-zero status code
- Fail CI/CD pipelines (if integrated)

### Sample Output

```
📊 LOAD TEST SUMMARY
====================================================================

🎯 Key Metrics:
  Requests:              15234
  Failed Requests:       0.12%
  Avg Response Time:     142.34ms
  p95 Response Time:     287.56ms
  p99 Response Time:     512.89ms
  Reservation Success:   99.88%
  Inventory Errors:      18

✅ Threshold Results:
  ✅ p(95)<500: 287.56ms < 500ms
  ✅ p(99)<1000: 512.89ms < 1000ms
  ✅ http_req_failed<0.01: 0.0012 < 0.01
  ✅ reservation_success_rate>0.99: 0.9988 > 0.99
```

## Acceptance Criteria

From Phase 12.7 Testing & Validation plan:

- ✅ **p95 latency < 500ms**: 95% of requests complete in under half a second
- ✅ **Error rate < 1%**: Less than 1% of requests fail
- ✅ **Throughput > 10 reservations/second**: System sustains target load
- ✅ **No database connection exhaustion**: Connection pool handles load
- ✅ **No memory leaks**: Memory usage remains stable during test

## Monitoring During Load Test

While the load test runs, monitor:

1. **Server logs**:
   ```bash
   tail -f server.log
   ```

2. **Database connections**:
   ```bash
   docker exec ticketing-postgres-events psql -U postgres -c "SELECT count(*) FROM pg_stat_activity;"
   ```

3. **Memory usage**:
   ```bash
   docker stats
   ```

4. **Redpanda metrics**:
   ```bash
   docker exec ticketing-redpanda rpk cluster health
   ```

## Troubleshooting

### High Error Rate

If you see > 1% error rate:

1. Check server logs for errors
2. Verify database connections aren't exhausted
3. Check for memory issues
4. Reduce load (lower `target` in stages)

### Slow Response Times

If p95 > 500ms:

1. Enable SQL query logging to find slow queries
2. Check database indices
3. Review projection update performance
4. Consider connection pool size tuning

### Insufficient Inventory Errors

If you see many "Insufficient inventory" errors:

1. This is expected when inventory runs out
2. The script tracks these separately: `insufficient_inventory_errors`
3. These are business logic "errors", not system failures
4. Create events with more capacity for longer tests

## Advanced Usage

### Smoke Test (Quick Validation)

```bash
k6 run --vus 1 --duration 30s scripts/load_test.js
```

### Stress Test (Find Breaking Point)

```bash
k6 run --stage "5m:500" --stage "5m:1000" scripts/load_test.js
```

### Soak Test (Long-Running Stability)

```bash
k6 run --stage "2m:100" --stage "30m:100" --stage "2m:0" scripts/load_test.js
```

## Integration with CI/CD

Add to your CI pipeline:

```yaml
- name: Run load tests
  run: |
    # Start system
    docker compose up -d
    cargo build --release
    cargo run --release &
    sleep 10

    # Run k6
    k6 run --summary-export=summary.json scripts/load_test.js

    # Check thresholds
    if [ $? -ne 0 ]; then
      echo "Load test failed: SLA thresholds not met"
      exit 1
    fi
```

## References

- [k6 Documentation](https://k6.io/docs/)
- [k6 Thresholds](https://k6.io/docs/using-k6/thresholds/)
- [k6 Metrics](https://k6.io/docs/using-k6/metrics/)
- [Performance Testing Best Practices](https://k6.io/docs/testing-guides/performance-testing/)
