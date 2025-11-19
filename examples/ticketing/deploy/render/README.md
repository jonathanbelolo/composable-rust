# Render Deployment Guide

**Status**: Placeholder - Render deployment configuration coming soon

Render.com is a fully-managed cloud platform with native Docker support and infrastructure-as-code via `render.yaml`.

## Why Render?

- **Infrastructure as Code**: `render.yaml` for version-controlled config
- **Managed databases**: PostgreSQL with automatic backups
- **Free tier**: Perfect for demos and prototypes
- **SSL by default**: HTTPS on all deployments

## Planned Configuration

```yaml
services:
  - type: web
    name: composable-ticketing
    env: docker
    dockerfilePath: examples/ticketing/Dockerfile
    plan: starter
    healthCheckPath: /health
    envVars:
      - key: DATABASE_URL
        fromDatabase:
          name: ticketing-events
          property: connectionString
      - key: PROJECTION_DATABASE_URL
        fromDatabase:
          name: ticketing-projections
          property: connectionString
      - key: AUTH_DATABASE_URL
        fromDatabase:
          name: ticketing-auth
          property: connectionString

databases:
  - name: ticketing-events
    plan: starter
  - name: ticketing-projections
    plan: starter
  - name: ticketing-auth
    plan: starter
```

## Quick Start (When Available)

1. Connect GitHub repository to Render
2. Add `render.yaml` to repository
3. Push to deploy automatically

Or use Render Dashboard:
1. Create Web Service
2. Point to Docker repository
3. Configure environment variables
4. Deploy

## References

- Render Docs: https://render.com/docs
- Render Rust Guide: https://render.com/docs/deploy-rust
- Infrastructure as Code: https://render.com/docs/infrastructure-as-code

## Contributing

Want to add Render support? See `deploy/fly/README.md` for inspiration.

Pull requests welcome!
