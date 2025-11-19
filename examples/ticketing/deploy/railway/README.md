# Railway Deployment Guide

**Status**: Placeholder - Railway deployment configuration coming soon

Railway.app is a modern hosting platform with excellent Docker support and instant PostgreSQL provisioning.

## Why Railway?

- **Zero-config databases**: PostgreSQL with one click
- **Docker-native**: Uses our existing Dockerfile
- **Simple pricing**: $5/month per service (free tier available)
- **Git integration**: Auto-deploy on push

## Planned Configuration

```json
{
  "build": {
    "dockerfile": "Dockerfile"
  },
  "deploy": {
    "startCommand": "/app/ticketing",
    "healthcheckPath": "/health",
    "restartPolicyType": "ON_FAILURE"
  }
}
```

## Quick Start (When Available)

```bash
# Install Railway CLI
npm install -g @railway/cli

# Login
railway login

# Initialize project
railway init

# Add PostgreSQL
railway add --database postgres

# Deploy
railway up
```

## References

- Railway Docs: https://docs.railway.app/
- Railway Rust Guide: https://docs.railway.app/guides/rust

## Contributing

Want to add Railway support? See `deploy/fly/README.md` for inspiration.

Pull requests welcome!
