# Grafana Dashboards for Agent Systems

This directory contains production-ready Grafana dashboard definitions for monitoring agent systems using Prometheus metrics.

## Available Dashboards

### 1. Agent Overview (`agent-overview.json`)

**Purpose**: High-level operational view of agent system health and activity.

**Panels**:
- Agent Execution Duration (p50, p95, p99)
- Active Agents (gauge)
- Tool Invocation Rate
- Pattern Usage Distribution
- Error Rate by Type (with alerting)
- LLM Token Usage by Model
- Agent Execution Summary Table

**Use Cases**:
- Real-time operational monitoring
- Capacity planning
- Tool usage analysis
- Pattern adoption tracking

**Recommended Refresh**: 10 seconds

### 2. Performance & SLOs (`performance-slo.json`)

**Purpose**: Track Service Level Objectives (SLOs) and performance budgets.

**Panels**:
- Request Success Rate with 99.9% SLO (with alerting)
- P99 Latency by Agent with 1s SLO (with alerting)
- Error Budget Remaining (30 days)
- Total Requests, Average Latency, Error Rate (stats)
- Latency Heatmap
- Tool Success Rate
- SLO Compliance Table (24h)

**SLO Definitions**:
- **Availability SLO**: 99.9% success rate
- **Latency SLO**: P99 < 1 second
- **Error Budget**: 0.1% (calculated over 30 days)

**Use Cases**:
- SLO monitoring and alerting
- Performance regression detection
- Error budget tracking
- Incident response

**Recommended Refresh**: 30 seconds

## Installation

### Prerequisites

1. **Prometheus** scraping your agent metrics:
   ```yaml
   scrape_configs:
     - job_name: 'agent-system'
       static_configs:
         - targets: ['localhost:9090']  # Your agent metrics endpoint
       scrape_interval: 15s
   ```

2. **Grafana** v10.0+ with Prometheus datasource configured.

### Import Dashboards

#### Method 1: Grafana UI

1. Open Grafana UI
2. Navigate to **Dashboards** → **Import**
3. Click **Upload JSON file**
4. Select one of the JSON files from this directory
5. Choose your Prometheus datasource
6. Click **Import**

#### Method 2: Grafana API

```bash
# Import agent overview dashboard
curl -X POST \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_GRAFANA_API_KEY" \
  -d @agent-overview.json \
  http://your-grafana:3000/api/dashboards/db

# Import performance/SLO dashboard
curl -X POST \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_GRAFANA_API_KEY" \
  -d @performance-slo.json \
  http://your-grafana:3000/api/dashboards/db
```

#### Method 3: Provisioning (Recommended for Production)

Create a provisioning file in Grafana:

```yaml
# /etc/grafana/provisioning/dashboards/agent-dashboards.yaml
apiVersion: 1

providers:
  - name: 'Agent Dashboards'
    orgId: 1
    folder: 'Agents'
    type: file
    disableDeletion: false
    updateIntervalSeconds: 10
    allowUiUpdates: true
    options:
      path: /var/lib/grafana/dashboards/agents
```

Copy dashboard files:
```bash
sudo mkdir -p /var/lib/grafana/dashboards/agents
sudo cp *.json /var/lib/grafana/dashboards/agents/
sudo chown -R grafana:grafana /var/lib/grafana/dashboards/
```

Restart Grafana:
```bash
sudo systemctl restart grafana-server
```

## Alerting

### Configured Alerts

Both dashboards include pre-configured alerts:

#### Agent Overview Dashboard

1. **High Error Rate**
   - **Condition**: Error rate > 1 error/sec for 5 minutes
   - **Severity**: Warning
   - **Action**: Investigate recent deployments, check logs

#### Performance & SLO Dashboard

1. **Success Rate Below SLO**
   - **Condition**: Success rate < 99.9% for 5 minutes
   - **Severity**: Critical
   - **Action**: Check error budget, investigate failing agents

2. **P99 Latency Above SLO**
   - **Condition**: P99 latency > 1 second for 5 minutes
   - **Severity**: Critical
   - **Action**: Check for slow tools, database issues, or resource constraints

### Alert Notifications

Configure notification channels in Grafana:

