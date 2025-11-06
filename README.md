# 🦐 Shrmpl - Simple Server Infrastructure

> **Lightweight replacements for heavy infrastructure components**  
> One executable. Zero dependencies. Production ready.

---

## 🚀 Why Shrmpl?

Tired of spinning up Docker containers for simple services? Frustrated with complex configuration just to run Redis locally? **Shrmpl** provides tiny, single-binary replacements for the infrastructure you actually need.

**No containers. No complex setup. Just run it.**

---

## 📦 Current Components

### 🔑 shrmpl-kv
**Redis replacement** - In-memory key-value store with Redis-compatible protocol

<img src="doc/img/shrmpl-kv.png" alt="shrmpl-kv logo" width="120"/>

```bash
# Start server
./shrmpl-kv-srv

# Use client
./shrmpl-kv-cli SET mykey myvalue
./shrmpl-kv-cli GET mykey
# → myvalue
```

**Features:** GET, SET, INCR, DEL, PING • 3-5 client support • 50-char limits • TCP persistence

---

### 📝 shrmpl-log  
**ELK/Splunk replacement** - Simple TCP log aggregation and daily rotation

<img src="doc/img/shrmpl-log.png" alt="shrmpl-log logo" width="120"/>

```bash
# Start log server
./shrmpl-log-srv

# Logs automatically rotate:
# activity-20251105.log  (ACTV level)
# error-20251105.log    (ERRO level)  
# misc-20251105.log      (everything else)
```

**Features:** Fixed-width protocol • Built-in stats • Minimal dependencies • File-based storage

---

### 🔐 shrmpl-vault
**HashiCorp Vault replacement** - Secure config/secrets management with HTTPS/mTLS

<img src="doc/img/shrmpl-vault.png" alt="shrmpl-vault logo" width="120"/>

```bash
# Start vault server
./shrmpl-vault-srv

# Retrieve config securely
curl -k "https://localhost:7474/my-config?secret=dev-secret-key"
```

**Features:** HTTPS/mTLS • Rate limiting • File-based storage • Certificate management

---

## 🎯 The Shrmpl Philosophy

| Traditional Approach | Shrmpl Approach |
|-------------------|-----------------|
| 🐳 Docker containers | 🦐 Single binaries |
| 📦 Complex dependencies | ⚡ Zero runtime dependencies |
| 🔄 Heavy resource usage | 💨 Lightweight & fast |
| 📚 Extensive configuration | ✅ Simple config files |
| 🌐 Network complexity | 🏠 Local-first design |

**Perfect for:**
- ✅ Development environments
- ✅ Small production deployments  
- ✅ Edge computing
- ✅ Resource-constrained environments
- ✅ Rapid prototyping

---

## 🛠️ Quick Start

### Build from Source
```bash
# Clone and build
git clone https://github.com/yourusername/shrmpl.git
cd shrmpl
cargo build --release

# Or use our build scripts
./bin/101-build-shrmpl-kv-release
./bin/201-build-shrmpl-log-release  
./bin/301-build-shrmpl-vault-release
```

### Development Mode
```bash
# Start all services locally
./bin/105-run-shrmpl-kv-dev
./bin/205-run-shrmpl-log-dev
./bin/305-run-shrmpl-vault-dev
```

### Pre-built Binaries
Download from [Releases](https://github.com/yourusername/shrmpl/releases) for:
- macOS (Apple Silicon)
- Linux (x86_64)

---

## 📋 What's Next?

**Planned components** (coming soon):

- 📬 **shrmpl-queue** - RabbitMQ/Kafka replacement  
- 📊 **shrmpl-metrics** - InfluxDB/Prometheus replacement
- 📧 **shrmpl-mail** - Postfix/Sendmail replacement
- 📁 **shrmpl-store** - S3/MinIO replacement
- ⏰ **shrmpl-cron** - Celery/Airflow replacement
- 🌐 **shrmpl-proxy** - Nginx/HAProxy replacement

---

## 🏗️ Architecture

```
shrmpl/
├── src/                    # Rust source code
├── bin/                    # Build & run scripts  
├── etc/                    # Configuration files
├── doc/                    # Documentation & specs
├── dist/                   # Built binaries
└── tmp/                    # Runtime data (logs, etc.)
```

**Built with:**
- 🦀 **Rust** - Memory safety, performance, single binary deployment
- ⚡ **Tokio** - Async runtime for high concurrency
- 🔒 **Modern TLS** - rustls for secure communications
- 📝 **Tracing** - Structured logging integration

---

## 🤝 Contributing

We love contributions! See our [Development Guide](doc/010-dev-env-setup.md) for:
- Development environment setup
- Code style guidelines  
- Testing procedures
- Release process

**Areas needing help:**
- Windows builds
- Additional platforms (ARM Linux, etc.)
- Performance testing
- Documentation improvements

---

## 📄 License

MIT License - see [LICENSE](LICENSE) for details.

---

## 🙏 Why "Shrmpl"?

**Simple** → **SHMPL** → **SHRMPL** (pronounced "shrumple")

Like a shrimp - small, efficient, but surprisingly powerful. 🦐

---

**⭐ Star us on GitHub!**  
Tired of heavy infrastructure? Give Shrmpl a try and simplify your stack.

---

*One binary. Zero complexity. Maximum productivity.*