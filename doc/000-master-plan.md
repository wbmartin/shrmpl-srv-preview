# Master Plan
This project aims to create light weight replacements for heavy counterparts and enabling devevlopment server parity.  One approach is to create containers for these services, but with AI support, simple, shrimpy executables that are server ready is more appealing options

# Current Implementations

## shrmpl-kv // Key-Value Server
**Replaces:** Redis, Memcached  
**Purpose:** Lightweight in-memory key-value store with Redis-compatible protocol  
**Features:** GET, SET, INCR, DEL, PING commands; TCP persistence; 3-5 client support; 50-char key/value limits

## shrmpl-log  // Log Server  
**Replaces:** ELK Stack, Splunk, Logstash  
**Purpose:** Simple TCP log aggregation and daily file rotation  
**Features:** Fixed-width protocol; activity/error/misc file separation; built-in stats; minimal dependencies

## shrmpl-vault // Config & Vault Server
**Replaces:** HashiCorp Vault, AWS Secrets Manager  
**Purpose:** Secure configuration and secret management with HTTPS/mTLS  
**Features:** File-based config storage; rate limiting; secret key auth; TLS certificate management

# Potential Additions to Consider:

## shrmpl-queue // Message Queue
**Replaces:** RabbitMQ, Kafka, SQS  
**Purpose:** Async job processing and event streaming  
**Features:** Simple TCP pub/sub; disk persistence; basic routing; 3-5 client support

## shrmpl-metrics // Time Series Database  
**Replaces:** InfluxDB, Prometheus, Graphite  
**Purpose:** Metrics storage and monitoring data  
**Features:** File-based time-series; basic query API; retention policies; simple aggregation

## shrmpl-mail // SMTP Server
**Replaces:** Postfix, Sendmail, AWS SES  
**Purpose:** Email notifications and application mail  
**Features:** Basic SMTP protocol; file-based queuing; simple forwarding; local delivery

## shrmpl-store // File Storage/CDN
**Replaces:** S3, MinIO, Azure Blob Storage  
**Purpose:** Static file serving and blob storage  
**Features:** HTTP file server; basic auth; directory structure; simple metadata

## shrmpl-cron // Scheduler
**Replaces:** Celery, Airflow, Kubernetes CronJobs  
**Purpose:** Scheduled jobs and periodic tasks  
**Features:** HTTP API job management; cron expressions; simple retry logic; status tracking

## shrmpl-proxy // Load Balancer
**Replaces:** Nginx, HAProxy, AWS ALB  
**Purpose:** Request routing and SSL termination  
**Features:** HTTP proxy; basic routing rules; health checks; connection pooling