```bash
# Slack
curl -X POST \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -d '{
    "name": "Agent Alerts",
    "type": "slack",
    "settings": {
      "url": "https://hooks.slack.com/services/YOUR/SLACK/WEBHOOK"
    }
  }' \
  http://your-grafana:3000/api/alert-notifications

# PagerDuty
curl -X POST \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -d '{
    "name": "Agent Critical Alerts",
    "type": "pagerduty",
    "settings": {
      "integrationKey": "YOUR_PAGERDUTY_KEY"
    }
  }' \
  http://your-grafana:3000/api/alert-notifications
```

## Customization

### Adding Custom Panels

Example: Add a custom panel for a specific agent:

```json
{
  "id": 10,
  "gridPos": { "h": 8, "w": 12, "x": 0, "y": 32 },
  "type": "graph",
  "title": "My Custom Agent Performance",
  "targets": [
    {
      "expr": "histogram_quantile(0.95, sum(rate(agent_execution_duration_seconds_bucket{agent_name=\"my_agent\"}[5m])) by (le))",
      "legendFormat": "p95 latency",
      "refId": "A"
    }
  ]
}
```

### Modifying SLO Thresholds

Edit the dashboard JSON and update threshold values:

```json
"thresholds": [
  {
    "value": 99.9,  // Change this to your SLO target
    "colorMode": "critical",
    "op": "lt"
  }
]
```

## Troubleshooting

### No Data in Dashboards

1. **Check Prometheus is scraping metrics**:
   ```bash
   curl http://localhost:9090/api/v1/query?query=agent_active_count
   ```

2. **Verify Grafana datasource**:
   - Go to **Configuration** → **Data Sources**
   - Test your Prometheus connection

3. **Check time range**:
   - Ensure dashboard time range has data
   - Try "Last 1 hour"

### Incorrect Metric Values

1. **Verify metric names** match your implementation:
   ```bash
   curl http://localhost:9090/api/v1/label/__name__/values | grep agent_
   ```

2. **Check label names** in your code match dashboard queries
   - Dashboard uses: `agent_name`, `tool_name`, `pattern_name`
   - Verify these match your `record_*()` calls

### Alerting Not Working

1. **Enable alerting** in Grafana config:
   ```ini
   [alerting]
   enabled = true
   execute_alerts = true
   ```

2. **Check alert states**:
   - Go to **Alerting** → **Alert Rules**
   - Verify rules are evaluating

3. **Test notifications**:
   - Go to **Alerting** → **Notification Channels**
   - Click **Test** to verify connectivity

## Performance Tips

### High Cardinality

If you have many agents/tools, use recording rules in Prometheus:

```yaml
# prometheus.yml
rule_files:
  - "agent_rules.yml"
```

```yaml
# agent_rules.yml
groups:
  - name: agent_aggregations
    interval: 30s
    rules:
      - record: agent:execution_duration:p99
        expr: histogram_quantile(0.99, sum(rate(agent_execution_duration_seconds_bucket[5m])) by (le, agent_name))

      - record: agent:success_rate
        expr: sum(rate(agent_execution_duration_seconds_count{status="success"}[5m])) by (agent_name) / sum(rate(agent_execution_duration_seconds_count[5m])) by (agent_name)
```

Then use the recorded metrics in dashboards for faster queries.

### Dashboard Load Time

1. **Reduce query time ranges** for expensive queries
2. **Use variables** for dynamic filtering
3. **Enable caching** in Grafana:
   ```ini
   [caching]
   enabled = true
   ```

## Resources

- [Prometheus Querying](https://prometheus.io/docs/prometheus/latest/querying/basics/)
- [Grafana Dashboard Best Practices](https://grafana.com/docs/grafana/latest/best-practices/best-practices-for-creating-dashboards/)
- [SLO/SLI Guide](https://sre.google/workbook/implementing-slos/)
- [Alert Rule Best Practices](https://grafana.com/docs/grafana/latest/alerting/alerting-rules/)

## Support

For issues or questions:
1. Check the main `agent-patterns` documentation
2. Review Prometheus metrics implementation in `src/metrics.rs`
3. Verify your metrics endpoint is exposing data correctly
