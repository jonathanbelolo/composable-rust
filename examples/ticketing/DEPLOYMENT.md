# Production Deployment Guide

This guide covers deploying the Ticketing application to production.

## Choose Your Deployment Platform

### Simple Managed Platforms (Recommended for Most Users)

**Best for**: Indie developers, startups, rapid prototyping, MVP deployments

- **Fly.io** - Global CDN, managed PostgreSQL, $5-10/month MVP ([Full Guide](deploy/fly/README.md))
- **Railway.app** - Instant PostgreSQL, simple pricing, $5/month ([Full Guide](deploy/railway/README.md))
- **Render.com** - Free tier available, infrastructure-as-code ([Full Guide](deploy/render/README.md))
- **Local Docker** - Development and testing ([Quick Start](#local-docker))

**Quick Start**:
```bash
# Deploy to Fly.io (recommended)
./scripts/deploy.sh fly

# Deploy to Railway
./scripts/deploy.sh railway

# Deploy locally with Docker
./scripts/deploy.sh docker
```

See the **[`deploy/`](deploy/)** directory for platform-specific configuration files and detailed guides.

### Enterprise Cloud Platforms (Advanced Users)

**Best for**: Large organizations, complex infrastructure requirements, enterprise compliance

- **AWS** - ECS/EKS with RDS and MSK ([See below](#aws-deployment))
- **GCP** - Cloud Run/GKE with Cloud SQL ([See below](#gcp-deployment))
- **Azure** - ACI/AKS with Azure Database and Event Hubs ([See below](#azure-deployment))

These deployments require more setup but provide enterprise-grade features like multi-region replication, advanced monitoring, and compliance certifications.

---

## Table of Contents

1. [Local Docker Deployment](#local-docker-deployment)
2. [AWS Deployment](#aws-deployment)
3. [GCP Deployment](#gcp-deployment)
4. [Azure Deployment](#azure-deployment)
5. [Configuration Reference](#configuration-reference)
6. [Monitoring & Observability](#monitoring--observability)
7. [Security Best Practices](#security-best-practices)
8. [Troubleshooting](#troubleshooting)

---

## Local Docker Deployment

**Best for**: Development, testing, local demos

### Quick Start

```bash
# Start all services (app + databases + Redpanda)
./scripts/deploy.sh docker

# Or use docker-compose directly
docker-compose up -d

# Verify deployment
curl http://localhost:8080/health

# View logs
docker-compose logs -f

# Stop services
docker-compose down
```

### What Gets Deployed

- **Application**: Ticketing service on port 8080
- **PostgreSQL** (×3): Event store, projections, auth databases
- **Redpanda**: Event bus for saga coordination
- **Volumes**: Persistent data storage for databases

See [`docker-compose.yml`](docker-compose.yml) for full configuration.

---

## AWS Deployment

### Architecture

- **Compute**: ECS Fargate or EKS
- **Database**: Amazon RDS for PostgreSQL (Multi-AZ)
- **Event Streaming**: Amazon MSK (Managed Streaming for Apache Kafka)
- **Secrets**: AWS Secrets Manager
- **Monitoring**: CloudWatch + Prometheus + Grafana

### Step 1: Set up RDS PostgreSQL

```bash
# Create a PostgreSQL instance
aws rds create-db-instance \
  --db-instance-identifier ticketing-db \
  --db-instance-class db.t3.medium \
  --engine postgres \
  --engine-version 15.3 \
  --master-username admin \
  --master-user-password <your-password> \
  --allocated-storage 100 \
  --storage-type gp3 \
  --storage-encrypted \
  --multi-az \
  --vpc-security-group-ids sg-xxxxx \
  --db-subnet-group-name my-subnet-group \
  --backup-retention-period 7 \
  --preferred-backup-window "03:00-04:00" \
  --preferred-maintenance-window "mon:04:00-mon:05:00"

# Enable SSL/TLS
aws rds modify-db-instance \
  --db-instance-identifier ticketing-db \
  --ca-certificate-identifier rds-ca-2019 \
  --apply-immediately
```

### Step 2: Set up Amazon MSK

```bash
# Create MSK cluster
aws kafka create-cluster \
  --cluster-name ticketing-events \
  --broker-node-group-info file://broker-config.json \
  --encryption-info file://encryption-config.json \
  --enhanced-monitoring PER_BROKER \
  --kafka-version 3.5.1 \
  --number-of-broker-nodes 3

# Example broker-config.json:
{
  "InstanceType": "kafka.m5.large",
  "ClientSubnets": [
    "subnet-xxxxx",
    "subnet-yyyyy",
    "subnet-zzzzz"
  ],
  "SecurityGroups": ["sg-xxxxx"],
  "StorageInfo": {
    "EbsStorageInfo": {
      "VolumeSize": 100
    }
  }
}

# Example encryption-config.json:
{
  "EncryptionAtRest": {
    "DataVolumeKMSKeyId": "arn:aws:kms:us-east-1:123456789:key/xxxxx"
  },
  "EncryptionInTransit": {
    "ClientBroker": "TLS",
    "InCluster": true
  }
}
```

### Step 3: Store Secrets in AWS Secrets Manager

```bash
# Create secret for database credentials
aws secretsmanager create-secret \
  --name ticketing/postgres \
  --secret-string '{
    "username": "admin",
    "password": "your-secure-password",
    "host": "ticketing-db.xxxxx.us-east-1.rds.amazonaws.com",
    "port": "5432",
    "database": "ticketing"
  }'

# Create secret for MSK credentials (if using SASL)
aws secretsmanager create-secret \
  --name ticketing/msk \
  --secret-string '{
    "username": "your-username",
    "password": "your-secure-password"
  }'
```

### Step 4: Configure Environment Variables

```bash
# .env.production for AWS
DATABASE_URL=postgresql://admin:password@ticketing-db.xxxxx.us-east-1.rds.amazonaws.com:5432/ticketing?sslmode=require
DATABASE_MAX_CONNECTIONS=20
DATABASE_MIN_CONNECTIONS=5
DATABASE_SSL_MODE=require
DATABASE_SSL_ROOT_CERT=/app/certs/rds-ca-2019-root.pem

REDPANDA_BROKERS=b-1.ticketing-events.xxxxx.kafka.us-east-1.amazonaws.com:9096,b-2.ticketing-events.xxxxx.kafka.us-east-1.amazonaws.com:9096
REDPANDA_SECURITY_PROTOCOL=sasl_ssl
REDPANDA_SASL_MECHANISM=SCRAM-SHA-512
REDPANDA_SASL_USERNAME=your-username
REDPANDA_SASL_PASSWORD=your-password
```

### Step 5: Deploy to ECS/EKS

**ECS Task Definition Example:**

```json
{
  "family": "ticketing-app",
  "networkMode": "awsvpc",
  "requiresCompatibilities": ["FARGATE"],
  "cpu": "1024",
  "memory": "2048",
  "containerDefinitions": [
    {
      "name": "ticketing",
      "image": "your-ecr-repo/ticketing:latest",
      "portMappings": [
        {
          "containerPort": 8080,
          "protocol": "tcp"
        },
        {
          "containerPort": 9090,
          "protocol": "tcp"
        }
      ],
      "secrets": [
        {
          "name": "DATABASE_URL",
          "valueFrom": "arn:aws:secretsmanager:us-east-1:123456789:secret:ticketing/postgres:url::"
        }
      ],
      "logConfiguration": {
        "logDriver": "awslogs",
        "options": {
          "awslogs-group": "/ecs/ticketing",
          "awslogs-region": "us-east-1",
          "awslogs-stream-prefix": "ticketing"
        }
      }
    }
  ]
}
```

---

## GCP Deployment

### Architecture

- **Compute**: Cloud Run or GKE
- **Database**: Cloud SQL for PostgreSQL (High Availability)
- **Event Streaming**: Self-managed RedPanda on GKE or Confluent Cloud
- **Secrets**: Secret Manager
- **Monitoring**: Cloud Monitoring + Prometheus + Grafana

### Step 1: Set up Cloud SQL PostgreSQL

```bash
# Create Cloud SQL instance
gcloud sql instances create ticketing-db \
  --database-version=POSTGRES_15 \
  --tier=db-custom-2-8192 \
  --region=us-central1 \
  --network=default \
  --availability-type=REGIONAL \
  --storage-type=SSD \
  --storage-size=100GB \
  --storage-auto-increase \
  --backup \
  --backup-start-time=03:00 \
  --maintenance-window-day=SUN \
  --maintenance-window-hour=4 \
  --require-ssl

# Create database
gcloud sql databases create ticketing --instance=ticketing-db

# Create user
gcloud sql users create ticketing-user \
  --instance=ticketing-db \
  --password=your-secure-password
```

### Step 2: Set up RedPanda on GKE

```bash
# Create GKE cluster
gcloud container clusters create ticketing-redpanda \
  --zone=us-central1-a \
  --num-nodes=3 \
  --machine-type=n2-standard-4 \
  --enable-autorepair \
  --enable-autoupgrade

# Install RedPanda using Helm
helm repo add redpanda https://charts.redpanda.com/
helm install redpanda redpanda/redpanda \
  --set storage.persistentVolume.size=100Gi \
  --set resources.cpu.cores=2 \
  --set resources.memory.container.max=4Gi
```

### Step 3: Store Secrets in Secret Manager

```bash
# Create database password secret
echo -n "your-secure-password" | gcloud secrets create ticketing-db-password \
  --data-file=- \
  --replication-policy="automatic"

# Create connection string secret
gcloud secrets create ticketing-db-url \
  --data-file=- \
  --replication-policy="automatic" <<EOF
postgresql://ticketing-user:password@/ticketing?host=/cloudsql/your-project:us-central1:ticketing-db&sslmode=require
EOF
```

### Step 4: Deploy to Cloud Run

```bash
# Build and push container
gcloud builds submit --tag gcr.io/your-project/ticketing

# Deploy to Cloud Run
gcloud run deploy ticketing \
  --image gcr.io/your-project/ticketing \
  --platform managed \
  --region us-central1 \
  --allow-unauthenticated \
  --add-cloudsql-instances your-project:us-central1:ticketing-db \
  --set-secrets DATABASE_URL=ticketing-db-url:latest \
  --set-env-vars REDPANDA_BROKERS=redpanda-0.redpanda.svc.cluster.local:9092 \
  --cpu 2 \
  --memory 4Gi \
  --max-instances 10 \
  --concurrency 80
```

---

## Azure Deployment

### Architecture

- **Compute**: Azure Container Instances or AKS
- **Database**: Azure Database for PostgreSQL (High Availability)
- **Event Streaming**: Azure Event Hubs (Kafka-compatible) or self-managed RedPanda on AKS
- **Secrets**: Azure Key Vault
- **Monitoring**: Azure Monitor + Prometheus + Grafana

### Step 1: Set up Azure Database for PostgreSQL

```bash
# Create resource group
az group create --name ticketing-rg --location eastus

# Create PostgreSQL server
az postgres flexible-server create \
  --resource-group ticketing-rg \
  --name ticketing-db \
  --location eastus \
  --admin-user adminuser \
  --admin-password <your-password> \
  --sku-name Standard_D2s_v3 \
  --tier GeneralPurpose \
  --storage-size 128 \
  --version 15 \
  --high-availability Enabled \
  --backup-retention 7

# Configure firewall (allow your IPs)
az postgres flexible-server firewall-rule create \
  --resource-group ticketing-rg \
  --name ticketing-db \
  --rule-name AllowMyIP \
  --start-ip-address <your-ip> \
  --end-ip-address <your-ip>

# Create database
az postgres flexible-server db create \
  --resource-group ticketing-rg \
  --server-name ticketing-db \
  --database-name ticketing
```

### Step 2: Set up Azure Event Hubs

```bash
# Create Event Hubs namespace
az eventhubs namespace create \
  --resource-group ticketing-rg \
  --name ticketing-events \
  --location eastus \
  --sku Standard \
  --capacity 2

# Create event hubs (topics)
az eventhubs eventhub create \
  --resource-group ticketing-rg \
  --namespace-name ticketing-events \
  --name ticketing-inventory-events \
  --partition-count 4

az eventhubs eventhub create \
  --resource-group ticketing-rg \
  --namespace-name ticketing-events \
  --name ticketing-reservation-events \
  --partition-count 4

az eventhubs eventhub create \
  --resource-group ticketing-rg \
  --namespace-name ticketing-events \
  --name ticketing-payment-events \
  --partition-count 4
```

### Step 3: Store Secrets in Key Vault

```bash
# Create Key Vault
az keyvault create \
  --resource-group ticketing-rg \
  --name ticketing-kv \
  --location eastus

# Store database connection string
az keyvault secret set \
  --vault-name ticketing-kv \
  --name database-url \
  --value "postgresql://adminuser:password@ticketing-db.postgres.database.azure.com:5432/ticketing?sslmode=require"

# Store Event Hubs connection string
az eventhubs namespace authorization-rule keys list \
  --resource-group ticketing-rg \
  --namespace-name ticketing-events \
  --name RootManageSharedAccessKey \
  --query primaryConnectionString --output tsv | \
az keyvault secret set \
  --vault-name ticketing-kv \
  --name eventhubs-connection \
  --file -
```

---

## Configuration Reference

See [.env.production.example](.env.production.example) for a complete list of configuration options.

### Required Configuration

| Variable | Description | Example |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL connection string | `postgresql://user:pass@host:5432/db` |
| `REDPANDA_BROKERS` | Comma-separated broker addresses | `broker1:9092,broker2:9092` |
| `DATABASE_SSL_MODE` | SSL mode for database | `require` |
| `REDPANDA_SECURITY_PROTOCOL` | Security protocol for event bus | `sasl_ssl` |

### Optional Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_MAX_CONNECTIONS` | 10 | Maximum connections in pool |
| `DATABASE_MIN_CONNECTIONS` | 2 | Minimum idle connections |
| `METRICS_PORT` | 9090 | Port for Prometheus metrics |
| `SHUTDOWN_TIMEOUT` | 30 | Graceful shutdown timeout (seconds) |

---

## Monitoring & Observability

### Prometheus Metrics

The application exposes Prometheus metrics on `http://localhost:9090/metrics`:

- **Event Store Metrics:**
  - `event_store_events_appended_total`
  - `event_store_events_loaded_total`
  - `event_store_append_duration_seconds`

- **Event Bus Metrics:**
  - `event_bus_messages_published_total`
  - `event_bus_messages_consumed_total`
  - `event_bus_publish_errors_total`

- **Circuit Breaker Metrics:**
  - `circuit_breaker_state` (0=closed, 1=half-open, 2=open)
  - `circuit_breaker_failures_total`
  - `circuit_breaker_rejections_total`

- **Retry Metrics:**
  - `retry_attempts_total`
  - `retry_successes_total`
  - `retry_exhausted_total`

### Grafana Dashboards

Import the provided dashboard:

```bash
# Import dashboard from grafana-dashboard.json (to be created)
```

### Alerting Rules

Example Prometheus alerting rules:

```yaml
groups:
  - name: ticketing
    rules:
      - alert: HighErrorRate
        expr: rate(event_bus_publish_errors_total[5m]) > 0.1
        for: 5m
        annotations:
          summary: "High error rate in event bus"

      - alert: CircuitBreakerOpen
        expr: circuit_breaker_state == 2
        for: 1m
        annotations:
          summary: "Circuit breaker is open"

      - alert: DatabaseConnectionPoolSaturated
        expr: database_connections_active / database_connections_max > 0.9
        for: 5m
        annotations:
          summary: "Database connection pool nearly saturated"
```

---

## Security Best Practices

### 1. Network Security

- **VPC/VNET Isolation**: Deploy all services in a private network
- **Security Groups**: Restrict access to only necessary ports
- **Firewall Rules**: Allow only application traffic

### 2. Encryption

- **At Rest**: Enable encryption for database storage and event streams
- **In Transit**: Always use SSL/TLS (set `DATABASE_SSL_MODE=require`)
- **Certificates**: Use valid SSL certificates in production

### 3. Access Control

- **Principle of Least Privilege**: Grant minimum required permissions
- **Separate Credentials**: Use different credentials for each service
- **Rotate Regularly**: Implement credential rotation policies
- **Audit Logging**: Enable audit logs for database and message broker

### 4. Secrets Management

- **Never Commit Secrets**: Use `.gitignore` for `.env.production`
- **Use Secret Managers**: Store secrets in AWS Secrets Manager, GCP Secret Manager, or Azure Key Vault
- **Environment Variables**: Inject secrets as environment variables at runtime

---

## Troubleshooting

### Database Connection Issues

**Problem**: Application cannot connect to database

**Solutions**:
1. Check network connectivity: `telnet db-host 5432`
2. Verify SSL certificate is valid
3. Check firewall/security group rules
4. Verify credentials are correct
5. Check connection pool limits

**Diagnostic Commands**:
```bash
# Test PostgreSQL connection
psql "$DATABASE_URL" -c "SELECT version();"

# Check SSL mode
echo "$DATABASE_URL" | grep -o 'sslmode=[^&]*'
```

### Event Bus Connection Issues

**Problem**: Cannot connect to RedPanda/Kafka

**Solutions**:
1. Verify broker addresses are correct
2. Check SASL/SSL credentials
3. Verify network connectivity to brokers
4. Check consumer group permissions

**Diagnostic Commands**:
```bash
# List topics
kafka-topics --bootstrap-server $REDPANDA_BROKERS --list

# Test producer
echo "test" | kafka-console-producer --bootstrap-server $REDPANDA_BROKERS --topic test-topic
```

### Performance Issues

**Problem**: Slow query performance

**Solutions**:
1. Check database connection pool saturation
2. Review slow query logs
3. Analyze query plans with `EXPLAIN ANALYZE`
4. Add database indexes
5. Increase connection pool size

**Metrics to Monitor**:
- `event_store_append_duration_seconds` (p95, p99)
- `database_connections_active`
- `circuit_breaker_failures_total`

---

## Support

For issues and questions:
- GitHub Issues: [repository-url]
- Documentation: [docs-url]
- Community: [community-url]
