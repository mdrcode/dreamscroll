# First setup of Postgres Cloud Sql

Create instance via Google Cloud Console

By default, it opts for Public IP enabled (but completely firewalled off) and
   Private IP disabled.

To use Public IP with Cloud Auth, must use a "Auth Proxy". General
   recommendation seems to be shifting towards Private IP by default.

Following Grok's guide on how to enable Private IP:

```bash
# 1. Allocate an IP range (do this once)
gcloud compute addresses create google-managed-services-default \
  --global \
  --purpose=VPC_PEERING \
  --prefix-length=16 \
  --description="peering range for Cloud SQL" \
  --network=default

# 2. Create the private connection
gcloud services vpc-peerings connect \
  --service=servicenetworking.googleapis.com \
  --ranges=google-managed-services-default \
  --network=default

# 3. Enable Private IP on the database instance via the Console
# (just checking a box for Private IP, accept defaults otherwise)
# (this may take some time)

#4 Get your db's private IP address
gcloud sql instances describe YOUR_INSTANCE_NAME \
  --format="value(ipAddresses[0].ipAddress)"
```

Configure Cloud Run for Direct VPC Egress via Console

- Cloud Run > your service > ... Networking Tab
- Connect to a VPC for outbound traffic -> → Send traffic directly to a VPC
- Network = default
- Subnet
- Traffic routing → Route only requests to private IPs to the VPC (best for Cloud SQL).


